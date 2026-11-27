//! Opt-in editor input latency measurements through surface submission.

use std::fs::OpenOptions;
use std::io::{self, BufWriter, Write};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use appkit_core::tab_set::types::TabId;

use super::SurfaceFrameSubmission;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputEventKind {
    Key,
    ImePreedit,
    ImeCommit,
}

const INPUT_LOG_FRAME_CAPACITY: usize = 128;
const INPUTS_PER_FRAME_LIMIT: usize = 512;
const INPUT_LOG_FLUSH_INTERVAL: Duration = Duration::from_millis(250);

struct InputSample {
    kind: InputEventKind,
    entered_at: Instant,
    handled_at: Instant,
    superseded: bool,
}

struct InputFrame {
    sequence: u64,
    inputs: Vec<InputSample>,
    redraw_started_at: Instant,
    submission: SurfaceFrameSubmission,
    frame_tail: Duration,
    accessibility: Duration,
    ime_position: Duration,
    skipped_redraws: usize,
    discarded_inputs: usize,
}

pub struct InputLatencyLog {
    sender: SyncSender<InputFrame>,
    tab: Option<TabId>,
    pending: Vec<InputSample>,
    next_frame: u64,
    skipped_redraws: usize,
    discarded_inputs: usize,
}

impl InputLatencyLog {
    pub fn has_pending_samples(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn cancel_pending_preedit(&mut self) {
        for sample in &mut self.pending {
            if sample.kind == InputEventKind::ImePreedit {
                sample.superseded = true;
            }
        }
    }

    pub fn open() -> Option<Self> {
        let path = std::env::var_os("TEXTORA_INPUT_LOG").filter(|path| !path.is_empty())?;
        let result = (|| -> io::Result<Self> {
            let file = OpenOptions::new().create(true).append(true).open(path)?;
            let (sender, receiver) = mpsc::sync_channel::<InputFrame>(INPUT_LOG_FRAME_CAPACITY);
            let time_anchor = (Instant::now(), SystemTime::now());
            std::thread::Builder::new().name("input-latency-log".into()).spawn(move || {
                let mut writer = BufWriter::new(file);
                let result = loop {
                    let result = match receiver.recv_timeout(INPUT_LOG_FLUSH_INTERVAL) {
                        Ok(frame) => frame.write(&mut writer, time_anchor),
                        Err(RecvTimeoutError::Timeout) => writer.flush(),
                        Err(RecvTimeoutError::Disconnected) => break writer.flush(),
                    };
                    if result.is_err() {
                        break result;
                    }
                };
                if let Err(error) = result {
                    eprintln!("[input-latency] write failed: {error}");
                }
            })?;
            Ok(Self {
                sender,
                tab: None,
                pending: Vec::new(),
                next_frame: 0,
                skipped_redraws: 0,
                discarded_inputs: 0,
            })
        })();
        result.inspect_err(|error| eprintln!("[input-latency] setup failed: {error}")).ok()
    }

    pub fn retain_tab(&mut self, active_tab: Option<TabId>) {
        if self.tab != active_tab {
            self.discarded_inputs += self.pending.len();
            self.pending.clear();
            self.skipped_redraws = 0;
            self.tab = active_tab;
        }
    }

    pub fn record(
        &mut self,
        tab: TabId,
        kind: InputEventKind,
        entered_at: Instant,
        handled_at: Instant,
    ) {
        self.retain_tab(Some(tab));
        if self.pending.len() == INPUTS_PER_FRAME_LIMIT {
            self.discarded_inputs += 1;
            return;
        }
        self.pending.push(InputSample { kind, entered_at, handled_at, superseded: false });
    }

    pub fn finish_redraw(
        &mut self,
        redraw_started_at: Instant,
        submission: Option<SurfaceFrameSubmission>,
        accessibility: Duration,
        ime_position: Duration,
        finished_at: Instant,
    ) {
        if self.pending.is_empty() {
            return;
        }
        let Some(submission) = submission else {
            self.skipped_redraws += 1;
            return;
        };
        let frame = InputFrame {
            sequence: self.next_frame,
            inputs: std::mem::take(&mut self.pending),
            redraw_started_at,
            frame_tail: finished_at - submission.present_returned_at,
            submission,
            accessibility,
            ime_position,
            skipped_redraws: std::mem::take(&mut self.skipped_redraws),
            discarded_inputs: self.discarded_inputs,
        };
        self.next_frame += 1;
        let input_count = frame.inputs.len();
        self.discarded_inputs = match self.sender.try_send(frame) {
            Ok(()) => 0,
            Err(_) => self.discarded_inputs + input_count,
        };
    }
}

impl InputFrame {
    fn write(
        &self,
        writer: &mut impl Write,
        (monotonic_origin, wall_origin): (Instant, SystemTime),
    ) -> io::Result<()> {
        let Some(preparation) = self.submission.preparation.as_ref() else { return Ok(()) };
        let present_wall_us = (wall_origin
            + (self.submission.present_returned_at - monotonic_origin))
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_micros());
        let last_preedit_or_commit = self.inputs.iter().rposition(|sample| {
            matches!(sample.kind, InputEventKind::ImePreedit | InputEventKind::ImeCommit)
        });
        for (index, input) in self.inputs.iter().enumerate() {
            let superseded = input.superseded
                || (input.kind == InputEventKind::ImePreedit
                    && last_preedit_or_commit != Some(index));
            writeln!(
                writer,
                "frame={} index={} kind={:?} superseded={} coalesced={} e2e_us={} handle_us={} queue_us={} render_cpu_us={} acquire_us={} encode_us={} submit_us={} frame_tail_us={} a11y_us={} ime_position_us={} skipped_redraws={} discarded_inputs={} present_wall_us={present_wall_us}",
                self.sequence,
                index,
                input.kind,
                u8::from(superseded),
                self.inputs.len(),
                (self.submission.present_returned_at - input.entered_at).as_micros(),
                (input.handled_at - input.entered_at).as_micros(),
                (self.redraw_started_at - input.handled_at).as_micros(),
                (preparation.acquire_started_at - self.redraw_started_at).as_micros(),
                preparation.acquire.as_micros(),
                preparation.encode.as_micros(),
                self.submission.submit_and_present.as_micros(),
                self.frame_tail.as_micros(),
                self.accessibility.as_micros(),
                self.ime_position.as_micros(),
                self.skipped_redraws,
                self.discarded_inputs,
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod input_latency_tests {
    use super::*;
    use crate::editor_runtime::SurfaceFramePreparation;
    use appkit_core::tab_set::types::TabIdAllocator;

    #[test]
    fn input_log_waits_for_submission_and_keeps_each_events_timestamp() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut log = InputLatencyLog {
            sender,
            tab: None,
            pending: Vec::new(),
            next_frame: 0,
            skipped_redraws: 0,
            discarded_inputs: 0,
        };
        let tab = TabIdAllocator::new().allocate();
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        log.record(tab, InputEventKind::ImePreedit, at(0), at(1));
        log.cancel_pending_preedit();
        assert!(log.pending[0].superseded);
        log.pending.clear();
        log.record(tab, InputEventKind::ImePreedit, at(0), at(1));
        log.finish_redraw(at(2), None, Duration::ZERO, Duration::ZERO, at(3));
        assert!(receiver.try_recv().is_err());

        log.record(tab, InputEventKind::ImeCommit, at(7), at(8));
        let submission = SurfaceFrameSubmission {
            present_returned_at: at(15),
            submit_and_present: Duration::from_millis(1),
            preparation: Some(SurfaceFramePreparation {
                acquire_started_at: at(10),
                acquire: Duration::from_millis(2),
                encode: Duration::from_millis(2),
            }),
        };
        log.finish_redraw(at(9), Some(submission), Duration::ZERO, Duration::ZERO, at(16));
        let frame = receiver.try_recv().expect("successful submission must settle pending inputs");
        let mut output = Vec::new();
        frame.write(&mut output, (start, UNIX_EPOCH)).expect("test log should write to memory");
        let output = String::from_utf8(output).expect("input log contains only ASCII");
        assert!(output.contains("kind=ImePreedit superseded=1 coalesced=2 e2e_us=15000"));
        assert!(output.contains("kind=ImeCommit superseded=0 coalesced=2 e2e_us=8000"));
        assert!(output.lines().all(|line| line.ends_with("present_wall_us=15000")));
        assert_eq!(frame.skipped_redraws, 1);
        assert!(log.pending.is_empty());

        log.record(tab, InputEventKind::Key, at(17), at(18));
        log.retain_tab(None);
        assert!(log.pending.is_empty());
        assert_eq!(log.discarded_inputs, 1);
    }
}
