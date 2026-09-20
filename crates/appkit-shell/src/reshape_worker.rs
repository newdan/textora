//! 单 worker 线程：后台异步 shape 文本行，结果回传主线程。
//!
//! 使用 mpsc channel 通信。generation 机制确保过期请求的结果被丢弃。

use std::ops::Range;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use appkit_core::content_hash;
use core::unicode::{
    ucd_grapheme_cluster_joins, ucd_grapheme_cluster_joins_done, ucd_grapheme_cluster_lookup,
};
use ui::typography::{TextSpacingMode, TypographyCluster, TypographyInput};

use crate::snap_tree::{DisplayLineEntry, VisualBreak};

const LAYOUT_HASH_FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const NATURAL_MODE_HASH_TAG: u64 = 0x4e41_5455_5241_4c00;
const VERBATIM_MODE_HASH_TAG: u64 = 0x5645_5242_4154_494d;
const FALLBACK_MIN_FILL_RATIO: f32 = 0.0;

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
    pub font_family: Arc<str>,
    /// 0 = 不截断，>0 = 最多 shape 这么多字节
    pub max_line_bytes: usize,
    /// Source spacing policy shared by foreground and background shaping.
    pub spacing_mode: TextSpacingMode,
    /// Line-local source ranges that must retain verbatim geometry.
    pub protected_ranges: Arc<[Range<usize>]>,
    /// Semantic rules version used alongside the protected ranges in cache keys.
    pub semantic_version: u64,
    /// Target DocumentView index (for routing results back).
    pub dv_idx: usize,
}

impl ReshapeRequest {
    pub fn content_hash(&self) -> u64 {
        content_hash_for_layout(
            self.line_bytes.as_ref(),
            self.byte_offset,
            self.viewport_width,
            self.font_size,
            self.spacing_mode,
            &self.protected_ranges,
            self.semantic_version,
            &self.font_family,
        )
    }
}

/// Compute the cache key shared by foreground rendering and worker reshaping.
pub fn content_hash_for_layout(
    line_bytes: &[u8],
    byte_offset: usize,
    viewport_width: f32,
    font_size: f32,
    spacing_mode: TextSpacingMode,
    protected_ranges: &[Range<usize>],
    semantic_version: u64,
    font_family: &str,
) -> u64 {
    let mut hash = content_hash::content_hash(line_bytes, byte_offset, viewport_width, font_size);
    for byte in font_family.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(LAYOUT_HASH_FNV_PRIME);
    }
    hash ^= match spacing_mode {
        TextSpacingMode::Natural => NATURAL_MODE_HASH_TAG,
        TextSpacingMode::Verbatim => VERBATIM_MODE_HASH_TAG,
    };
    hash = hash.wrapping_mul(LAYOUT_HASH_FNV_PRIME);
    hash ^= semantic_version;
    hash = hash.wrapping_mul(LAYOUT_HASH_FNV_PRIME);
    for range in protected_ranges {
        hash ^= range.start as u64;
        hash = hash.wrapping_mul(LAYOUT_HASH_FNV_PRIME);
        hash ^= range.end as u64;
        hash = hash.wrapping_mul(LAYOUT_HASH_FNV_PRIME);
    }
    hash
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
                                Some(s) => process_with_shaper(s, &req),
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
    shaper.set_font_family(Some(&req.font_family));
    shaper.set_font_size(req.font_size);
    let bytes = &req.line_bytes;
    let max_bytes =
        if req.max_line_bytes > 0 { req.max_line_bytes.min(bytes.len()) } else { bytes.len() };

    let source_line_str = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(error) => std::str::from_utf8(&bytes[..error.valid_up_to()]).unwrap_or(""),
    };
    let line_str = match std::str::from_utf8(&bytes[..max_bytes]) {
        Ok(s) => s,
        Err(e) => std::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap_or(""),
    };

    if line_str.is_empty() {
        let content_hash = req.content_hash();
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
    let typography_clusters: Vec<TypographyCluster> = shaped
        .clusters
        .iter()
        .map(|cluster| TypographyCluster::new(cluster.byte_range.clone(), req.font_size))
        .collect();
    let typography_input = TypographyInput::new(
        source_line_str,
        req.spacing_mode,
        &typography_clusters,
        &req.protected_ranges,
    );
    let gaps = ui::typography::calculate_gaps(&typography_input);
    let layout = ui::layout::typography::layout_shaped_run_with_gaps(
        &shaped,
        &gaps,
        source_line_str.as_bytes(),
        char_width,
        viewport_width,
        0.5,
        0.0,
    );

    let mut breaks: smallvec::SmallVec<[VisualBreak; 1]> = smallvec::SmallVec::new();
    for (start, end, pixel_width) in layout.visual_lines {
        let byte_start = layout.shaped.clusters[start].byte_range.start as u32;
        let byte_end = layout.shaped.clusters[end - 1].byte_range.end as u32;
        breaks.push(VisualBreak { byte_start, byte_end, pixel_width });
    }

    let visual_line_count = breaks.len().max(1) as u16;
    let content_hash = req.content_hash();

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
        let content_hash = req.content_hash();
        return DisplayLineEntry::placeholder(0, bytes.len() as u32, content_hash, 1);
    }

    let source_line_str = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(error) => std::str::from_utf8(&bytes[..error.valid_up_to()]).unwrap_or(""),
    };
    let line_str = match std::str::from_utf8(&bytes[..max_bytes]) {
        Ok(s) => s,
        Err(e) => std::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap_or(""),
    };
    if line_str.is_empty() {
        let content_hash = req.content_hash();
        return DisplayLineEntry::placeholder(0, bytes.len() as u32, content_hash, 1);
    }

    // Per-character width: CJK/wide chars use font_size (1em), ASCII uses font_size*0.6
    let char_w = |ch: char| -> f32 {
        if ch == '\t' {
            return ascii_w * ui::layout::DEFAULT_TAB_WIDTH as f32;
        }
        if ui::layout::is_cjk_char(ch) { cjk_w } else { ascii_w }
    };

    let fallback_clusters = approximate_grapheme_clusters(line_str, char_w);

    if fallback_clusters.is_empty() {
        let content_hash = req.content_hash();
        return DisplayLineEntry::placeholder(0, bytes.len() as u32, content_hash, 1);
    }

    let typography_clusters: Vec<TypographyCluster> = fallback_clusters
        .iter()
        .map(|cluster| TypographyCluster::new(cluster.byte_range.clone(), req.font_size))
        .collect();
    let typography_input = TypographyInput::new(
        source_line_str,
        req.spacing_mode,
        &typography_clusters,
        &req.protected_ranges,
    );
    let gaps = ui::typography::calculate_gaps(&typography_input);
    let shaped = shaping::ShapedRun {
        width: fallback_clusters.iter().map(|cluster| cluster.advance).sum(),
        clusters: fallback_clusters,
    };
    let layout = ui::layout::typography::layout_shaped_run_with_gaps(
        &shaped,
        &gaps,
        source_line_str.as_bytes(),
        ascii_w,
        req.viewport_width.max(1.0),
        FALLBACK_MIN_FILL_RATIO,
        0.0,
    );

    let mut breaks: smallvec::SmallVec<[VisualBreak; 1]> = smallvec::SmallVec::new();
    for (start, end, pixel_width) in layout.visual_lines {
        let Some(first_cluster) = layout.shaped.clusters.get(start) else {
            continue;
        };
        let Some(last_cluster) = layout.shaped.clusters.get(end.saturating_sub(1)) else {
            continue;
        };
        breaks.push(VisualBreak {
            byte_start: first_cluster.byte_range.start as u32,
            byte_end: last_cluster.byte_range.end as u32,
            pixel_width,
        });
    }

    let visual_line_count = breaks.len().max(1) as u16;
    let content_hash = req.content_hash();

    DisplayLineEntry {
        visual_line_count,
        visual_breaks: breaks,
        byte_offset: req.byte_offset,
        byte_length: req.byte_length,
        content_hash,
    }
}

/// Keep estimated widths on the same UAX#29 boundaries used by editing.
fn approximate_grapheme_clusters(
    text: &str,
    character_width: impl Fn(char) -> f32,
) -> Vec<shaping::GlyphCluster> {
    let mut clusters = Vec::new();
    let mut characters = text.char_indices().peekable();
    while let Some((byte_start, character)) = characters.next() {
        let mut byte_end = byte_start + character.len_utf8();
        let mut advance = character_width(character);
        let mut previous_properties = ucd_grapheme_cluster_lookup(character);
        let mut join_state = 0;
        while let Some(&(next_byte, next_character)) = characters.peek() {
            let next_properties = ucd_grapheme_cluster_lookup(next_character);
            join_state =
                ucd_grapheme_cluster_joins(join_state, previous_properties, next_properties);
            if ucd_grapheme_cluster_joins_done(join_state) {
                break;
            }
            previous_properties = next_properties;
            characters.next();
            byte_end = next_byte + next_character.len_utf8();
            advance = advance.max(character_width(next_character));
        }
        clusters.push(shaping::GlyphCluster {
            byte_range: byte_start..byte_end,
            advance,
            glyph_id: 0,
            font_id: Default::default(),
            x_offset: 0.0,
            y_offset: 0.0,
        });
    }
    clusters
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_change_invalidates_the_shared_layout_hash() {
        let hash = |font_family| {
            content_hash_for_layout(
                b"Skill agent loop",
                0,
                800.0,
                16.0,
                TextSpacingMode::Natural,
                &[],
                1,
                font_family,
            )
        };
        assert_ne!(hash("Menlo"), hash("Helvetica"));
    }

    #[test]
    fn worker_uses_each_requests_font_instead_of_its_startup_font() {
        let fonts = Arc::new(std::sync::Mutex::new(shaping::FontSystem::new()));
        let worker = ReshapeWorker::spawn(Arc::clone(&fonts), 16.0, "Menlo".into());
        let source = "iiiiiiiiiiii";
        let mut expected = shaping::Shaper::from_shared_font_system(fonts, 16.0, "Helvetica");
        let expected_width = expected.shape(source).expect("reference text must shape").width;
        worker
            .submit(ReshapeRequest {
                font_family: "Helvetica".into(),
                generation: 1,
                doc_line: 0,
                byte_offset: 0,
                byte_length: source.len() as u32,
                line_bytes: Arc::from(source.as_bytes()),
                viewport_width: 800.0,
                font_size: 16.0,
                max_line_bytes: 0,
                spacing_mode: TextSpacingMode::Natural,
                protected_ranges: Arc::from([]),
                semantic_version: 1,
                dv_idx: 0,
            })
            .expect("worker must accept font change request");
        let completed = recv_one(&worker, std::time::Duration::from_secs(2));
        worker.shutdown();
        assert!(
            (completed.entry.visual_breaks[0].pixel_width - expected_width).abs() < 0.01,
            "worker should shape using requested Helvetica instead of startup Menlo"
        );
    }

    #[test]
    fn process_empty_line() {
        let req = ReshapeRequest {
            font_family: "Menlo".into(),
            generation: 1,
            doc_line: 0,
            line_bytes: Arc::from(vec![].into_boxed_slice()),
            viewport_width: 800.0,
            font_size: 14.0,
            max_line_bytes: 0,
            spacing_mode: TextSpacingMode::Natural,
            protected_ranges: Arc::from([]),
            semantic_version: 0,
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
            font_family: "Menlo".into(),
            generation: 1,
            doc_line: 0,
            line_bytes: Arc::from(b"hello".to_vec().into_boxed_slice()),
            viewport_width: 800.0,
            font_size: 14.0,
            max_line_bytes: 0,
            spacing_mode: TextSpacingMode::Natural,
            protected_ranges: Arc::from([]),
            semantic_version: 0,
            dv_idx: 0,
            byte_offset: 0,
            byte_length: 5,
        };
        let entry = process_fallback(&req);
        assert_eq!(entry.visual_line_count, 1);
    }

    #[test]
    fn fallback_entries_do_not_share_hash_for_equal_length_different_content() {
        let build_request = |line: &str| ReshapeRequest {
            font_family: "Menlo".into(),
            generation: 1,
            doc_line: 0,
            line_bytes: Arc::from(line.as_bytes()),
            viewport_width: 800.0,
            font_size: 14.0,
            max_line_bytes: 0,
            spacing_mode: TextSpacingMode::Natural,
            protected_ranges: Arc::from([]),
            semantic_version: 0,
            dv_idx: 0,
            byte_offset: 0,
            byte_length: line.len() as u32,
        };
        let ascii_entry = process_fallback(&build_request("# tokens"));
        let cjk_entry = process_fallback(&build_request("# 企业"));

        assert_ne!(ascii_entry.content_hash, cjk_entry.content_hash);
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
            font_family: "Menlo".into(),
            generation: 1,
            doc_line: 0,
            line_bytes: Arc::from(b"test".to_vec().into_boxed_slice()),
            viewport_width: 200.0,
            font_size: 14.0,
            max_line_bytes: 0,
            spacing_mode: TextSpacingMode::Natural,
            protected_ranges: Arc::from([]),
            semantic_version: 0,
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
            font_family: "Menlo".into(),
            generation: 99,
            doc_line: 0,
            line_bytes: Arc::from(b"old".to_vec().into_boxed_slice()),
            viewport_width: 800.0,
            font_size: 14.0,
            max_line_bytes: 0,
            spacing_mode: TextSpacingMode::Natural,
            protected_ranges: Arc::from([]),
            semantic_version: 0,
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
            font_family: "Menlo".into(),
            generation: 1,
            doc_line: 0,
            line_bytes: Arc::from(big.into_boxed_slice()),
            viewport_width: 800.0,
            font_size: 14.0,
            max_line_bytes: 1000,
            spacing_mode: TextSpacingMode::Natural,
            protected_ranges: Arc::from([]),
            semantic_version: 0,
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
            font_family: "Menlo".into(),
            generation: 1,
            doc_line: 0,
            line_bytes: Arc::from(data.to_vec().into_boxed_slice()),
            viewport_width: 800.0,
            font_size: 14.0,
            max_line_bytes: 0,
            spacing_mode: TextSpacingMode::Natural,
            protected_ranges: Arc::from([]),
            semantic_version: 0,
            dv_idx: 0,
            byte_offset: 0,
            byte_length: data.len() as u32,
        });
        let r = recv_one(&w, std::time::Duration::from_secs(2));
        assert_eq!(r.entry.byte_length, data.len() as u32);
        w.shutdown();
    }

    #[test]
    fn fallback_natural_spacing_changes_final_line_geometry() {
        let build_request = |spacing_mode| ReshapeRequest {
            font_family: "Menlo".into(),
            generation: 1,
            doc_line: 0,
            line_bytes: Arc::from("甲,乙".as_bytes()),
            viewport_width: 100.0,
            font_size: 10.0,
            max_line_bytes: 0,
            dv_idx: 0,
            byte_offset: 0,
            byte_length: "甲,乙".len() as u32,
            spacing_mode,
            protected_ranges: Arc::from([]),
            semantic_version: 0,
        };

        let natural = process_fallback(&build_request(ui::typography::TextSpacingMode::Natural));
        let verbatim = process_fallback(&build_request(ui::typography::TextSpacingMode::Verbatim));

        assert!(natural.visual_breaks[0].pixel_width > verbatim.visual_breaks[0].pixel_width);
    }

    #[test]
    fn fallback_keeps_combining_mark_with_base_for_wrap_boundary() {
        let data = "e\u{301}x";
        let req = ReshapeRequest {
            font_family: "Menlo".into(),
            generation: 1,
            doc_line: 0,
            line_bytes: Arc::from(data.as_bytes()),
            viewport_width: 10.0,
            font_size: 14.0,
            max_line_bytes: 0,
            spacing_mode: TextSpacingMode::Natural,
            protected_ranges: Arc::from([]),
            semantic_version: 0,
            dv_idx: 0,
            byte_offset: 0,
            byte_length: data.len() as u32,
        };

        let entry = process_fallback(&req);
        assert_eq!(entry.visual_breaks[0].byte_end, 3);
        assert_eq!(entry.visual_breaks[1].byte_start, 3);
    }

    #[test]
    fn fallback_does_not_split_extended_grapheme_clusters() {
        for grapheme in ["👩‍💻", "🇨🇳", "👍🏽", "e\u{301}"] {
            let source = format!("{grapheme}x");
            let request = ReshapeRequest {
                font_family: "Menlo".into(),
                generation: 1,
                doc_line: 0,
                line_bytes: Arc::from(source.as_bytes()),
                viewport_width: 1.0,
                font_size: 14.0,
                max_line_bytes: 0,
                spacing_mode: TextSpacingMode::Natural,
                protected_ranges: Arc::from([]),
                semantic_version: 0,
                dv_idx: 0,
                byte_offset: 0,
                byte_length: source.len() as u32,
            };
            let entry = process_fallback(&request);
            assert_eq!(entry.visual_breaks[0].byte_end as usize, grapheme.len(), "{source}");
            assert_eq!(entry.visual_breaks[1].byte_start as usize, grapheme.len(), "{source}");
        }
    }

    #[test]
    fn fallback_truncated_subset_keeps_full_quote_context_for_gaps() {
        let full_source = "甲\"x\"乙";
        let visible_prefix = "甲\"x";
        let build_request = |line: &str, max_line_bytes| ReshapeRequest {
            font_family: "Menlo".into(),
            generation: 1,
            doc_line: 0,
            line_bytes: Arc::from(line.as_bytes()),
            viewport_width: 200.0,
            font_size: 14.0,
            max_line_bytes,
            spacing_mode: TextSpacingMode::Natural,
            protected_ranges: Arc::from([]),
            semantic_version: 0,
            dv_idx: 0,
            byte_offset: 0,
            byte_length: line.len() as u32,
        };

        let truncated = process_fallback(&build_request(full_source, visible_prefix.len()));
        let isolated = process_fallback(&build_request(visible_prefix, 0));

        assert!(
            truncated.visual_breaks[0].pixel_width > isolated.visual_breaks[0].pixel_width,
            "full source quote pairing should preserve the opening quote gap"
        );
    }

    #[test]
    fn fallback_long_number_backtrack() {
        // Long ASCII number without spaces: should backtrack to before the number.
        let data = b"ID: 123456789012345678901234567890";
        let req = ReshapeRequest {
            font_family: "Menlo".into(),
            generation: 1,
            doc_line: 0,
            line_bytes: Arc::from(data.to_vec().into_boxed_slice()),
            viewport_width: 120.0,
            font_size: 14.0,
            max_line_bytes: 0,
            spacing_mode: TextSpacingMode::Natural,
            protected_ranges: Arc::from([]),
            semantic_version: 0,
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
            font_family: "Menlo".into(),
            generation: 1,
            doc_line: 0,
            line_bytes: Arc::from(big.into_boxed_slice()),
            viewport_width: 800.0,
            font_size: 14.0,
            max_line_bytes: 0,
            spacing_mode: TextSpacingMode::Natural,
            protected_ranges: Arc::from([]),
            semantic_version: 0,
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
            font_family: "Menlo".into(),
            generation: 1,
            doc_line: 0,
            line_bytes: Arc::from(data.to_vec().into_boxed_slice()),
            viewport_width: 120.0,
            font_size: 14.0,
            max_line_bytes: 0,
            spacing_mode: TextSpacingMode::Natural,
            protected_ranges: Arc::from([]),
            semantic_version: 0,
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
