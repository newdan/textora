//! 绘制命令枚举与命令列表。
//! ui crate 内部除本文件外不应出现 NDC 形态的坐标。

use crate::core::geom::Rect;
use crate::core::text_layout::UiTextLayout;
use crate::tapered_path::TaperedMesh;
use shaping::{Style, Weight};
use std::sync::Arc;

/// 绘制命令：widget 输出语义化绘图指令，
/// 由 app 端 paint_backend 翻译为 GPU 顶点。
#[derive(Clone, Debug, PartialEq)]
pub enum DrawCmd {
    /// 填充矩形
    FillRect { rect: Rect, color: [f32; 4], radius: f32 },
    /// 描边矩形（仅轮廓，line_width 为像素宽度）
    StrokeRect { rect: Rect, color: [f32; 4], radius: f32, line_width: f32 },
    /// 预 shape 的文本布局 — 携带 harfbuzz 结果和绘制参数。
    /// app 层 drain 时做 atlas rasterize + emit。
    TextLayout { layout: Arc<UiTextLayout>, x: f32, y_baseline: f32, color: [f32; 4] },
    /// 填充三角形（3 个顶点，像素坐标）
    FillTriangle { p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], color: [f32; 4] },
    /// 共享的渐变宽度路径网格（顶点坐标为像素坐标）。
    TaperedMesh { mesh: Arc<TaperedMesh>, translation: [f32; 2], color: [f32; 4] },
    /// 推入裁剪区域
    PushClip(Rect),
    /// 弹出裁剪区域
    PopClip,
}

/// 绘制命令列表。widget 通过 helper 方法往里追加命令。
#[derive(Clone, Debug, PartialEq)]
pub struct DrawList {
    pub cmds: Vec<DrawCmd>,
    /// 容器维护的累计坐标偏移。Widget 调用 helper 方法时，
    /// 实际写入 DrawCmd 的坐标 = 传入坐标 + offset。
    pub offset: (f32, f32),
}

impl Default for DrawList {
    fn default() -> Self {
        Self::new()
    }
}

impl DrawList {
    pub fn new() -> Self {
        Self { cmds: Vec::new(), offset: (0.0, 0.0) }
    }

    pub fn fill(&mut self, rect: Rect, color: [f32; 4]) {
        self.fill_rounded(rect, color, 0.0);
    }

    pub fn fill_rounded(&mut self, rect: Rect, color: [f32; 4], radius: f32) {
        let r = Rect::new(rect.x + self.offset.0, rect.y + self.offset.1, rect.w, rect.h);
        self.cmds.push(DrawCmd::FillRect { rect: r, color, radius });
    }

    pub fn stroke(&mut self, rect: Rect, color: [f32; 4], line_width: f32) {
        let r = Rect::new(rect.x + self.offset.0, rect.y + self.offset.1, rect.w, rect.h);
        self.cmds.push(DrawCmd::StrokeRect { rect: r, color, radius: 0.0, line_width });
    }

    /// 绘制填充三角形（像素坐标，3 个顶点）
    pub fn fill_triangle(&mut self, p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], color: [f32; 4]) {
        let o = self.offset;
        self.cmds.push(DrawCmd::FillTriangle {
            p0: [p0[0] + o.0, p0[1] + o.1],
            p1: [p1[0] + o.0, p1[1] + o.1],
            p2: [p2[0] + o.0, p2[1] + o.1],
            color,
        });
    }

    /// 绘制共享的渐变宽度路径网格。
    pub fn tapered_mesh(&mut self, mesh: Arc<TaperedMesh>, translation: [f32; 2], color: [f32; 4]) {
        self.cmds.push(DrawCmd::TaperedMesh {
            mesh,
            translation: [translation[0] + self.offset.0, translation[1] + self.offset.1],
            color,
        });
    }

    pub fn stroke_rounded(&mut self, rect: Rect, color: [f32; 4], radius: f32, line_width: f32) {
        let r = Rect::new(rect.x + self.offset.0, rect.y + self.offset.1, rect.w, rect.h);
        self.cmds.push(DrawCmd::StrokeRect { rect: r, color, radius, line_width });
    }

    /// 使用预 shape 的 UiTextLayout 绘制文本。
    pub fn text_layout(
        &mut self,
        layout: Arc<UiTextLayout>,
        x: f32,
        y_baseline: f32,
        color: [f32; 4],
    ) {
        self.cmds.push(DrawCmd::TextLayout {
            layout,
            x: x + self.offset.0,
            y_baseline: y_baseline + self.offset.1,
            color,
        });
    }

    /// Shape text via harfbuzz and emit a TextLayout command (default weight/style).
    /// Convenience for UI widgets; for repeated text prefer pre-building a UiTextLayout.
    pub fn text_shaped(
        &mut self,
        x: f32,
        y_baseline: f32,
        font_size: f32,
        color: [f32; 4],
        text: &str,
        shaper: &mut shaping::Shaper,
    ) -> f32 {
        self.text_shaped_with_font(
            x,
            y_baseline,
            font_size,
            color,
            text,
            None,
            Weight::NORMAL,
            Style::Normal,
            false,
            shaper,
        )
    }

    /// Shape text with explicit font family, weight, and style.
    /// Returns the actual shaped width (in pixels) so callers can advance cursors precisely.
    #[allow(
        clippy::too_many_arguments,
        reason = "draw command boundary keeps font and placement attributes explicit"
    )]
    pub fn text_shaped_with_font(
        &mut self,
        x: f32,
        y_baseline: f32,
        font_size: f32,
        color: [f32; 4],
        text: &str,
        font_family: Option<String>,
        font_weight: Weight,
        font_style: Style,
        italic: bool,
        shaper: &mut shaping::Shaper,
    ) -> f32 {
        let layout = UiTextLayout::new(
            text,
            font_size,
            font_family,
            font_weight,
            font_style,
            italic,
            shaper,
        );
        if let Some(layout) = layout {
            let w = layout.shaped.width;
            self.text_layout(Arc::new(layout), x, y_baseline, color);
            w
        } else {
            0.0
        }
    }

    /// 推入裁剪区域，执行闭包，再弹出裁剪。
    /// 保证 PushClip / PopClip 配对。
    pub fn clip<F: FnOnce(&mut DrawList)>(&mut self, rect: Rect, f: F) {
        let r = Rect::new(rect.x + self.offset.0, rect.y + self.offset.1, rect.w, rect.h);
        self.cmds.push(DrawCmd::PushClip(r));
        f(self);
        self.cmds.push(DrawCmd::PopClip);
    }

    /// Draw a menu-style hover highlight: shrinks the rect and fills with rounded corners.
    pub fn fill_menu_hover(&mut self, rect: Rect, color: [f32; 4], dpi: f32) {
        let pad_x = 2.0 * dpi;
        let pad_y = 1.0 * dpi;
        let hr = rect.shrink(pad_y, pad_x, pad_y, pad_x);
        if hr.w > 0.0 && hr.h > 0.0 {
            self.fill_rounded(hr, color, 8.0 * dpi);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tapered_mesh_command_shares_geometry_and_applies_draw_list_offset() {
        use crate::tapered_path::{
            TAPERED_PATH_FEATHER_PX, TaperedPathInput, tessellate_tapered_path,
        };

        let centerline = [[0.0, 0.0], [100.0, 0.0]];
        let mesh = Arc::new(
            tessellate_tapered_path(TaperedPathInput {
                centerline: &centerline,
                head_width: 10.0,
                tail_width: 2.0,
                scale: 1.0,
                feather_width: TAPERED_PATH_FEATHER_PX,
            })
            .expect("fixture must tessellate"),
        );
        let mut draw_list = DrawList::new();
        draw_list.offset = (7.0, 11.0);
        draw_list.tapered_mesh(Arc::clone(&mesh), [13.0, 17.0], [0.2, 0.4, 0.6, 0.8]);
        let cloned = draw_list.clone();

        match (&draw_list.cmds[0], &cloned.cmds[0]) {
            (
                DrawCmd::TaperedMesh { mesh: first, translation, color },
                DrawCmd::TaperedMesh { mesh: second, .. },
            ) => {
                assert!(Arc::ptr_eq(first, &mesh));
                assert!(Arc::ptr_eq(first, second));
                assert_eq!(*translation, [20.0, 28.0]);
                assert_eq!(*color, [0.2, 0.4, 0.6, 0.8]);
            }
            _ => panic!("expected one shared tapered mesh command"),
        }
    }

    #[test]
    fn fill_emits_fillrect_command() {
        let mut dl = DrawList::new();
        dl.fill(Rect::new(10.0, 20.0, 100.0, 50.0), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(dl.cmds.len(), 1);
        match &dl.cmds[0] {
            DrawCmd::FillRect { rect, color, radius } => {
                assert_eq!(rect.x, 10.0);
                assert_eq!(rect.y, 20.0);
                assert_eq!(rect.w, 100.0);
                assert_eq!(rect.h, 50.0);
                assert_eq!(*color, [1.0, 0.0, 0.0, 1.0]);
                assert_eq!(*radius, 0.0);
            }
            _ => panic!("expected FillRect"),
        }
    }

    #[test]
    fn fill_rounded_sets_radius() {
        let mut dl = DrawList::new();
        dl.fill_rounded(Rect::new(0.0, 0.0, 10.0, 10.0), [0.0; 4], 5.0);
        match &dl.cmds[0] {
            DrawCmd::FillRect { radius, .. } => assert_eq!(*radius, 5.0),
            _ => panic!("expected FillRect"),
        }
    }

    #[test]
    fn text_layout_preserves_color() {
        let text = "hello";
        let mut shaper = shaping::Shaper::new().unwrap();
        let layout =
            UiTextLayout::new(text, 14.0, None, Weight::NORMAL, Style::Normal, false, &mut shaper)
                .unwrap();
        let mut dl = DrawList::new();
        dl.text_layout(Arc::new(layout), 10.0, 20.0, [0.2, 0.4, 0.6, 0.8]);
        match &dl.cmds[0] {
            DrawCmd::TextLayout { color, .. } => assert_eq!(*color, [0.2, 0.4, 0.6, 0.8]),
            _ => panic!("expected TextLayout"),
        }
    }

    #[test]
    fn text_shaped_emits_textlayout() {
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut dl = DrawList::new();
        dl.text_shaped(50.0, 30.0, 14.0, [0.0, 0.0, 0.0, 1.0], "hello", &mut shaper);
        match &dl.cmds[0] {
            DrawCmd::TextLayout { x, y_baseline, .. } => {
                assert_eq!(*x, 50.0);
                assert_eq!(*y_baseline, 30.0);
            }
            _ => panic!("expected TextLayout"),
        }
    }

    #[test]
    fn text_shaped_empty_returns_none() {
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut dl = DrawList::new();
        dl.text_shaped(0.0, 0.0, 14.0, [0.0; 4], "", &mut shaper);
        assert!(dl.cmds.is_empty(), "empty text should not emit command");
    }

    #[test]
    fn clip_emits_push_then_inner_then_pop() {
        let mut dl = DrawList::new();
        dl.clip(Rect::new(0.0, 0.0, 100.0, 100.0), |inner| {
            inner.fill(Rect::new(10.0, 10.0, 80.0, 80.0), [1.0, 1.0, 1.0, 1.0]);
        });
        assert_eq!(dl.cmds.len(), 3);
        assert!(matches!(dl.cmds[0], DrawCmd::PushClip(_)));
        assert!(matches!(dl.cmds[1], DrawCmd::FillRect { .. }));
        assert!(matches!(dl.cmds[2], DrawCmd::PopClip));
    }

    #[test]
    fn nested_clip_emits_balanced_push_pop() {
        let mut dl = DrawList::new();
        dl.clip(Rect::new(0.0, 0.0, 200.0, 200.0), |outer| {
            outer.fill(Rect::new(0.0, 0.0, 200.0, 200.0), [0.5; 4]);
            outer.clip(Rect::new(50.0, 50.0, 100.0, 100.0), |inner| {
                inner.fill(Rect::new(50.0, 50.0, 100.0, 100.0), [1.0, 0.0, 0.0, 1.0]);
            });
        });
        assert_eq!(dl.cmds.len(), 6);
        let push_count = dl.cmds.iter().filter(|c| matches!(c, DrawCmd::PushClip(_))).count();
        let pop_count = dl.cmds.iter().filter(|c| matches!(c, DrawCmd::PopClip)).count();
        assert_eq!(push_count, pop_count);
        assert_eq!(push_count, 2);
    }

    #[test]
    fn fill_applies_offset() {
        let mut dl = DrawList::new();
        dl.offset = (100.0, 200.0);
        dl.fill(Rect::new(10.0, 20.0, 50.0, 30.0), [1.0, 0.0, 0.0, 1.0]);
        match &dl.cmds[0] {
            DrawCmd::FillRect { rect, .. } => {
                assert_eq!(rect.x, 110.0);
                assert_eq!(rect.y, 220.0);
                assert_eq!(rect.w, 50.0);
                assert_eq!(rect.h, 30.0);
            }
            _ => panic!("expected FillRect"),
        }
    }

    #[test]
    fn text_layout_applies_offset() {
        let mut shaper = shaping::Shaper::new().unwrap();
        let layout =
            UiTextLayout::new("hi", 14.0, None, Weight::NORMAL, Style::Normal, false, &mut shaper)
                .unwrap();
        let mut dl = DrawList::new();
        dl.offset = (50.0, 100.0);
        dl.text_layout(Arc::new(layout), 10.0, 20.0, [0.0; 4]);
        match &dl.cmds[0] {
            DrawCmd::TextLayout { x, y_baseline, .. } => {
                assert_eq!(*x, 60.0);
                assert_eq!(*y_baseline, 120.0);
            }
            _ => panic!("expected TextLayout"),
        }
    }

    #[test]
    fn clip_applies_offset_to_pushclip() {
        let mut dl = DrawList::new();
        dl.offset = (10.0, 20.0);
        dl.clip(Rect::new(0.0, 0.0, 100.0, 100.0), |inner| {
            inner.fill(Rect::new(0.0, 0.0, 100.0, 100.0), [1.0; 4]);
        });
        match &dl.cmds[0] {
            DrawCmd::PushClip(r) => {
                assert_eq!(r.x, 10.0);
                assert_eq!(r.y, 20.0);
            }
            _ => panic!("expected PushClip"),
        }
    }

    #[test]
    fn zero_offset_is_identity() {
        let mut dl = DrawList::new();
        assert_eq!(dl.offset, (0.0, 0.0));
        dl.fill(Rect::new(5.0, 5.0, 10.0, 10.0), [0.0; 4]);
        match &dl.cmds[0] {
            DrawCmd::FillRect { rect, .. } => {
                assert_eq!(rect.x, 5.0);
                assert_eq!(rect.y, 5.0);
            }
            _ => panic!("expected FillRect"),
        }
    }

    #[test]
    fn text_shaped_bold_weight() {
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut dl = DrawList::new();
        dl.text_shaped_with_font(
            10.0,
            20.0,
            14.0,
            [0.0; 4],
            "bold",
            None,
            Weight::BOLD,
            Style::Normal,
            false,
            &mut shaper,
        );
        match &dl.cmds[0] {
            DrawCmd::TextLayout { layout, .. } => {
                assert_eq!(layout.font_weight, Weight::BOLD);
                assert!(layout.font_style == Style::Normal);
                assert_eq!(layout.text, "bold");
            }
            _ => panic!("expected TextLayout"),
        }
    }

    #[test]
    fn text_shaped_italic_style() {
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut dl = DrawList::new();
        dl.text_shaped_with_font(
            10.0,
            20.0,
            14.0,
            [0.0; 4],
            "italic",
            None,
            Weight::NORMAL,
            Style::Italic,
            true,
            &mut shaper,
        );
        match &dl.cmds[0] {
            DrawCmd::TextLayout { layout, .. } => {
                assert_eq!(layout.font_weight, Weight::NORMAL);
                assert!(layout.font_style == Style::Italic);
                assert!(layout.italic);
            }
            _ => panic!("expected TextLayout"),
        }
    }
}
