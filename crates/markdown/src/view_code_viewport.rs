//! Keep overlays inside the same code viewport that clips the rendered glyphs.

use super::*;
use crate::layout::{LaidOutBlock, LaidOutBlockKind};
use ui::core::{geom::Rect, paint::DrawCmd};

impl<S: BlockSource> PreviewEngine<S> {
    pub(super) fn clip_code_highlights(&self, mut highlights: DrawList) -> DrawList {
        let Some(lazy) = &self.lazy else { return highlights };
        let mut code_viewports = Vec::new();
        for (index, block) in lazy.laid_out.iter().enumerate() {
            let Some(block) = block else { continue };
            collect_code_viewports(
                block,
                self.cached_offset_x,
                self.cached_offset_y - self.scroll_y
                    + lazy.y_delta.get(index).copied().unwrap_or(0.0),
                &mut code_viewports,
            );
        }
        highlights.cmds.retain_mut(|command| {
            let DrawCmd::FillRect { rect, .. } = command else { return true };
            let center_y = rect.y + rect.h * 0.5;
            let Some(viewport) = code_viewports
                .iter()
                .find(|viewport| center_y >= viewport.y && center_y < viewport.y + viewport.h)
            else {
                return true;
            };
            let right = (rect.x + rect.w).min(viewport.x + viewport.w);
            let bottom = (rect.y + rect.h).min(viewport.y + viewport.h);
            rect.x = rect.x.max(viewport.x);
            rect.y = rect.y.max(viewport.y);
            rect.w = (right - rect.x).max(0.0);
            rect.h = (bottom - rect.y).max(0.0);
            rect.w > 0.0 && rect.h > 0.0
        });
        highlights
    }
}

fn collect_code_viewports(
    block: &LaidOutBlock,
    offset_x: f32,
    offset_y: f32,
    clips: &mut Vec<Rect>,
) {
    match &block.kind {
        LaidOutBlockKind::CodeBlock { .. } => clips.push(Rect::new(
            block.rect.x + offset_x,
            block.rect.y + offset_y,
            block.rect.w,
            block.rect.h,
        )),
        LaidOutBlockKind::BlockQuote { blocks } | LaidOutBlockKind::ListItem { blocks, .. } => {
            for child in blocks {
                collect_code_viewports(child, offset_x, offset_y, clips);
            }
        }
        _ => {}
    }
}
