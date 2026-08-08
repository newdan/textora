//! Widget trait 与上下文类型。
//! 所有 UI 组件实现此 trait；app 层通过上下文体注入依赖。

use std::any::Any;

use crate::core::accessibility::{
    AccessibilityActionRequest, AccessibilityContext, AccessibilityNode,
};
use crate::core::geom::Rect;
use crate::core::measure::TextMeasure;
use crate::core::overlay::OverlayAction;
use crate::core::paint::DrawList;
use crate::theme::Theme;
use crate::widgets::settings_view::SettingsViewAction;
use crate::widgets::tooltip::TooltipHint;
use shaping::Shaper;

/// 全局唯一 Widget 标识符（基于手写常量）。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct WidgetId(pub u64);

/// 手写常量 WidgetId，统一集中管理。
pub mod ids {
    use super::WidgetId;

    /// 搜索栏
    pub const SEARCH_BAR: WidgetId = WidgetId(1);
    /// mmap 风格面板
    pub const MINDMAP_STYLE_PANEL: WidgetId = WidgetId(2);
}

/// 鼠标按键
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// 键盘按键（最小集合，按需扩展）
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum KeyCode {
    Escape,
    Enter,
    Backspace,
    Tab,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Char(char),
}

/// Keyboard modifier flags.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub cmd: bool,
    pub alt: bool,
    pub ctrl: bool,
}

impl Modifiers {
    pub const NONE: Self = Modifiers { shift: false, cmd: false, alt: false, ctrl: false };
}

/// 布局上下文：widget 在 `set_rect` 阶段使用。
pub struct LayoutCtx<'a> {
    /// 正文字体测量（等宽，如 Menlo）
    pub measure: &'a mut dyn TextMeasure,
    /// UI 字体测量（proportional，如 -apple-system）。
    /// 为 None 时回退到 `measure`。
    pub ui_measure: Option<&'a mut dyn TextMeasure>,
    pub theme: &'a Theme,
    pub dpi: f32,
}

/// 绘制上下文：widget 在 `paint` 阶段使用。
pub struct PaintCtx<'a> {
    pub list: &'a mut DrawList,
    pub theme: &'a Theme,
    pub dpi: f32,
    /// 容器维护的累计坐标偏移，(dx, dy)。widget 不修改此字段。
    pub offset: (f32, f32),
    /// 全局透明度（0.0–1.0）。默认 1.0；用于后续淡入淡出动画（如 sidebar HoverPeek fade）。
    pub global_alpha: f32,
    /// Harfbuzz shaper for building UiTextLayout in paint.
    /// None in tests; set by the app layer.
    pub shaper: Option<&'a mut Shaper>,
}

impl<'a> PaintCtx<'a> {
    /// Convenience constructor for tests (no shaper).
    pub fn new(list: &'a mut DrawList, theme: &'a Theme, dpi: f32) -> Self {
        Self { list, theme, dpi, offset: (0.0, 0.0), global_alpha: 1.0, shaper: None }
    }

    /// Shape and emit text via harfbuzz. No-op when shaper is None.
    pub fn text(&mut self, x: f32, y_baseline: f32, font_size: f32, color: [f32; 4], s: &str) {
        if let Some(ref mut shaper) = self.shaper {
            self.list.text_shaped(x, y_baseline, font_size, color, s, shaper);
        }
    }

    /// Shape and emit text with explicit font family, weight, and style.
    /// No-op when shaper is None.
    pub fn text_with_font(
        &mut self,
        x: f32,
        y_baseline: f32,
        font_size: f32,
        color: [f32; 4],
        s: &str,
        font_family: Option<String>,
        font_weight: shaping::Weight,
        font_style: shaping::Style,
    ) {
        if let Some(ref mut shaper) = self.shaper {
            self.list.text_shaped_with_font(
                x,
                y_baseline,
                font_size,
                color,
                s,
                font_family,
                font_weight,
                font_style,
                false,
                shaper,
            );
        }
    }
}

/// 事件上下文：widget 在 `on_event` 阶段使用。
pub struct EventCtx<'a> {
    pub theme: &'a Theme,
    pub dpi: f32,
    /// Widget 期望的光标图标。
    ///
    /// 契约：如果 widget 在 `on_event` 中消费了 MouseMove 事件（返回非 None），
    /// 则必须设置此字段以告知外层 handler 正确的光标。否则光标将停留在上一个值。
    /// 事件消费但不设 cursor_hint 会触发安全兜底（回退到 Default 箭头光标）。
    pub cursor_hint: Option<winit::window::CursorIcon>,
}

/// 输入事件
#[derive(Clone, PartialEq)]
pub enum Event {
    MouseMove { px: f32, py: f32 },
    PointerLeave,
    MouseDown { px: f32, py: f32, button: MouseButton },
    MouseUp { px: f32, py: f32, button: MouseButton },
    InteractionCancel,
    Wheel { dx: f32, dy: f32, px: f32, py: f32 },
    KeyDown(KeyCode, Modifiers),
    ImePreedit { text: String, cursor: Option<(usize, usize)> },
    ImeCommit(String),
    ImeEnable,
    ImeDisable,
}

impl std::fmt::Debug for Event {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MouseMove { px, py } => {
                formatter.debug_struct("MouseMove").field("px", px).field("py", py).finish()
            }
            Self::PointerLeave => formatter.write_str("PointerLeave"),
            Self::MouseDown { px, py, button } => formatter
                .debug_struct("MouseDown")
                .field("px", px)
                .field("py", py)
                .field("button", button)
                .finish(),
            Self::MouseUp { px, py, button } => formatter
                .debug_struct("MouseUp")
                .field("px", px)
                .field("py", py)
                .field("button", button)
                .finish(),
            Self::InteractionCancel => formatter.write_str("InteractionCancel"),
            Self::Wheel { dx, dy, px, py } => formatter
                .debug_struct("Wheel")
                .field("dx", dx)
                .field("dy", dy)
                .field("px", px)
                .field("py", py)
                .finish(),
            Self::KeyDown(key_code, modifiers) => {
                formatter.debug_tuple("KeyDown").field(key_code).field(modifiers).finish()
            }
            Self::ImePreedit { cursor, .. } => formatter
                .debug_struct("ImePreedit")
                .field("text", &"<redacted>")
                .field("cursor", cursor)
                .finish(),
            Self::ImeCommit(_) => formatter.write_str("ImeCommit(<redacted>)"),
            Self::ImeEnable => formatter.write_str("ImeEnable"),
            Self::ImeDisable => formatter.write_str("ImeDisable"),
        }
    }
}

impl zeroize::Zeroize for Event {
    fn zeroize(&mut self) {
        match self {
            Self::ImePreedit { text, .. } | Self::ImeCommit(text) => {
                zeroize::Zeroize::zeroize(text);
            }
            Self::MouseMove { .. }
            | Self::PointerLeave
            | Self::MouseDown { .. }
            | Self::MouseUp { .. }
            | Self::InteractionCancel
            | Self::Wheel { .. }
            | Self::KeyDown(..)
            | Self::ImeEnable
            | Self::ImeDisable => {}
        }
    }
}

impl zeroize::ZeroizeOnDrop for Event {}

impl Drop for Event {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(self);
    }
}

pub struct SensitiveText(zeroize::Zeroizing<String>);

impl SensitiveText {
    pub fn new(value: String) -> Self {
        Self(zeroize::Zeroizing::new(value))
    }

    pub fn expose(&self) -> &str {
        self.0.as_str()
    }

    pub(crate) fn clear(&mut self) {
        zeroize::Zeroize::zeroize(self);
    }

    pub(crate) fn replace_range(&mut self, range: std::ops::Range<usize>, replacement: &str) {
        let updated = {
            let current = self.expose();
            let prefix = &current[..range.start];
            let suffix = &current[range.end..];
            let required_capacity = prefix.len() + replacement.len() + suffix.len();
            let mut updated = zeroize::Zeroizing::new(String::with_capacity(required_capacity));
            updated.push_str(prefix);
            updated.push_str(replacement);
            updated.push_str(suffix);
            updated
        };

        zeroize::Zeroize::zeroize(&mut self.0);
        self.0 = updated;
    }
}

impl zeroize::Zeroize for SensitiveText {
    fn zeroize(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.0);
    }
}

impl zeroize::ZeroizeOnDrop for SensitiveText {}

impl Clone for SensitiveText {
    fn clone(&self) -> Self {
        Self::new(self.expose().to_owned())
    }
}

impl PartialEq for SensitiveText {
    fn eq(&self, other: &Self) -> bool {
        self.expose() == other.expose()
    }
}

impl std::fmt::Debug for SensitiveText {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SensitiveText(<redacted>)")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TextPayload {
    Plain(String),
    Sensitive(SensitiveText),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ControlAction {
    Activated { id: WidgetId },
    Toggled { id: WidgetId, checked: bool },
    TextEdited { id: WidgetId, value: TextPayload },
    TextCommitted { id: WidgetId, value: TextPayload },
    FocusRequested { id: WidgetId },
}

impl ControlAction {
    pub fn id(&self) -> WidgetId {
        match self {
            Self::Activated { id }
            | Self::Toggled { id, .. }
            | Self::TextEdited { id, .. }
            | Self::TextCommitted { id, .. }
            | Self::FocusRequested { id } => *id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MindmapStylePanelAction {
    Close,
    TogglePresets,
    SelectTheme(String),
}

/// 统一的 Widget Action 类型。
/// 替换 `Box<dyn Any>` downcast，提供编译期穷尽检查。
#[derive(Debug, Clone, PartialEq)]
pub enum WidgetAction {
    Control(ControlAction),
    Overlay(OverlayAction),
    Settings(SettingsViewAction),
    Sidebar(crate::widgets::sidebar::SidebarAction),
    TabBar(crate::tab_bar::TabBarAction),
    Scrollbar(crate::widgets::scrollbar::ScrollbarAction),
    CanvasScrollbars(crate::widgets::canvas_scrollbars::CanvasScrollbarsAction),
    SearchBar(crate::widgets::search_bar::SearchBarAction),
    Popup(crate::popup_menu::PopupOutcome),
    List(crate::widgets::list::ListAction),
    TitleBar(crate::widgets::title_bar::TitleBarAction),
    Toc(crate::widgets::toc::TocAction),
    TreeList(crate::widgets::tree_list::TreeListAction),
    VirtualCardList(crate::widgets::virtual_card_list::VirtualCardListAction),
    Splitter(crate::widgets::splitter::SplitterAction),
    MindmapStylePanel(MindmapStylePanelAction),
    /// 事件已消费但无需 AppAction（如 hover 更新）
    Consumed,
}

/// UI 组件 trait。
/// 每个 widget 持有自己的 `rect`，通过 `set_rect` 获得布局结果。
pub trait Widget: Any {
    /// 设置 widget 的像素矩形。dock 在 layout 阶段调用。
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx);

    /// 生成绘制命令列表。
    fn paint(&self, ctx: &mut PaintCtx);

    /// 点击测试：给定像素坐标，是否命中本 widget。
    fn hit(&self, px: f32, py: f32) -> bool;

    /// 返回本 widget 的 WidgetId。默认 None；只有需键盘路由的 widget override。
    fn id(&self) -> Option<WidgetId> {
        None
    }

    fn is_focusable(&self) -> bool {
        false
    }

    fn collect_focusable_ids(&self, output: &mut Vec<WidgetId>) {
        if self.is_focusable()
            && let Some(id) = self.id()
        {
            output.push(id);
        }
    }

    fn set_keyboard_focus(&mut self, _focused_id: Option<WidgetId>) {}

    /// 返回以屏幕物理像素描述的语义子树。默认 widget 不产生节点。
    fn accessibility_node(&self, _ctx: &AccessibilityContext) -> Option<AccessibilityNode> {
        None
    }

    /// 收集当前 widget 暴露的语义子树；容器可覆盖此方法递归收集视觉子节点。
    fn collect_accessibility_nodes(
        &self,
        ctx: &AccessibilityContext,
        output: &mut Vec<AccessibilityNode>,
    ) {
        if let Some(node) = self.accessibility_node(ctx) {
            output.push(node);
        }
    }

    /// 处理辅助技术动作，并复用现有的 typed `WidgetAction` 业务通道。
    fn on_accessibility_action(
        &mut self,
        _request: &AccessibilityActionRequest,
    ) -> Option<WidgetAction> {
        None
    }

    /// 处理输入事件，返回可选的 action（上行给 app 层）。
    fn on_event(&mut self, _ev: &Event, _ctx: &mut EventCtx) -> Option<WidgetAction> {
        None
    }

    /// 鼠标捕获：当 widget 正在拖拽（thumb / resize / 等等）时返回 true。
    /// dock 在 dispatch 时会优先把所有鼠标事件派给 capturing widget，
    /// 跳过 hit test，从而保证拖拽中光标移出 widget 矩形也能继续接收事件。
    fn is_capturing(&self) -> bool {
        false
    }

    /// Return a tooltip hint if (px, py) in widget-local coordinates
    /// hovers over a sub-region that should show a tooltip.
    fn tooltip_at(&self, _px: f32, _py: f32) -> Option<TooltipHint> {
        None
    }

    /// 支持 downcast 到具体 widget 类型。默认返回空引用。
    fn as_any(&self) -> &dyn std::any::Any {
        &()
    }

    /// 支持可变 downcast 到具体 widget 类型。
    /// 每个具体 widget 必须 override。
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::measure::NoopMeasure;
    use crate::core::paint::DrawCmd;
    use std::collections::HashMap;

    /// 最小测试 widget：记录 set_rect 参数，paint 输出固定 fill。
    struct TestWidget {
        pub rect: Rect,
    }

    impl TestWidget {
        fn new() -> Self {
            Self { rect: Rect::ZERO }
        }
    }

    impl Widget for TestWidget {
        fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
            self.rect = rect;
        }

        fn paint(&self, ctx: &mut PaintCtx) {
            ctx.list.fill(self.rect, [1.0, 0.0, 0.0, 1.0]);
        }

        fn hit(&self, px: f32, py: f32) -> bool {
            self.rect.contains(px, py)
        }

        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    fn dummy_theme() -> Theme {
        crate::theme::test_theme()
    }

    #[test]
    fn mindmap_style_panel_select_theme_action_keeps_theme_id() {
        let action =
            WidgetAction::MindmapStylePanel(MindmapStylePanelAction::SelectTheme("tide".into()));

        assert!(matches!(
            action,
            WidgetAction::MindmapStylePanel(MindmapStylePanelAction::SelectTheme(id)) if id == "tide"
        ));
    }

    #[test]
    fn widget_set_rect_stores_rect() {
        let mut w = TestWidget::new();
        let theme = dummy_theme();
        let mut measure = NoopMeasure;
        let mut ctx =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        w.set_rect(Rect::new(10.0, 20.0, 100.0, 50.0), &mut ctx);
        assert_eq!(w.rect, Rect::new(10.0, 20.0, 100.0, 50.0));
    }

    #[test]
    fn widget_paint_emits_commands() {
        let w = TestWidget { rect: Rect::new(0.0, 0.0, 100.0, 100.0) };
        let theme = dummy_theme();
        let mut dl = DrawList::new();
        let mut ctx = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: None,
        };
        w.paint(&mut ctx);
        assert_eq!(dl.cmds.len(), 1);
        assert!(matches!(dl.cmds[0], DrawCmd::FillRect { .. }));
    }

    #[test]
    fn widget_hit_delegates_to_rect_contains() {
        let w = TestWidget { rect: Rect::new(10.0, 10.0, 80.0, 80.0) };
        assert!(w.hit(10.0, 10.0));
        assert!(w.hit(89.99, 89.99));
        assert!(!w.hit(90.0, 10.0));
        assert!(!w.hit(10.0, 90.0));
    }

    #[test]
    fn default_on_event_returns_none() {
        let mut w = TestWidget::new();
        let theme = dummy_theme();
        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        let ev = Event::MouseMove { px: 0.0, py: 0.0 };
        assert!(w.on_event(&ev, &mut ctx).is_none());
    }

    #[test]
    fn default_accessibility_contract_is_empty_and_inert() {
        let mut widget = TestWidget::new();
        let accessibility_context = AccessibilityContext::default();
        let mut nodes = Vec::new();
        widget.collect_accessibility_nodes(&accessibility_context, &mut nodes);

        assert!(nodes.is_empty());
        assert_eq!(
            widget.on_accessibility_action(&AccessibilityActionRequest::new(
                crate::core::AccessibilityId(1),
                crate::core::AccessibilityAction::Activate,
            )),
            None
        );
    }

    #[test]
    fn lifecycle_events_have_stable_debug_names() {
        assert_eq!(format!("{:?}", Event::PointerLeave), "PointerLeave");
        assert_eq!(format!("{:?}", Event::InteractionCancel), "InteractionCancel");
    }

    #[test]
    fn as_any_mut_works() {
        use std::any::Any;
        struct MyWidget {
            val: u32,
        }
        impl MyWidget {
            fn new() -> Self {
                Self { val: 42 }
            }
        }
        impl Widget for MyWidget {
            fn set_rect(&mut self, _: Rect, _: &mut LayoutCtx) {}
            fn paint(&self, _: &mut PaintCtx) {}
            fn hit(&self, _: f32, _: f32) -> bool {
                false
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
            fn as_any_mut(&mut self) -> &mut dyn Any {
                self
            }
        }
        let mut w = MyWidget::new();
        {
            let r: &mut dyn Widget = &mut w;
            let down: &mut MyWidget = r.as_any_mut().downcast_mut::<MyWidget>().unwrap();
            down.val = 99;
        }
        assert_eq!(w.val, 99);
    }

    #[test]
    fn widget_id_copy_eq_hash() {
        let a = WidgetId(1);
        let b = WidgetId(1);
        assert_eq!(a, b);
        let mut m = HashMap::new();
        m.insert(a, ());
        // Copy
        let c = a;
        assert_eq!(c, a);
    }

    #[test]
    fn widget_id_default_none() {
        let mut w = TestWidget::new();
        let theme = dummy_theme();
        let mut measure = NoopMeasure;
        let mut ctx =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 100.0, 100.0), &mut ctx);
        assert_eq!(w.id(), None);
    }

    #[test]
    fn sensitive_text_debug_is_redacted() {
        let secret = SensitiveText::new("never-print-me".into());
        assert_eq!(format!("{secret:?}"), "SensitiveText(<redacted>)");
        assert!(!format!("{:?}", TextPayload::Sensitive(secret)).contains("never-print-me"));
    }

    #[test]
    fn ime_event_debug_never_exposes_text() {
        fn assert_zeroize_on_drop<T: zeroize::ZeroizeOnDrop>() {}

        const SECRET: &str = "ime-api-key-never-print";
        let preedit = Event::ImePreedit { text: SECRET.to_owned(), cursor: Some((0, 4)) };
        let commit = Event::ImeCommit(SECRET.to_owned());

        assert_zeroize_on_drop::<Event>();
        assert!(!format!("{preedit:?}").contains(SECRET));
        assert!(!format!("{commit:?}").contains(SECRET));
    }

    #[test]
    fn sensitive_text_supports_explicit_zeroizing_clear_and_range_replacement() {
        fn assert_zeroize_on_drop<T: zeroize::ZeroizeOnDrop>() {}

        assert_zeroize_on_drop::<SensitiveText>();

        let mut secret = SensitiveText::new("prefix-secret-suffix".into());
        secret.replace_range(7..13, "updated");
        assert_eq!(secret.expose(), "prefix-updated-suffix");

        secret.clear();
        assert_eq!(secret.expose(), "");
    }

    #[test]
    fn control_action_preserves_widget_identity() {
        let id = WidgetId(42);
        assert_eq!(ControlAction::Toggled { id, checked: true }.id(), id);
    }
}
