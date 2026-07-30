//! 单 worker 线程：后台异步 shape 文本行，结果回传主线程。
//!
//! 使用 mpsc channel 通信。generation 机制确保过期请求的结果被丢弃。

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use appkit_core::content_hash;

use crate::snap_tree::{DisplayLineEntry, VisualBreak};

/// 发送给 worker 的 reshape 请求。
#[derive(Debug)]
pub struct ReshapeRequest {
    pub generation: u64,
    pub doc_line: usize,
    pub byte_offset: usize,
    pub byte_length: u32,
    /// 原始行字节（Arc 共享避免拷贝大行）
    pub line_bytes: Arc<[u8]>,
    pub viewport_width: f32,
    pub font_size: f32,
    /// 0 = 不截断，>0 = 最多 shape 这么多字节
    pub max_line_bytes: usize,
    /// Target DocumentView index (for routing results back).
    pub dv_idx: usize,
}

/// Worker 返回的 reshape 结果。
#[derive(Debug)]
pub struct ReshapeResult {
    pub generation: u64,
    pub doc_line: usize,
    pub entry: DisplayLineEntry,
    pub dv_idx: usize,
}

pub enum WorkerCommand {
    Shape(ReshapeRequest),
    Shutdown,
}

use std::sync::atomic::{AtomicU64, Ordering};

/// 后台 reshape worker。
pub struct ReshapeWorker {
    sender: Sender<WorkerCommand>,
    receiver: Receiver<ReshapeResult>,
    latest_generation: Arc<AtomicU64>,
    _handle: JoinHandle<()>,
}

impl ReshapeWorker {
    /// 启动 worker 线程，使用共享的 FontSystem 创建 Shaper。
    /// 如果 shaper 初始化失败，回退到字符宽度估算。
    pub fn spawn(
        font_system: Arc<std::sync::Mutex<shaping::FontSystem>>,
        font_size: f32,
        font_family: String,
    ) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<WorkerCommand>();
        let (result_tx, result_rx) = mpsc::channel::<ReshapeResult>();
        let latest_generation = Arc::new(AtomicU64::new(0));
        let worker_generation = Arc::clone(&latest_generation);

        let handle = thread::Builder::new()
            .name("reshape-worker".into())
            .spawn(move || {
                // Create a shaper from the shared FontSystem (one-time cost).
                let mut shaper: Option<shaping::Shaper> = Some(
                    shaping::Shaper::from_shared_font_system(font_system, font_size, &font_family),
                );

                for cmd in cmd_rx {
                    match cmd {
                        WorkerCommand::Shape(req) => {
                            if req.generation < worker_generation.load(Ordering::Relaxed) {
                                continue;
                            }
                            let entry = match &mut shaper {
                                Some(s) => {
                                    s.set_font_size(req.font_size);
                                    process_with_shaper(s, &req)
                                }
                                None => process_fallback(&req),
                            };
                            let _ = result_tx.send(ReshapeResult {
                                generation: req.generation,
                                doc_line: req.doc_line,
                                entry,
                                dv_idx: req.dv_idx,
                            });
                        }
                        WorkerCommand::Shutdown => break,
                    }
                }
            })
            .expect("failed to spawn reshape worker");

        Self { sender: cmd_tx, receiver: result_rx, latest_generation, _handle: handle }
    }

    /// 提交 reshape 请求（非阻塞）。
    pub fn submit(&self, request: ReshapeRequest) -> Result<(), mpsc::SendError<WorkerCommand>> {
        self.sender.send(WorkerCommand::Shape(request))
    }

    /// 排空所有已完成的结果。
    pub fn drain_completed(&self, max: usize) -> Vec<ReshapeResult> {
        let mut results = Vec::new();
        for _ in 0..max {
            match self.receiver.try_recv() {
                Ok(r) => results.push(r),
                Err(_) => break,
            }
        }
        results
    }

    /// 取消指定 generation 之前的所有进行中请求（跳过已排队的过时请求）。
    pub fn cancel_before(&self, generation: u64) {
        // Fetch max to avoid lowering generation by mistake
        let mut current = self.latest_generation.load(Ordering::Relaxed);
        while generation > current {
            if self
                .latest_generation
                .compare_exchange_weak(current, generation, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                break;
            }
            current = self.latest_generation.load(Ordering::Relaxed);
        }
    }

    /// 关闭 worker 线程。
    pub fn shutdown(self) {
        let _ = self.sender.send(WorkerCommand::Shutdown);
    }
}

/// 使用真实 Shaper 的 shape_fast 计算换行结果。
fn process_with_shaper(shaper: &mut shaping::Shaper, req: &ReshapeRequest) -> DisplayLineEntry {
    let bytes = &req.line_bytes;
    let max_bytes =
        if req.max_line_bytes > 0 { req.max_line_bytes.min(bytes.len()) } else { bytes.len() };

    let line_str = match std::str::from_utf8(&bytes[..max_bytes]) {
        Ok(s) => s,
        Err(e) => std::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap_or(""),
    };

    if line_str.is_empty() {
        let content_hash = content_hash::content_hash(
            req.byte_offset,
            req.byte_length,
            req.viewport_width,
            req.font_size,
        );
        return DisplayLineEntry::placeholder(0, bytes.len() as u32, content_hash, 1);
    }

    let shaped = match shaper.shape_fast(line_str) {
        Ok(s) => s,
        Err(_) => match shaper.shape(line_str) {
            Ok(s) => s,
            Err(_) => return process_fallback(req),
        },
    };

    let viewport_width = req.viewport_width.max(1.0);
    let char_width = shaper.col_width();
    let visual_lines =
        ui::layout::compute_visual_lines(&shaped.clusters, bytes, char_width, viewport_width, 0.5);

    let mut breaks: smallvec::SmallVec<[VisualBreak; 1]> = smallvec::SmallVec::new();
    for (start, end, pixel_width) in visual_lines {
        let byte_start = shaped.clusters[start].byte_range.start as u32;
        let byte_end = shaped.clusters[end - 1].byte_range.end as u32;
        breaks.push(VisualBreak { byte_start, byte_end, pixel_width });
    }

    let visual_line_count = breaks.len().max(1) as u16;
    let content_hash = content_hash::content_hash(
        req.byte_offset,
        req.byte_length,
        req.viewport_width,
        req.font_size,
    );

    DisplayLineEntry {
        visual_line_count,
        visual_breaks: breaks,
        byte_offset: req.byte_offset,
        byte_length: req.byte_length,
        content_hash,
    }
}

/// 回退：不使用 shaper，仅凭字符宽度估算。
fn process_fallback(req: &ReshapeRequest) -> DisplayLineEntry {
    let bytes = &req.line_bytes;
    let max_bytes =
        if req.max_line_bytes > 0 { req.max_line_bytes.min(bytes.len()) } else { bytes.len() };

    let ascii_w = req.font_size * 0.6;
    let cjk_w = req.font_size;
    if ascii_w <= 0.0 {
        let content_hash = content_hash::content_hash(
            req.byte_offset,
            req.byte_length,
            req.viewport_width,
            req.font_size,
        );
        return DisplayLineEntry::placeholder(0, bytes.len() as u32, content_hash, 1);
    }

    let line_str = match std::str::from_utf8(&bytes[..max_bytes]) {
        Ok(s) => s,
        Err(e) => std::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap_or(""),
    };
    if line_str.is_empty() {
        let content_hash = content_hash::content_hash(
            req.byte_offset,
            req.byte_length,
            req.viewport_width,
            req.font_size,
        );
        return DisplayLineEntry::placeholder(0, bytes.len() as u32, content_hash, 1);
    }

    // Per-character width: CJK/wide chars use font_size (1em), ASCII uses font_size*0.6
    let char_w = |ch: char| -> f32 {
        if ch == '\t' {
            return ascii_w * ui::layout::DEFAULT_TAB_WIDTH as f32;
        }
        if ui::layout::is_cjk_char(ch) { cjk_w } else { ascii_w }
    };

    // Build char info array with pre-computed widths for O(1) range queries
    struct CharInfo {
        byte_start: usize,
        #[allow(dead_code)]
        byte_end: usize,
        width: f32,
        is_ws: bool,
        is_alnum: bool,
        is_punct: bool,
        is_newline: bool,
    }

    let mut char_infos: Vec<CharInfo> = Vec::new();
    for (ci, ch) in line_str.char_indices() {
        char_infos.push(CharInfo {
            byte_start: ci,
            byte_end: ci + ch.len_utf8(),
            width: char_w(ch),
            is_ws: ch.is_ascii_whitespace(),
            is_alnum: ch.is_ascii_alphanumeric(),
            is_punct: ch.is_ascii_punctuation(),
            is_newline: ch == '\n',
        });
    }

    if char_infos.is_empty() {
        let content_hash = content_hash::content_hash(
            req.byte_offset,
            req.byte_length,
            req.viewport_width,
            req.font_size,
        );
        return DisplayLineEntry::placeholder(0, bytes.len() as u32, content_hash, 1);
    }

    let n = char_infos.len();
    let vp = req.viewport_width.max(1.0);

    // Prefix sums for O(1) range width queries
    let mut prefix: Vec<f32> = Vec::with_capacity(n + 1);
    prefix.push(0.0);
    for ci in &char_infos {
        prefix.push(prefix.last().unwrap() + ci.width);
    }
    let width_of = |s: usize, e: usize| prefix[e] - prefix[s];

    // Width after stripping trailing whitespace
    let trimmed_width = |s: usize, e: usize| -> f32 {
        let mut w = width_of(s, e);
        let mut i = e;
        while i > s && char_infos[i - 1].is_ws {
            w -= char_infos[i - 1].width;
            i -= 1;
        }
        w
    };

    let mut breaks: smallvec::SmallVec<[VisualBreak; 1]> = smallvec::SmallVec::new();
    let mut start = 0usize;
    let mut last_ws: Option<usize> = None; // index of first non-ws char after space

    let mut ci = 0usize;
    while ci < n {
        let ch = &char_infos[ci];

        // Track word boundary (after whitespace → before non-whitespace)
        if !ch.is_ws && ci > 0 && char_infos[ci - 1].is_ws {
            last_ws = Some(ci);
        }

        // Explicit newline: force break and skip
        if ch.is_newline {
            breaks.push(VisualBreak {
                byte_start: char_infos[start].byte_start as u32,
                byte_end: ch.byte_start as u32,
                pixel_width: width_of(start, ci).min(vp),
            });
            start = ci + 1; // skip newline char
            last_ws = None;
            ci += 1;
            continue;
        }

        let line_x = width_of(start, ci);
        if line_x + ch.width > vp && ci > start {
            let hard_x = line_x;
            let mut break_at = ci;

            // Rule 1: if hard break falls inside ASCII alnum run, backtrack to run start
            if ch.is_alnum && ci > start {
                let mut run_start = ci;
                while run_start > start && char_infos[run_start - 1].is_alnum {
                    run_start -= 1;
                }
                if run_start > start {
                    break_at = run_start;
                }
            }

            // Rule 2: prefer word boundary (space), but only if line is reasonably filled
            if let Some(ws) = last_ws
                && ws > start
                && ws <= ci
            {
                // Don't break right before punctuation
                let next_is_punct = char_infos[ws].is_punct;
                if !next_is_punct {
                    let ws_x = trimmed_width(start, ws);
                    if ws_x >= hard_x * 0.5 {
                        break_at = ws;
                    }
                }
            }

            // Rule 3: if chosen break leaves punctuation at line start, fall back to hard break
            if break_at < n && char_infos[break_at].is_punct {
                break_at = ci;
            }

            let break_byte_end = char_infos[break_at].byte_start;
            let break_x = if break_at == ci { line_x } else { trimmed_width(start, break_at) };

            breaks.push(VisualBreak {
                byte_start: char_infos[start].byte_start as u32,
                byte_end: break_byte_end as u32,
                pixel_width: break_x.min(vp),
            });

            start = break_at;
            // Skip leading whitespace on continuation line
            while start < n && char_infos[start].is_ws {
                start += 1;
            }
            last_ws = None;
            if start > ci {
                ci = start;
            }
            continue;
        }
        ci += 1;
    }

    // Final visual line
    if start < n {
        breaks.push(VisualBreak {
            byte_start: char_infos[start].byte_start as u32,
            byte_end: max_bytes as u32,
            pixel_width: width_of(start, n).min(vp),
        });
    }

    let visual_line_count = breaks.len().max(1) as u16;
    let content_hash = content_hash::content_hash(
        req.byte_offset,
        req.byte_length,
        req.viewport_width,
        req.font_size,
    );

    DisplayLineEntry {
        visual_line_count,
        visual_breaks: breaks,
        byte_offset: req.byte_offset,
        byte_length: req.byte_length,
        content_hash,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_empty_line() {
        let req = ReshapeRequest {
            generation: 1,
            doc_line: 0,
            line_bytes: Arc::from(vec![].into_boxed_slice()),
            viewport_width: 800.0,
            font_size: 14.0,
            max_line_bytes: 0,
            dv_idx: 0,
            byte_offset: 0,
            byte_length: 0,
        };
        let entry = process_fallback(&req);
        assert_eq!(entry.visual_line_count, 1);
    }

    #[test]
    fn process_single_short_line() {
        let req = ReshapeRequest {
            generation: 1,
            doc_line: 0,
            line_bytes: Arc::from(b"hello".to_vec().into_boxed_slice()),
            viewport_width: 800.0,
            font_size: 14.0,
            max_line_bytes: 0,
            dv_idx: 0,
            byte_offset: 0,
            byte_length: 5,
        };
        let entry = process_fallback(&req);
        assert_eq!(entry.visual_line_count, 1);
    }

    fn recv_one(worker: &ReshapeWorker, timeout: std::time::Duration) -> ReshapeResult {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Some(result) = worker.drain_completed(1).into_iter().next() {
                return result;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "condition was not met within {timeout:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn worker_spawn_and_result() {
        let fs = shaping::FontSystem::new();
        let fs = Arc::new(std::sync::Mutex::new(fs));
        let w = ReshapeWorker::spawn(Arc::clone(&fs), 14.0, "Menlo".into());
        let _ = w.submit(ReshapeRequest {
            generation: 1,
            doc_line: 0,
            line_bytes: Arc::from(b"test".to_vec().into_boxed_slice()),
            viewport_width: 200.0,
            font_size: 14.0,
            max_line_bytes: 0,
            dv_idx: 0,
            byte_offset: 0,
            byte_length: 4,
        });
        let r = recv_one(&w, std::time::Duration::from_secs(2));
        assert_eq!(r.entry.byte_length, 4);
        w.shutdown();
    }

    #[test]
    fn generation_filtering() {
        let fs = shaping::FontSystem::new();
        let fs = Arc::new(std::sync::Mutex::new(fs));
        let w = ReshapeWorker::spawn(Arc::clone(&fs), 14.0, "Menlo".into());
        let _ = w.submit(ReshapeRequest {
            generation: 99,
            doc_line: 0,
            line_bytes: Arc::from(b"old".to_vec().into_boxed_slice()),
            viewport_width: 800.0,
            font_size: 14.0,
            max_line_bytes: 0,
            dv_idx: 0,
            byte_offset: 0,
            byte_length: 3,
        });
        let r = recv_one(&w, std::time::Duration::from_secs(2));
        assert_eq!(r.generation, 99);
        w.shutdown();
    }

    #[test]
    fn truncates_long_line_when_threshold_set() {
        let big = vec![b'a'; 200_000];
        let fs = shaping::FontSystem::new();
        let fs = Arc::new(std::sync::Mutex::new(fs));
        let w = ReshapeWorker::spawn(Arc::clone(&fs), 14.0, "Menlo".into());
        let _ = w.submit(ReshapeRequest {
            generation: 1,
            doc_line: 0,
            line_bytes: Arc::from(big.into_boxed_slice()),
            viewport_width: 800.0,
            font_size: 14.0,
            max_line_bytes: 1000,
            dv_idx: 0,
            byte_offset: 0,
            byte_length: 200_000,
        });
        let r = recv_one(&w, std::time::Duration::from_secs(2));
        assert_eq!(r.entry.byte_length, 200_000);
        w.shutdown();
    }

    #[test]
    fn max_line_bytes_zero_no_truncation() {
        let data = b"short line";
        let fs = shaping::FontSystem::new();
        let fs = Arc::new(std::sync::Mutex::new(fs));
        let w = ReshapeWorker::spawn(Arc::clone(&fs), 14.0, "Menlo".into());
        let _ = w.submit(ReshapeRequest {
            generation: 1,
            doc_line: 0,
            line_bytes: Arc::from(data.to_vec().into_boxed_slice()),
            viewport_width: 800.0,
            font_size: 14.0,
            max_line_bytes: 0,
            dv_idx: 0,
            byte_offset: 0,
            byte_length: data.len() as u32,
        });
        let r = recv_one(&w, std::time::Duration::from_secs(2));
        assert_eq!(r.entry.byte_length, data.len() as u32);
        w.shutdown();
    }

    #[test]
    fn fallback_long_number_backtrack() {
        // Long ASCII number without spaces: should backtrack to before the number.
        let data = b"ID: 123456789012345678901234567890";
        let req = ReshapeRequest {
            generation: 1,
            doc_line: 0,
            line_bytes: Arc::from(data.to_vec().into_boxed_slice()),
            viewport_width: 120.0,
            font_size: 14.0,
            max_line_bytes: 0,
            dv_idx: 0,
            byte_offset: 0,
            byte_length: data.len() as u32,
        };
        let entry = process_fallback(&req);
        // Should break into multiple lines — number should not be split mid-digit
        assert!(
            entry.visual_line_count >= 2,
            "expected >=2 visual lines for long number, got {}",
            entry.visual_line_count
        );
        // First break should be before the number (not inside it)
        if entry.visual_breaks.len() > 1 {
            let first_break = &entry.visual_breaks[0];
            // The first break should end around "ID: " — before the number starts
            // "ID: " = 4 bytes. Break might include space or not.
            let end = first_break.byte_end as usize;
            // Assert break is before the number's first significant digit
            // The number starts at byte 4 ("ID: " is bytes 0-3)
            assert!(end <= 5, "first break should end before the number, got byte_end={}", end);
        }
    }

    #[test]
    fn fallback_long_word_no_space() {
        // Long ASCII word with NO spaces at all: must hard-break inside.
        let big: Vec<u8> = (0..200).map(|_| b'x').collect();
        let req = ReshapeRequest {
            generation: 1,
            doc_line: 0,
            line_bytes: Arc::from(big.into_boxed_slice()),
            viewport_width: 800.0,
            font_size: 14.0,
            max_line_bytes: 0,
            dv_idx: 0,
            byte_offset: 0,
            byte_length: 200,
        };
        let entry = process_fallback(&req);
        // With viewport 800px and font 14px, char_w ≈ 8.4px, 200 chars ≈ 1680px
        // Should wrap into multiple lines
        assert!(
            entry.visual_line_count >= 2,
            "expected >=2 visual lines for 200-char word, got {}",
            entry.visual_line_count
        );
    }

    #[test]
    fn fallback_punct_not_at_start() {
        // Comma after space should not start a wrapped line.
        let data = b"hello, world, foo bar baz qux extra padding here";
        let req = ReshapeRequest {
            generation: 1,
            doc_line: 0,
            line_bytes: Arc::from(data.to_vec().into_boxed_slice()),
            viewport_width: 120.0,
            font_size: 14.0,
            max_line_bytes: 0,
            dv_idx: 0,
            byte_offset: 0,
            byte_length: data.len() as u32,
        };
        let entry = process_fallback(&req);
        // Check that no visual break starts right after a punctuation byte
        for b in &entry.visual_breaks {
            let start = b.byte_start as usize;
            if start < data.len() {
                let byte = data[start];
                assert!(
                    !byte.is_ascii_punctuation(),
                    "visual break starts at punctuation byte {byte} at offset {start}"
                );
            }
        }
    }
}
