//! Layout pass — computes positions and sizes for all blocks.
//!
//! This module is organized into submodules:
//! - `types`: All layout output types and `LazyLayout` implementation
//! - `context`: `LayoutCtx` and text wrapping/width estimation helpers
//! - `block`: Block-level layout functions (`layout_doc`, `layout_block`, etc.)
//! - `shaping`: Shaping helpers (`shape_line`, `compute_style_segments`, etc.)

pub(crate) mod ascii_diagram;
pub mod block;
pub mod context;
pub mod shaping;
pub mod source_line_map;
pub mod types;

// ===== Re-exports for external callers =====
#[allow(unused_imports)]
pub(crate) use ascii_diagram::{
    AsciiDiagram, AsciiDiagramRegistry, AsciiDiagramRow, BoxConnections, detect_ascii_diagram,
};
pub use block::{
    MarkdownLayout, layout_doc, layout_doc_for_rendering, layout_doc_with_shaper,
    layout_doc_with_shaper_for_rendering,
};
pub use context::{LayoutCtx, is_cjk_or_fullwidth};
pub use types::{
    FlatLine, LaidOutBlock, LaidOutBlockKind, LaidOutDoc, LaidOutLine, LazyLayout, StyleSegment,
};

// ===== BlockSource trait =====

/// A single heading entry for the table of contents.
#[derive(Clone, Debug, Default)]
pub struct HeadingEntry {
    pub text: String,
    pub level: u8,
    pub y_offset: f32,
}

/// LazyLayout 通过此 trait 查询文档结构，而不直接依赖 MarkdownDoc。
/// Phase 3 将引入 NovelStructure 作为轻量级替代实现。
pub trait BlockSource {
    /// 返回顶层文档 block 树，供树遍历使用（build_flat_lines、line_byte_offsets 等）。
    fn blocks(&self) -> &[crate::builder::BlockNode];

    /// 返回文档级标题列表（用于 ToC）。
    /// MarkdownDoc 暂返回空切片；标题由 view 层从 laid-out blocks 收集。
    fn headings(&self) -> &[HeadingEntry];
}

// pub(crate) re-exports needed by other modules in this crate
pub(crate) use context::{grapheme_at_x, grapheme_x};

// ===== Top-level functions =====

/// Batch-propagate height deltas into y_delta array.
/// `height_deltas` must be sorted by block_idx ascending.
/// A block's own height change only shifts blocks i+1 and beyond.
pub fn apply_deltas(y_delta: &mut [f32], height_deltas: &[(usize, f32)]) {
    if height_deltas.is_empty() {
        return;
    }
    let mut cum: f32 = 0.0;
    let mut di = 0usize;
    for i in 0..y_delta.len() {
        while di < height_deltas.len() && height_deltas[di].0 < i {
            cum += height_deltas[di].1;
            di += 1;
        }
        y_delta[i] += cum;
    }
}

/// Scale heading top spacing by level: H1 keeps full, H2-H3 80%, H4-H6 65%.
pub(crate) fn heading_spacing_scale(level: u8) -> f32 {
    if level <= 1 {
        1.0
    } else if level <= 3 {
        0.8
    } else {
        0.65
    }
}

/// Flatten a list of block nodes by recursively expanding Container blocks.
/// This produces a 1:1 correspondence with the output of layout_doc.
/// Count how many laid-out blocks a Container's children will produce.
pub(crate) fn count_laid_out_blocks(blocks: &[crate::builder::BlockNode]) -> usize {
    let mut count = 0;
    for block in blocks {
        match block.kind {
            crate::builder::BlockKind::Container => count += count_laid_out_blocks(&block.children),
            _ => count += 1,
        }
    }
    count
}

pub(crate) fn flatten_blocks(blocks: &[crate::builder::BlockNode]) -> Vec<usize> {
    let mut result = Vec::new();
    for (i, block) in blocks.iter().enumerate() {
        match block.kind {
            crate::builder::BlockKind::Container => {
                let n = count_laid_out_blocks(&block.children);
                result.extend(std::iter::repeat_n(i, n));
            }
            _ => {
                result.push(i);
            }
        }
    }
    result
}
