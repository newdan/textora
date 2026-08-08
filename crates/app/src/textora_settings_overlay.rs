use std::any::Any;

use ui::button::{Button, ButtonStyle};
use ui::core::widget::{ControlAction, WidgetId};
use ui::core::{
    AccessibilityActionRequest, AccessibilityContext, AccessibilityNode, Event, EventCtx,
    LayoutCtx, PaintCtx, Rect, Widget, WidgetAction,
};
use ui::settings_view::{SettingsView, SettingsViewInput};
use ui::theme::SettingsTheme;

use crate::sync_settings_page::SyncSettingsPage;
use crate::sync_settings_types::{SyncSettingsAction, SyncSettingsInput};

const SETTINGS_SIDEBAR_WIDTH_LOGICAL: f32 = 160.0;
const SETTINGS_COMPACT_SIDEBAR_WIDTH_LOGICAL: f32 = 96.0;
const SETTINGS_COMPACT_LAYOUT_THRESHOLD_LOGICAL: f32 = 400.0;
const SETTINGS_SIDEBAR_TOP_INSET_LOGICAL: f32 = 12.0;
const SETTINGS_FORM_INSET_LOGICAL: f32 = 24.0;
const SETTINGS_COMPACT_FORM_INSET_LOGICAL: f32 = 12.0;
const SETTINGS_CATEGORY_HORIZONTAL_INSET_LOGICAL: f32 = 10.0;
const SETTINGS_CATEGORY_BUTTON_HEIGHT_LOGICAL: f32 = 34.0;
const SETTINGS_CATEGORY_BUTTON_GAP_LOGICAL: f32 = 4.0;
const SETTINGS_FORM_GAP_LOGICAL: f32 = 16.0;
const SETTINGS_COMPACT_FORM_GAP_LOGICAL: f32 = 8.0;
const SETTINGS_BUTTON_FONT_SIZE_LOGICAL: f32 = 14.0;
const SETTINGS_BUTTON_PADDING_LOGICAL: f32 = 12.0;
const SETTINGS_BUTTON_RADIUS_LOGICAL: f32 = 8.0;
const SETTINGS_SIDEBAR_SEPARATOR_WIDTH_LOGICAL: f32 = 1.0;
const SETTINGS_TRANSPARENT: [f32; 4] = [0.0, 0.0, 0.0, 0.0];
const SETTINGS_CATEGORY_HOVER_ACCENT_BLEND: f32 = 0.05;
const SETTINGS_CATEGORY_PRESSED_ACCENT_BLEND: f32 = 0.09;
const SETTINGS_CATEGORY_SELECTED_ACCENT_BLEND: f32 = 0.14;
const SETTINGS_DISABLED_FOREGROUND_ALPHA: f32 = 0.45;

const APPEARANCE_CATEGORY_ID: WidgetId = WidgetId(0x7365_7474_6170_7065);
const EDITOR_CATEGORY_ID: WidgetId = WidgetId(0x7365_7474_6564_6974);
const INTERFACE_CATEGORY_ID: WidgetId = WidgetId(0x7365_7474_696e_7466);
const SYNC_CATEGORY_ID: WidgetId = WidgetId(0x7365_7474_7379_6e63);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingsHoverTarget {
    Category(usize),
    Content,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProductSettingsCategory {
    Appearance,
    Editor,
    Interface,
    Sync,
}

pub(crate) struct TextoraSettingsOverlay {
    rect: Rect,
    sidebar_width: f32,
    active_category: ProductSettingsCategory,
    category_buttons: Vec<(ProductSettingsCategory, Button)>,
    category_rects: Vec<Rect>,
    category_pointer_index: Option<usize>,
    hover_target: Option<SettingsHoverTarget>,
    settings_view: SettingsView,
    sync_page: SyncSettingsPage,
    generic_page_rect: Rect,
    sync_page_rect: Rect,
    settings_theme: SettingsTheme,
}

impl TextoraSettingsOverlay {
    pub(crate) fn new(settings_input: SettingsViewInput, sync_input: SyncSettingsInput) -> Self {
        let active_category = ProductSettingsCategory::Appearance;
        let settings_theme = fallback_settings_theme();
        let mut settings_view = SettingsView::new(settings_input);
        settings_view.set_category_navigation_visible(false);
        let mut overlay = Self {
            rect: Rect::ZERO,
            sidebar_width: 0.0,
            active_category,
            category_buttons: Vec::new(),
            category_rects: Vec::new(),
            category_pointer_index: None,
            hover_target: None,
            settings_view,
            sync_page: SyncSettingsPage::new(sync_input),
            generic_page_rect: Rect::ZERO,
            sync_page_rect: Rect::ZERO,
            settings_theme,
        };
        overlay.category_buttons = overlay.build_category_buttons();
        overlay
    }

    pub(crate) fn set_settings_input(&mut self, input: SettingsViewInput) {
        self.settings_view.set_input(input);
    }

    pub(crate) fn set_sync_input(&mut self, input: SyncSettingsInput) {
        self.sync_page.set_input(input);
    }

    pub(crate) fn take_pending_sync_action(&mut self) -> Option<SyncSettingsAction> {
        self.sync_page.take_pending_action()
    }

    fn build_category_buttons(&self) -> Vec<(ProductSettingsCategory, Button)> {
        [
            (ProductSettingsCategory::Appearance, "外观", APPEARANCE_CATEGORY_ID),
            (ProductSettingsCategory::Editor, "编辑器", EDITOR_CATEGORY_ID),
            (ProductSettingsCategory::Interface, "界面", INTERFACE_CATEGORY_ID),
            (ProductSettingsCategory::Sync, "同步", SYNC_CATEGORY_ID),
        ]
        .into_iter()
        .map(|(category, title, id)| {
            let mut button = Button::new(id, category_button_style(self.settings_theme));
            button.set_text(Some(title.to_owned()));
            button.set_selected(category == self.active_category);
            (category, button)
        })
        .collect()
    }

    fn category_index_at(&self, px: f32, py: f32) -> Option<usize> {
        self.category_rects.iter().position(|rect| rect.contains(px, py))
    }

    fn activate_category(&mut self, category: ProductSettingsCategory) {
        self.active_category = category;
        for (candidate, button) in &mut self.category_buttons {
            button.set_selected(*candidate == category);
        }
        match category {
            ProductSettingsCategory::Appearance => self
                .settings_view
                .set_active_category(ui::settings_view::SettingsCategory::Appearance),
            ProductSettingsCategory::Editor => {
                self.settings_view.set_active_category(ui::settings_view::SettingsCategory::Editor)
            }
            ProductSettingsCategory::Interface => self
                .settings_view
                .set_active_category(ui::settings_view::SettingsCategory::Interface),
            ProductSettingsCategory::Sync => self.sync_page.reset_scroll_and_focus_endpoint(),
        }
    }

    fn dispatch_category_event(
        &mut self,
        index: usize,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        let rect = *self.category_rects.get(index)?;
        let local_event = ui::core::dock::Dock::to_local(event, rect.x, rect.y);
        let action = self.category_buttons[index].1.on_event(local_event.as_ref(), ctx)?;
        match action {
            WidgetAction::Control(ControlAction::Activated { .. }) => {
                let category = self.category_buttons[index].0;
                self.activate_category(category);
                Some(WidgetAction::Consumed)
            }
            WidgetAction::Control(_) => Some(WidgetAction::Consumed),
            other => Some(other),
        }
    }

    fn dispatch_active_page_event(
        &mut self,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        if self.active_category == ProductSettingsCategory::Sync {
            let local_event =
                ui::core::dock::Dock::to_local(event, self.sync_page_rect.x, self.sync_page_rect.y);
            return self.sync_page.on_event(local_event.as_ref(), ctx);
        }
        let local_event = ui::core::dock::Dock::to_local(
            event,
            self.generic_page_rect.x,
            self.generic_page_rect.y,
        );
        self.settings_view.on_event(local_event.as_ref(), ctx)
    }

    fn active_page_is_capturing(&self) -> bool {
        if self.active_category == ProductSettingsCategory::Sync {
            return self.sync_page.is_capturing();
        }
        self.settings_view.is_capturing()
    }

    fn hover_target_at(&self, px: f32, py: f32) -> Option<SettingsHoverTarget> {
        if let Some(index) = self.category_index_at(px, py) {
            return Some(SettingsHoverTarget::Category(index));
        }
        self.active_page_rect().contains(px, py).then_some(SettingsHoverTarget::Content)
    }

    fn active_page_rect(&self) -> Rect {
        if self.active_category == ProductSettingsCategory::Sync {
            return self.sync_page_rect;
        }
        self.generic_page_rect
    }

    fn dispatch_hover_target(
        &mut self,
        target: SettingsHoverTarget,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        match target {
            SettingsHoverTarget::Category(index) => self.dispatch_category_event(index, event, ctx),
            SettingsHoverTarget::Content => self.dispatch_active_page_event(event, ctx),
        }
    }

    fn dispatch_mouse_move(
        &mut self,
        px: f32,
        py: f32,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        let next_hover_target = self.hover_target_at(px, py);
        let previous_hover_action = if self.hover_target != next_hover_target {
            self.hover_target.and_then(|target| {
                let saved_cursor_hint = ctx.cursor_hint;
                let action = self.dispatch_hover_target(target, event, ctx);
                ctx.cursor_hint = saved_cursor_hint;
                action
            })
        } else {
            None
        };
        self.hover_target = next_hover_target;
        next_hover_target
            .and_then(|target| self.dispatch_hover_target(target, event, ctx))
            .or(previous_hover_action)
    }

    fn dispatch_interaction_lifecycle(
        &mut self,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        let container_changed = if matches!(event, Event::InteractionCancel) {
            self.category_pointer_index.take().is_some() | self.hover_target.take().is_some()
        } else {
            self.hover_target.take().is_some()
        };
        let mut first_action = None;
        for category_index in 0..self.category_buttons.len() {
            if let Some(action) = self.dispatch_category_event(category_index, event, ctx)
                && first_action.is_none()
            {
                first_action = Some(action);
            }
        }
        if let Some(action) = self.dispatch_active_page_event(event, ctx)
            && first_action.is_none()
        {
            first_action = Some(action);
        }
        first_action.or_else(|| container_changed.then_some(WidgetAction::Consumed))
    }

    fn layout_category_buttons(&mut self, ctx: &mut LayoutCtx) {
        self.category_rects.clear();
        let mut category_y = SETTINGS_SIDEBAR_TOP_INSET_LOGICAL * ctx.dpi;
        for (_, button) in &mut self.category_buttons {
            button.set_style(category_button_style(self.settings_theme));
            let category_rect = Rect::new(
                SETTINGS_CATEGORY_HORIZONTAL_INSET_LOGICAL * ctx.dpi,
                category_y,
                (self.sidebar_width - 2.0 * SETTINGS_CATEGORY_HORIZONTAL_INSET_LOGICAL * ctx.dpi)
                    .max(0.0),
                SETTINGS_CATEGORY_BUTTON_HEIGHT_LOGICAL * ctx.dpi,
            );
            self.category_rects.push(category_rect);
            button.set_rect(Rect::new(0.0, 0.0, category_rect.w, category_rect.h), ctx);
            category_y += category_rect.h + SETTINGS_CATEGORY_BUTTON_GAP_LOGICAL * ctx.dpi;
        }
    }

    fn layout_pages(&mut self, compact_layout: bool, ctx: &mut LayoutCtx) {
        let form_gap_logical = if compact_layout {
            SETTINGS_COMPACT_FORM_GAP_LOGICAL
        } else {
            SETTINGS_FORM_GAP_LOGICAL
        };
        let form_gap = form_gap_logical * ctx.dpi;
        let page_rect = Rect::new(
            self.sidebar_width + form_gap,
            0.0,
            (self.rect.w - self.sidebar_width - form_gap).max(0.0),
            self.rect.h,
        );
        let form_inset_logical = if compact_layout {
            SETTINGS_COMPACT_FORM_INSET_LOGICAL
        } else {
            SETTINGS_FORM_INSET_LOGICAL
        };
        let form_inset = form_inset_logical * ctx.dpi;
        let internal_settings_inset =
            if page_rect.w < SETTINGS_COMPACT_LAYOUT_THRESHOLD_LOGICAL * ctx.dpi {
                SETTINGS_COMPACT_FORM_INSET_LOGICAL * ctx.dpi
            } else {
                SETTINGS_FORM_INSET_LOGICAL * ctx.dpi
            };
        let settings_wrapper_inset = (form_inset - internal_settings_inset).max(0.0);
        self.generic_page_rect = inset_rect(page_rect, settings_wrapper_inset);
        self.settings_view
            .set_rect(Rect::new(0.0, 0.0, self.generic_page_rect.w, self.generic_page_rect.h), ctx);

        self.sync_page_rect = inset_rect(page_rect, form_inset);
        self.sync_page
            .set_rect(Rect::new(0.0, 0.0, self.sync_page_rect.w, self.sync_page_rect.h), ctx);
    }
}

impl Widget for TextoraSettingsOverlay {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w.max(0.0), rect.h.max(0.0));
        self.settings_theme = ctx.theme.settings_theme();
        let compact_layout = self.rect.w < SETTINGS_COMPACT_LAYOUT_THRESHOLD_LOGICAL * ctx.dpi;
        let sidebar_width_logical = if compact_layout {
            SETTINGS_COMPACT_SIDEBAR_WIDTH_LOGICAL
        } else {
            SETTINGS_SIDEBAR_WIDTH_LOGICAL
        };
        self.sidebar_width = (sidebar_width_logical * ctx.dpi).min(self.rect.w);
        self.layout_category_buttons(ctx);
        self.layout_pages(compact_layout, ctx);
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        ctx.list.fill(
            Rect::new(0.0, 0.0, self.sidebar_width, self.rect.h),
            self.settings_theme.sidebar_surface,
        );
        let separator_width = SETTINGS_SIDEBAR_SEPARATOR_WIDTH_LOGICAL * ctx.dpi;
        ctx.list.fill(
            Rect::new(
                (self.sidebar_width - separator_width).max(0.0),
                0.0,
                separator_width,
                self.rect.h,
            ),
            self.settings_theme.separator,
        );
        for ((_, button), rect) in self.category_buttons.iter().zip(&self.category_rects) {
            let saved_offset = ctx.list.offset;
            ctx.list.offset = (saved_offset.0 + rect.x, saved_offset.1 + rect.y);
            button.paint(ctx);
            ctx.list.offset = saved_offset;
        }

        let page_rect = self.active_page_rect();
        let saved_offset = ctx.list.offset;
        ctx.list.offset = (saved_offset.0 + page_rect.x, saved_offset.1 + page_rect.y);
        if self.active_category == ProductSettingsCategory::Sync {
            self.sync_page.paint(ctx);
        } else {
            self.settings_view.paint(ctx);
        }
        ctx.list.offset = saved_offset;
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn collect_focusable_ids(&self, output: &mut Vec<WidgetId>) {
        for (_, button) in &self.category_buttons {
            button.collect_focusable_ids(output);
        }
        if self.active_category == ProductSettingsCategory::Sync {
            self.sync_page.collect_focusable_ids(output);
        } else {
            self.settings_view.collect_focusable_ids(output);
        }
    }

    fn set_keyboard_focus(&mut self, focused_id: Option<WidgetId>) {
        for (_, button) in &mut self.category_buttons {
            button.set_keyboard_focus(focused_id);
        }
        if self.active_category == ProductSettingsCategory::Sync {
            self.sync_page.set_keyboard_focus(focused_id);
        } else {
            self.settings_view.set_keyboard_focus(focused_id);
        }
    }

    fn collect_accessibility_nodes(
        &self,
        context: &AccessibilityContext,
        output: &mut Vec<AccessibilityNode>,
    ) {
        for ((_, button), rect) in self.category_buttons.iter().zip(&self.category_rects) {
            if rect.w > 0.0 && rect.h > 0.0 {
                button.collect_accessibility_nodes(&context.offset_by(rect.x, rect.y), output);
            }
        }

        let page_rect = self.active_page_rect();
        let page_context = context.offset_by(page_rect.x, page_rect.y);
        if self.active_category == ProductSettingsCategory::Sync {
            self.sync_page.collect_accessibility_nodes(&page_context, output);
        } else {
            self.settings_view.collect_accessibility_nodes(&page_context, output);
        }
    }

    fn on_accessibility_action(
        &mut self,
        request: &AccessibilityActionRequest,
    ) -> Option<WidgetAction> {
        for index in 0..self.category_buttons.len() {
            let Some(action) = self.category_buttons[index].1.on_accessibility_action(request)
            else {
                continue;
            };
            return match action {
                WidgetAction::Control(ControlAction::Activated { .. }) => {
                    let category = self.category_buttons[index].0;
                    self.activate_category(category);
                    Some(WidgetAction::Consumed)
                }
                WidgetAction::Control(ControlAction::FocusRequested { id }) => {
                    self.set_keyboard_focus(Some(id));
                    Some(WidgetAction::Control(ControlAction::FocusRequested { id }))
                }
                WidgetAction::Control(_) => Some(WidgetAction::Consumed),
                other => Some(other),
            };
        }

        if self.active_category == ProductSettingsCategory::Sync {
            self.sync_page.on_accessibility_action(request)
        } else {
            self.settings_view.on_accessibility_action(request)
        }
    }

    fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        if matches!(event, Event::PointerLeave | Event::InteractionCancel) {
            return self.dispatch_interaction_lifecycle(event, ctx);
        }

        if self.active_page_is_capturing()
            && matches!(event, Event::MouseMove { .. } | Event::MouseUp { .. })
        {
            return self.dispatch_active_page_event(event, ctx);
        }
        if let Some(index) = self.category_pointer_index
            && matches!(event, Event::MouseMove { .. } | Event::MouseUp { .. })
        {
            let action = self.dispatch_category_event(index, event, ctx);
            if matches!(event, Event::MouseUp { .. }) {
                self.category_pointer_index = None;
            }
            return action;
        }

        match event {
            Event::MouseDown { px, py, .. } => {
                if let Some(index) = self.category_index_at(*px, *py) {
                    self.category_pointer_index = Some(index);
                    return self.dispatch_category_event(index, event, ctx);
                }
                self.dispatch_active_page_event(event, ctx)
            }
            Event::MouseMove { px, py } => self.dispatch_mouse_move(*px, *py, event, ctx),
            _ => self.dispatch_active_page_event(event, ctx),
        }
    }

    fn is_capturing(&self) -> bool {
        self.category_pointer_index.is_some() || self.active_page_is_capturing()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn category_button_style(settings: SettingsTheme) -> ButtonStyle {
    ButtonStyle {
        font_size_logical: SETTINGS_BUTTON_FONT_SIZE_LOGICAL,
        pad_x_logical: SETTINGS_BUTTON_PADDING_LOGICAL,
        foreground: settings.text_primary,
        selected_foreground: settings.accent,
        background: SETTINGS_TRANSPARENT,
        border: SETTINGS_TRANSPARENT,
        hover_background: blend_color(
            settings.sidebar_surface,
            settings.accent,
            SETTINGS_CATEGORY_HOVER_ACCENT_BLEND,
        ),
        pressed_background: blend_color(
            settings.sidebar_surface,
            settings.accent,
            SETTINGS_CATEGORY_PRESSED_ACCENT_BLEND,
        ),
        selected_background: blend_color(
            settings.sidebar_surface,
            settings.accent,
            SETTINGS_CATEGORY_SELECTED_ACCENT_BLEND,
        ),
        disabled_foreground: with_alpha(settings.text_primary, SETTINGS_DISABLED_FOREGROUND_ALPHA),
        disabled_background: SETTINGS_TRANSPARENT,
        corner_radius_logical: SETTINGS_BUTTON_RADIUS_LOGICAL,
    }
}

fn blend_color(base: [f32; 4], accent: [f32; 4], accent_factor: f32) -> [f32; 4] {
    let base_factor = 1.0 - accent_factor;
    [
        base[0] * base_factor + accent[0] * accent_factor,
        base[1] * base_factor + accent[1] * accent_factor,
        base[2] * base_factor + accent[2] * accent_factor,
        base[3] * base_factor + accent[3] * accent_factor,
    ]
}

fn with_alpha(mut color: [f32; 4], alpha: f32) -> [f32; 4] {
    color[3] *= alpha;
    color
}

fn inset_rect(rect: Rect, inset: f32) -> Rect {
    Rect::new(
        rect.x + inset,
        rect.y + inset,
        (rect.w - 2.0 * inset).max(0.0),
        (rect.h - 2.0 * inset).max(0.0),
    )
}

fn fallback_settings_theme() -> SettingsTheme {
    ui::theme::test_theme().settings_theme()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync_settings_types::{SyncSettingsAction, SyncSettingsInput};
    use ui::ThemeMode;
    use ui::core::measure::NoopMeasure;
    use ui::core::paint::{DrawCmd, DrawList};
    use ui::core::widget::MouseButton;
    use ui::core::{Event, EventCtx, LayoutCtx, PaintCtx, Rect, Widget, WidgetAction};
    use ui::settings_view::{SettingsPersistenceView, SettingsViewAction, SettingsViewInput};
    use ui::view_mode::ViewMode;

    #[test]
    fn defaults_to_appearance_with_the_legacy_category_identity() {
        let overlay = TextoraSettingsOverlay::new(
            settings_input(SettingsPersistenceView::Saved),
            SyncSettingsInput::default(),
        );

        assert_eq!(overlay.active_category, ProductSettingsCategory::Appearance);
        assert_eq!(
            overlay
                .category_buttons
                .iter()
                .map(|(category, button)| (*category, button.id()))
                .collect::<Vec<_>>(),
            vec![
                (ProductSettingsCategory::Appearance, Some(APPEARANCE_CATEGORY_ID)),
                (ProductSettingsCategory::Editor, Some(EDITOR_CATEGORY_ID)),
                (ProductSettingsCategory::Interface, Some(INTERFACE_CATEGORY_ID)),
                (ProductSettingsCategory::Sync, Some(SYNC_CATEGORY_ID)),
            ],
        );
    }

    #[test]
    fn sync_page_action_is_consumed_then_taken_once_as_a_product_action() {
        let mut overlay = laid_out_overlay(SettingsPersistenceView::Saved);
        click_category(&mut overlay, 3);
        assert_eq!(overlay.active_category, ProductSettingsCategory::Sync);
        let draw_list = paint_overlay(&overlay);
        let configure_button_rect = button_rect_for_text(&draw_list, "保存连接");

        assert_eq!(
            click_at(
                &mut overlay,
                configure_button_rect.x + configure_button_rect.w * 0.5,
                configure_button_rect.y + configure_button_rect.h * 0.5,
            ),
            Some(WidgetAction::Consumed),
        );
        assert!(matches!(
            overlay.take_pending_sync_action(),
            Some(SyncSettingsAction::ConfigureConnection { .. }),
        ));
        assert_eq!(overlay.take_pending_sync_action(), None);
    }

    #[test]
    fn outer_compact_breakpoint_keeps_generic_and_sync_content_geometry_aligned() {
        for (width, expected_content_rect) in [
            (399.0, Rect::new(116.0, 12.0, 271.0, 456.0)),
            (400.0, Rect::new(200.0, 24.0, 176.0, 432.0)),
            (575.0, Rect::new(200.0, 24.0, 351.0, 432.0)),
            (576.0, Rect::new(200.0, 24.0, 352.0, 432.0)),
        ] {
            let overlay = overlay_at_width(width);
            assert_eq!(overlay.sync_page_rect, expected_content_rect);

            let generic_page_inset =
                if overlay.generic_page_rect.w < SETTINGS_COMPACT_LAYOUT_THRESHOLD_LOGICAL {
                    SETTINGS_COMPACT_FORM_INSET_LOGICAL
                } else {
                    SETTINGS_FORM_INSET_LOGICAL
                };
            assert_eq!(overlay.generic_page_rect.x + generic_page_inset, expected_content_rect.x,);
            assert_eq!(
                overlay.generic_page_rect.w - 2.0 * generic_page_inset,
                expected_content_rect.w,
            );
        }
    }

    #[test]
    fn generic_page_keeps_its_settings_widget_action() {
        let mut overlay = laid_out_overlay(SettingsPersistenceView::Saved);
        click_category(&mut overlay, 3);
        click_category(&mut overlay, 0);
        assert_eq!(overlay.active_category, ProductSettingsCategory::Appearance);

        let draw_list = paint_overlay(&overlay);
        let system_theme_button_rect = button_rect_for_text(&draw_list, "跟随系统");

        assert_eq!(
            click_at(
                &mut overlay,
                system_theme_button_rect.x + system_theme_button_rect.w * 0.5,
                system_theme_button_rect.y + system_theme_button_rect.h * 0.5,
            ),
            Some(WidgetAction::Settings(SettingsViewAction::SetThemeMode(ThemeMode::System,))),
        );
        assert_eq!(overlay.take_pending_sync_action(), None);
    }

    #[test]
    fn product_overlay_exposes_active_page_semantics_and_routes_category_activation() {
        let mut overlay = laid_out_overlay(SettingsPersistenceView::Saved);
        let mut nodes = Vec::new();
        overlay.collect_accessibility_nodes(
            &ui::core::AccessibilityContext::new(40.0, 60.0),
            &mut nodes,
        );

        assert!(semantic_roles(&nodes).contains(&ui::core::AccessibilityRole::TextField));
        let mut root = ui::core::AccessibilityNode::new(
            ui::core::AccessibilityId(0x7465_7874_6f72_6173),
            ui::core::AccessibilityRole::Group,
            Rect::new(40.0, 60.0, 720.0, 480.0),
        );
        root.children = nodes;
        assert_eq!(ui::core::AccessibilityTree::new(root, None).validate(), Ok(()));

        assert_eq!(
            overlay.on_accessibility_action(&ui::core::AccessibilityActionRequest::new(
                ui::core::AccessibilityId::from(EDITOR_CATEGORY_ID),
                ui::core::AccessibilityAction::Activate,
            )),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(overlay.active_category, ProductSettingsCategory::Editor);
    }

    #[test]
    fn product_category_press_is_cleared_by_interaction_cancel() {
        let mut overlay = laid_out_overlay(SettingsPersistenceView::Saved);
        let editor_rect = overlay.category_rects[1];
        let pointer = (editor_rect.x + editor_rect.w * 0.5, editor_rect.y + editor_rect.h * 0.5);
        let theme = ui::theme::test_theme();
        let mut event_ctx = EventCtx { theme: &theme, dpi: 1.0, cursor_hint: None };

        assert!(
            overlay
                .on_event(
                    &Event::MouseDown { px: pointer.0, py: pointer.1, button: MouseButton::Left },
                    &mut event_ctx,
                )
                .is_some()
        );
        assert!(overlay.is_capturing());

        overlay.on_event(&Event::PointerLeave, &mut event_ctx);
        assert!(overlay.is_capturing());

        assert_eq!(
            overlay.on_event(&Event::InteractionCancel, &mut event_ctx),
            Some(WidgetAction::Consumed)
        );
        assert!(!overlay.is_capturing());
        assert_eq!(overlay.on_event(&Event::InteractionCancel, &mut event_ctx), None);
        assert_eq!(
            overlay.on_event(
                &Event::MouseUp { px: pointer.0, py: pointer.1, button: MouseButton::Left },
                &mut event_ctx,
            ),
            None
        );
        assert_eq!(overlay.active_category, ProductSettingsCategory::Appearance);
    }

    fn settings_input(persistence: SettingsPersistenceView) -> SettingsViewInput {
        SettingsViewInput {
            theme_mode: ThemeMode::System,
            font_family: "Menlo".to_owned(),
            font_size: 15.0,
            line_height_ratio: 1.618,
            word_wrap: true,
            show_line_numbers: true,
            tab_width: 4,
            view_mode: ViewMode::Sidebar,
            show_status_bar: false,
            persistence,
        }
    }

    fn semantic_roles(nodes: &[ui::core::AccessibilityNode]) -> Vec<ui::core::AccessibilityRole> {
        let mut roles = Vec::new();
        for node in nodes {
            roles.push(node.role);
            roles.extend(semantic_roles(&node.children));
        }
        roles
    }

    fn laid_out_overlay(persistence: SettingsPersistenceView) -> TextoraSettingsOverlay {
        overlay_at_width_with_persistence(720.0, persistence)
    }

    fn overlay_at_width(width: f32) -> TextoraSettingsOverlay {
        overlay_at_width_with_persistence(width, SettingsPersistenceView::Saved)
    }

    fn overlay_at_width_with_persistence(
        width: f32,
        persistence: SettingsPersistenceView,
    ) -> TextoraSettingsOverlay {
        let theme = ui::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        let mut overlay =
            TextoraSettingsOverlay::new(settings_input(persistence), SyncSettingsInput::default());
        overlay.set_rect(Rect::new(0.0, 0.0, width, 480.0), &mut layout);
        overlay
    }

    fn click_category(overlay: &mut TextoraSettingsOverlay, index: usize) {
        let category_rect = overlay.category_rects[index];
        let click_x = category_rect.x + category_rect.w * 0.5;
        let click_y = category_rect.y + category_rect.h * 0.5;
        click_at(overlay, click_x, click_y);
    }

    fn click_at(
        overlay: &mut TextoraSettingsOverlay,
        click_x: f32,
        click_y: f32,
    ) -> Option<WidgetAction> {
        let theme = ui::theme::test_theme();
        let mut event_ctx = EventCtx { theme: &theme, dpi: 1.0, cursor_hint: None };
        overlay.on_event(
            &Event::MouseDown { px: click_x, py: click_y, button: MouseButton::Left },
            &mut event_ctx,
        );
        overlay.on_event(
            &Event::MouseUp { px: click_x, py: click_y, button: MouseButton::Left },
            &mut event_ctx,
        )
    }

    fn paint_overlay(overlay: &TextoraSettingsOverlay) -> DrawList {
        let theme = ui::theme::test_theme();
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        overlay.paint(&mut PaintCtx {
            list: &mut draw_list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            global_alpha: 1.0,
            shaper: Some(&mut shaper),
        });
        draw_list
    }

    fn button_rect_for_text(draw_list: &DrawList, text: &str) -> Rect {
        const ACTION_BUTTON_RADIUS_LOGICAL: f32 = 8.0;

        let text_index = draw_list
            .cmds
            .iter()
            .position(|command| {
                matches!(command, DrawCmd::TextLayout { layout, .. } if layout.text == text)
            })
            .expect("expected action button text to be painted");
        draw_list.cmds[..text_index]
            .iter()
            .rev()
            .find_map(|command| match command {
                DrawCmd::FillRect { rect, radius, .. }
                    if *radius == ACTION_BUTTON_RADIUS_LOGICAL =>
                {
                    Some(*rect)
                }
                _ => None,
            })
            .expect("expected action button background before its text")
    }
}
