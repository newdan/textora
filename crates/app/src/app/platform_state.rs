use std::fs::OpenOptions;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use appkit_core::tab_set::types::TabId;
use appkit_shell::accessibility_adapter::PlatformAccessibilityAdapter;
use appkit_shell::editor_runtime::SurfaceFrameSubmission;
use winit::event_loop::EventLoopProxy;

use crate::app_event::AppEvent;
use crate::app_lifecycle::InputEventKind;
use crate::mouse::MouseState;

pub(crate) struct PlatformState {
    startup_file: Option<PathBuf>,
    startup_started_at: Instant,
    font_preparation: Option<JoinHandle<shaping::FontSystem>>,
    event_loop_proxy: Option<EventLoopProxy<AppEvent>>,
    accessibility: Option<PlatformAccessibilityAdapter>,
    redraw_pending: bool,
    last_cursor_visible: bool,
    mouse: MouseState,
    input_latency: Option<InputLatencyLog>,
}

impl PlatformState {
    pub(crate) fn new(
        startup_file: Option<PathBuf>,
        startup_started_at: Instant,
        font_preparation: Option<JoinHandle<shaping::FontSystem>>,
    ) -> Self {
        Self {
            startup_file,
            startup_started_at,
            font_preparation,
            event_loop_proxy: None,
            accessibility: None,
            redraw_pending: true,
            last_cursor_visible: true,
            mouse: MouseState::new(),
            input_latency: InputLatencyLog::open(),
        }
    }

    pub(crate) fn startup_file(&self) -> Option<&Path> {
        self.startup_file.as_deref()
    }

    pub(crate) fn clear_startup_file(&mut self) {
        self.startup_file = None;
    }

    pub(crate) fn startup_elapsed(&self) -> Duration {
        self.startup_started_at.elapsed()
    }

    pub(crate) fn take_font_preparation(&mut self) -> Option<JoinHandle<shaping::FontSystem>> {
        self.font_preparation.take()
    }

    #[cfg(test)]
    pub(crate) fn has_font_preparation(&self) -> bool {
        self.font_preparation.is_some()
    }

    pub(crate) fn register_event_loop_proxy(&mut self, event_loop_proxy: EventLoopProxy<AppEvent>) {
        self.event_loop_proxy = Some(event_loop_proxy);
    }

    pub(crate) fn event_loop_proxy(&self) -> Option<&EventLoopProxy<AppEvent>> {
        self.event_loop_proxy.as_ref()
    }

    pub(crate) fn install_accessibility(&mut self, adapter: PlatformAccessibilityAdapter) {
        self.accessibility = Some(adapter);
    }

    pub(crate) fn accessibility(&self) -> Option<&PlatformAccessibilityAdapter> {
        self.accessibility.as_ref()
    }

    pub(crate) fn accessibility_mut(&mut self) -> Option<&mut PlatformAccessibilityAdapter> {
        self.accessibility.as_mut()
    }

    pub(crate) fn schedule_redraw(&mut self) {
        self.redraw_pending = true;
    }

    pub(crate) fn clear_redraw(&mut self) {
        self.redraw_pending = false;
    }

    pub(crate) fn redraw_pending(&self) -> bool {
        self.redraw_pending
    }

    pub(crate) fn update_cursor_visibility(&mut self, visible: bool) {
        if visible != self.last_cursor_visible {
            self.last_cursor_visible = visible;
            self.schedule_redraw();
        }
    }

    pub(crate) fn mouse(&self) -> &MouseState {
        &self.mouse
    }

    pub(crate) fn mouse_mut(&mut self) -> &mut MouseState {
        &mut self.mouse
    }

    pub(crate) fn input_latency(&mut self) -> Option<&mut InputLatencyLog> {
        self.input_latency.as_mut()
    }

    pub(crate) fn input_latency_enabled(&self) -> bool {
        self.input_latency.is_some()
    }

    pub(crate) fn has_pending_input_sample(&self) -> bool {
        self.input_latency.as_ref().is_some_and(|log| !log.pending.is_empty())
    }
}

const INPUT_LOG_FRAME_CAPACITY: usize = 128;
const INPUTS_PER_FRAME_LIMIT: usize = 512;
const INPUT_LOG_FLUSH_INTERVAL: Duration = Duration::from_millis(250);

struct InputSample {
    kind: InputEventKind,
    entered_at: Instant,
    handled_at: Instant,
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

pub(crate) struct InputLatencyLog {
    sender: SyncSender<InputFrame>,
    tab: Option<TabId>,
    pending: Vec<InputSample>,
    next_frame: u64,
    skipped_redraws: usize,
    discarded_inputs: usize,
}

impl InputLatencyLog {
    fn open() -> Option<Self> {
        let path = std::env::var_os("TEXTORA_INPUT_LOG").filter(|path| !path.is_empty())?;
        let result = (|| -> io::Result<Self> {
            let file = OpenOptions::new().create(true).append(true).open(path)?;
            let (sender, receiver) = mpsc::sync_channel::<InputFrame>(INPUT_LOG_FRAME_CAPACITY);
            std::thread::Builder::new().name("input-latency-log".into()).spawn(move || {
                let mut writer = BufWriter::new(file);
                let result = loop {
                    let result = match receiver.recv_timeout(INPUT_LOG_FLUSH_INTERVAL) {
                        Ok(frame) => frame.write(&mut writer),
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

    pub(crate) fn retain_tab(&mut self, active_tab: Option<TabId>) {
        if self.tab != active_tab {
            self.discarded_inputs += self.pending.len();
            self.pending.clear();
            self.skipped_redraws = 0;
            self.tab = active_tab;
        }
    }

    pub(crate) fn record(
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
        self.pending.push(InputSample { kind, entered_at, handled_at });
    }

    pub(crate) fn finish_redraw(
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
    fn write(&self, writer: &mut impl Write) -> io::Result<()> {
        let Some(preparation) = self.submission.preparation.as_ref() else { return Ok(()) };
        let last_preedit_or_commit = self.inputs.iter().rposition(|sample| {
            matches!(sample.kind, InputEventKind::ImePreedit | InputEventKind::ImeCommit)
        });
        for (index, input) in self.inputs.iter().enumerate() {
            let superseded =
                input.kind == InputEventKind::ImePreedit && last_preedit_or_commit != Some(index);
            writeln!(
                writer,
                "frame={} index={} kind={:?} superseded={} coalesced={} e2e_us={} handle_us={} queue_us={} render_cpu_us={} acquire_us={} encode_us={} submit_us={} frame_tail_us={} a11y_us={} ime_position_us={} skipped_redraws={} discarded_inputs={}",
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
mod tests {
    use super::*;
    use appkit_core::tab_set::types::TabIdAllocator;
    use appkit_shell::editor_runtime::SurfaceFramePreparation;

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
        frame.write(&mut output).expect("test log should write to memory");
        let output = String::from_utf8(output).expect("input log contains only ASCII");
        assert!(output.contains("kind=ImePreedit superseded=1 coalesced=2 e2e_us=15000"));
        assert!(output.contains("kind=ImeCommit superseded=0 coalesced=2 e2e_us=8000"));
        assert_eq!(frame.skipped_redraws, 1);
        assert!(log.pending.is_empty());

        log.record(tab, InputEventKind::Key, at(17), at(18));
        log.retain_tab(None);
        assert!(log.pending.is_empty());
        assert_eq!(log.discarded_inputs, 1);
    }
}
