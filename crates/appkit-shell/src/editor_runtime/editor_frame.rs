//! 产品 chrome 与编辑器共享的单帧借用 API。

use crate::editor_runtime::EditorOutcome;
use render::GlyphVertex;
use std::sync::{Arc, Mutex};

pub struct RenderResources {
    pub text: Option<crate::render_state::TextState>,
    pub gpu: Option<crate::render_state::GpuState>,
    pub frame_cache: crate::frame_cache::FrameCache,
}

/// 帧组合和提交错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    InvalidEditorRect,
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidEditorRect => {
                formatter.write_str("editor rect must be finite and non-negative")
            }
        }
    }
}

impl std::error::Error for RenderError {}

/// 在同一 DrawList 中组合产品 chrome 和编辑器内容的短生命周期对象。
pub struct EditorFrame {
    draw_list: ui::DrawList,
    text_measure: ui::NoopMeasure,
    theme: ui::Theme,
    dpi: f32,
    ui_shaper: Option<Arc<Mutex<shaping::Shaper>>>,
    editor_vertices: Vec<GlyphVertex>,
}

impl EditorFrame {
    pub(crate) fn new_for_backend(
        theme: ui::Theme,
        dpi: f32,
        ui_shaper: Option<Arc<Mutex<shaping::Shaper>>>,
    ) -> Self {
        Self {
            draw_list: ui::DrawList::new(),
            text_measure: ui::NoopMeasure,
            theme,
            dpi,
            ui_shaper,
            editor_vertices: Vec::new(),
        }
    }

    pub fn with_layout_context<T>(
        &mut self,
        layout: impl FnOnce(&mut ui::LayoutCtx<'_>) -> T,
    ) -> T {
        let mut context = ui::LayoutCtx {
            measure: &mut self.text_measure,
            ui_measure: None,
            theme: &self.theme,
            dpi: self.dpi,
        };
        layout(&mut context)
    }

    pub fn with_paint_context<T>(&mut self, paint: impl FnOnce(&mut ui::PaintCtx<'_>) -> T) -> T {
        let mut ui_shaper = self.ui_shaper.as_ref().map(|shaper| {
            shaper.lock().expect("UI shaper mutex must not be poisoned by another paint callback")
        });
        let mut context = ui::PaintCtx {
            list: &mut self.draw_list,
            theme: &self.theme,
            dpi: self.dpi,
            offset: (0.0, 0.0),
            global_alpha: 1.0,
            shaper: ui_shaper.as_deref_mut(),
        };
        paint(&mut context)
    }

    pub fn paint_editor(&mut self, editor_rect: ui::Rect) -> Result<(), RenderError> {
        self.paint_editor_with(editor_rect, |_| ()).map(|_| ())
    }

    pub fn paint_editor_with<T>(
        &mut self,
        editor_rect: ui::Rect,
        paint: impl FnOnce(&mut ui::PaintCtx<'_>) -> T,
    ) -> Result<Option<T>, RenderError> {
        if !is_valid_rect(editor_rect) {
            return Err(RenderError::InvalidEditorRect);
        }
        if editor_rect.w == 0.0 || editor_rect.h == 0.0 {
            return Ok(None);
        }
        let mut result = None;
        let theme = &self.theme;
        let dpi = self.dpi;
        let mut ui_shaper = self.ui_shaper.as_ref().map(|shaper| {
            shaper.lock().expect("UI shaper mutex must not be poisoned by another paint callback")
        });
        self.draw_list.clip(editor_rect, |draw_list| {
            let mut context = ui::PaintCtx {
                list: draw_list,
                theme,
                dpi,
                offset: (0.0, 0.0),
                global_alpha: 1.0,
                shaper: ui_shaper.as_deref_mut(),
            };
            result = Some(paint(&mut context));
        });
        Ok(result)
    }

    /// 接收编辑器已经按物理屏幕坐标生成的顶点，确保它们与产品 chrome 共用一帧。
    pub fn paint_editor_vertices(
        &mut self,
        editor_rect: ui::Rect,
        vertices: impl IntoIterator<Item = GlyphVertex>,
    ) -> Result<(), RenderError> {
        if !is_valid_rect(editor_rect) {
            return Err(RenderError::InvalidEditorRect);
        }
        if editor_rect.w == 0.0 || editor_rect.h == 0.0 {
            return Ok(());
        }
        self.editor_vertices.extend(vertices);
        Ok(())
    }

    pub fn drain_into(
        &mut self,
        screen: ui::Screen,
        resources: &mut RenderResources,
        vertices: &mut Vec<GlyphVertex>,
    ) {
        vertices.append(&mut self.editor_vertices);
        crate::paint_backend::drain_into(
            std::mem::take(&mut self.draw_list),
            screen,
            resources.text.as_mut(),
            resources.gpu.as_ref(),
            vertices,
        );
    }

    pub fn present(self) -> Result<EditorOutcome, RenderError> {
        Ok(EditorOutcome::default())
    }
}

fn is_valid_rect(rect: ui::Rect) -> bool {
    [rect.x, rect.y, rect.w, rect.h].into_iter().all(f32::is_finite)
        && rect.x >= 0.0
        && rect.y >= 0.0
        && rect.w >= 0.0
        && rect.h >= 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> ui::Theme {
        ui::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark())
    }

    #[test]
    fn layout_and_paint_contexts_are_only_borrowed_inside_callbacks() {
        let theme = theme();
        let mut frame = EditorFrame::new_for_backend(theme, 2.0, None);

        let layout_dpi = frame.with_layout_context(|context| context.dpi);
        let paint_dpi = frame.with_paint_context(|context| {
            context.list.fill(ui::Rect::new(1.0, 2.0, 3.0, 4.0), [1.0; 4]);
            context.dpi
        });

        assert_eq!(layout_dpi, 2.0);
        assert_eq!(paint_dpi, 2.0);
        frame.present().expect("frame should be consumed once");
    }

    #[test]
    fn product_text_is_shaped_into_the_frame_draw_list() {
        let theme = theme();
        let shaper = shaping::Shaper::new().expect("system fonts should create a UI shaper");
        let mut frame =
            EditorFrame::new_for_backend(theme, 1.0, Some(Arc::new(Mutex::new(shaper))));

        frame.with_paint_context(|context| {
            context.text(12.0, 24.0, 14.0, [1.0; 4], "Workspace");
        });

        assert!(
            frame
                .draw_list
                .cmds
                .iter()
                .any(|command| matches!(command, ui::DrawCmd::TextLayout { .. })),
            "product chrome text must produce a shaped draw command"
        );
    }

    #[test]
    fn product_editor_overlay_order_shares_one_draw_list() {
        let theme = theme();
        let mut frame = EditorFrame::new_for_backend(theme, 1.0, None);

        frame.with_paint_context(|context| {
            context.list.fill(ui::Rect::new(0.0, 0.0, 10.0, 10.0), [0.0; 4]);
        });
        frame
            .paint_editor(ui::Rect::new(24.0, 36.0, 400.0, 300.0))
            .expect("finite editor rect should paint");
        frame.with_paint_context(|context| {
            context.list.fill(ui::Rect::new(5.0, 5.0, 6.0, 6.0), [1.0; 4]);
        });
        frame.present().expect("frame should be consumed once");
    }

    #[test]
    fn invalid_or_zero_editor_rects_are_safe() {
        let theme = theme();
        let mut frame = EditorFrame::new_for_backend(theme, 1.0, None);

        assert_eq!(
            frame.paint_editor(ui::Rect::new(-1.0, 0.0, 10.0, 10.0)),
            Err(RenderError::InvalidEditorRect)
        );
        frame.paint_editor(ui::Rect::ZERO).expect("zero rect should skip safely");
        frame.present().expect("zero rect frame should still present");
    }

    #[test]
    fn editor_vertices_are_submitted_before_product_chrome() {
        let theme = theme();
        let mut frame = EditorFrame::new_for_backend(theme, 1.0, None);
        frame.with_paint_context(|context| {
            context.list.fill(ui::Rect::new(10.0, 10.0, 5.0, 5.0), [1.0; 4]);
        });
        frame
            .paint_editor_vertices(
                ui::Rect::new(20.0, 20.0, 80.0, 60.0),
                [GlyphVertex { position: [0.25, 0.5], tex_coords: [0.0, 0.0], color: [0.0; 4] }],
            )
            .expect("finite editor rect should accept editor vertices");

        let mut resources = RenderResources {
            text: None,
            gpu: None,
            frame_cache: crate::frame_cache::FrameCache::new(),
        };
        let mut vertices = Vec::new();
        frame.drain_into(ui::Screen::new(100.0, 100.0), &mut resources, &mut vertices);

        assert_eq!(vertices.first().expect("editor vertex must be retained").position, [0.25, 0.5]);
        assert_eq!(vertices.len(), 7, "one editor vertex plus one product quad");
        frame.present().expect("frame should be consumed once");
    }
}
