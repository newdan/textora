//! 外壳布局与各图层的绘制顺序。

use crate::encrypted_note_dialog::{EncryptedNoteDialog, EncryptedNoteDialogInput};
use crate::encrypted_note_unlock::{EncryptedNoteUnlock, EncryptedNoteUnlockInput};
use appkit_shell::editor_runtime::{EditorFrame, RenderError};
use ui::canvas_scrollbars::CanvasScrollbarsInput;
use ui::icon::draw_icon;
use ui::inspector_panel::PANEL_WIDTH_LOGICAL;
use ui::popup_menu::PopupMenuWidget;
use ui::splitter::SplitterInput;
use ui::tooltip::TooltipWidget;
use ui::{Rect, Widget};

use crate::ResponsiveLayoutMode;
use crate::editor_pane::EditorPaneRects;
use crate::new_workspace_dialog::{NewWorkspaceDialog, NewWorkspaceDialogInput};
use crate::shell::layout::ShellLayout;

use super::buttons::ChromeButtonKey;
use super::{
    EditorPaneState, NOTE_TOOL_ICON_SIZE_LOGICAL, NewNoteControlState, NoteToolbarButtonInput,
    NotoraRenderModel, NotoraShell, RenderedToolbarButton, SEARCH_ICON_AREA_WIDTH_LOGICAL,
    SIDEBAR_ICON_SIZE_LOGICAL,
};

pub(super) const NEW_NOTE_BUTTON_WIDTH_LOGICAL: f32 =
    ui::button::ButtonMetrics::split_icon_text_width(2, NOTE_TOOL_ICON_SIZE_LOGICAL);
pub(super) const NOTE_TOOL_BUTTON_WIDTH_LOGICAL: f32 =
    ui::button::ButtonMetrics::icon_text_width(2, NOTE_TOOL_ICON_SIZE_LOGICAL);
pub(super) const NOTE_TOOL_BUTTON_HEIGHT_LOGICAL: f32 = 28.0;
pub(super) const NOTE_TOOL_BUTTON_GAP_LOGICAL: f32 = ui::button::ButtonMetrics::ACTION_GAP;
const CARD_HEADER_TITLE_FONT_SIZE_LOGICAL: f32 = 16.0;
const CARD_HEADER_TITLE_BASELINE_LOGICAL: f32 = 28.0;
const CARD_HEADER_CONTROL_TOP_LOGICAL: f32 = 8.0;
const CARD_HEADER_WRAPPED_CONTROL_TOP_LOGICAL: f32 = 44.0;
const CARD_HEADER_CONTENT_TOP_LOGICAL: f32 = 44.0;
const CARD_HEADER_WRAPPED_CONTENT_TOP_LOGICAL: f32 = 80.0;
pub(super) const SEARCH_BAR_HEIGHT_LOGICAL: f32 = 32.0;
pub(super) const SHELL_PADDING_LOGICAL: f32 = 12.0;
const SIDEBAR_CONTROL_HEIGHT_LOGICAL: f32 = 32.0;
const COMPACT_NAVIGATION_BUTTON_WIDTH_LOGICAL: f32 = ui::button::ButtonMetrics::text_width(3);
const COMPACT_BACK_BUTTON_WIDTH_LOGICAL: f32 = ui::button::ButtonMetrics::text_width(2);
const NAVIGATION_VISIBILITY_BUTTON_SIZE_LOGICAL: f32 = 28.0;
const NAVIGATION_COLLAPSE_BUTTON_GAP_LOGICAL: f32 = 6.0;
const CONFIRMATION_PANEL_WIDTH_LOGICAL: f32 = 360.0;
const CONFIRMATION_PANEL_HEIGHT_LOGICAL: f32 = 160.0;
const CONFIRMATION_BUTTON_WIDTH_LOGICAL: f32 = ui::button::ButtonMetrics::text_width(4);
const CANCEL_BUTTON_WIDTH_LOGICAL: f32 = ui::button::ButtonMetrics::text_width(2);
const CONFIRMATION_BUTTON_HEIGHT_LOGICAL: f32 = 32.0;
const SAVE_CONFLICT_PANEL_WIDTH_LOGICAL: f32 = 440.0;
const SAVE_CONFLICT_PANEL_HEIGHT_LOGICAL: f32 = 196.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CardHeaderLayout {
    pub(super) title_x: f32,
    title_baseline_y: f32,
    pub(super) toolbar_rect: Rect,
    pub(super) content_top_y: f32,
}

pub(crate) fn editor_inspector_rects(layout: ShellLayout, inspector_open: bool) -> (Rect, Rect) {
    let body = layout.editor_body_rect;
    if !inspector_open {
        return (body, Rect::ZERO);
    }
    let panel_width = (PANEL_WIDTH_LOGICAL * layout.dpi).min(body.w.max(0.0));
    (
        Rect::new(body.x, body.y, (body.w - panel_width).max(0.0), body.h),
        Rect::new(body.right() - panel_width, body.y, panel_width, body.h),
    )
}

impl NotoraShell {
    pub fn render(
        &mut self,
        frame: &mut EditorFrame,
        layout: ShellLayout,
        model: &NotoraRenderModel,
    ) -> Result<(), RenderError> {
        self.render_with_document_search(frame, layout, model, None)
    }

    pub(crate) fn render_with_document_search(
        &mut self,
        frame: &mut EditorFrame,
        layout: ShellLayout,
        model: &NotoraRenderModel,
        document_search: Option<&ui::document_search_bar::DocumentSearchBarWidget>,
    ) -> Result<(), RenderError> {
        self.update_model(model);
        let dpi = layout.dpi;
        let padding = SHELL_PADDING_LOGICAL * dpi;
        self.navigation_collapse_rect = navigation_collapse_button_rect(layout, dpi, padding);
        self.navigation_expand_rect = navigation_expand_button_rect(layout, dpi, padding);
        let search_right = if self.navigation_collapse_rect == Rect::ZERO {
            layout.navigation_rect.right() - padding
        } else {
            self.navigation_collapse_rect.x - NAVIGATION_COLLAPSE_BUTTON_GAP_LOGICAL * dpi
        };
        let search_rect = Rect::new(
            layout.navigation_rect.x + padding,
            layout.navigation_rect.y + padding,
            (search_right - layout.navigation_rect.x - padding).max(0.0),
            SEARCH_BAR_HEIGHT_LOGICAL * dpi,
        );
        let search_icon_area_width = SEARCH_ICON_AREA_WIDTH_LOGICAL * dpi;
        let settings_rect = Rect::new(
            layout.navigation_rect.x + padding,
            layout.navigation_rect.bottom() - (SIDEBAR_CONTROL_HEIGHT_LOGICAL + 10.0) * dpi,
            (layout.navigation_rect.w - padding * 2.0).max(0.0),
            SIDEBAR_CONTROL_HEIGHT_LOGICAL * dpi,
        );
        let tree_rect = Rect::new(
            layout.navigation_rect.x + padding,
            search_rect.bottom() + padding,
            (layout.navigation_rect.w - padding * 2.0).max(0.0),
            (settings_rect.y - search_rect.bottom() - padding * 2.0).max(0.0),
        );
        self.navigation_tree_rect = tree_rect;
        self.search_rect = search_rect;
        self.editor_rect = layout.editor_rect;
        let (editor_body_rect, inspector_panel_rect) =
            editor_inspector_rects(layout, model.inspector_panel.is_some());
        self.inspector_panel_rect = inspector_panel_rect;
        if let Some(panel) = &model.inspector_panel {
            self.inspector_panel.set_input(panel.descriptor.clone(), &panel.presentation);
        }
        self.inspector_panel.set_keyboard_focus(
            model.inspector_panel.as_ref().map(|_| ui::core::widget::ids::INSPECTOR_PANEL),
        );
        frame.with_layout_context(|context| {
            self.editor_pane.set_rects(
                EditorPaneRects {
                    header: layout.editor_header_rect,
                    toolbar: layout.editor_toolbar_rect,
                    body: editor_body_rect,
                },
                context,
            );
            self.inspector_panel.set_rect(local_rect(inspector_panel_rect), context);
        });
        self.inspector_panel_stamp =
            model.inspector_panel.as_ref().map(|panel| panel.stamp.clone());
        self.settings_rect = settings_rect;
        let card_header = card_header_layout(
            layout.card_list_rect,
            dpi,
            &model.card_list_title,
            model.new_note_control,
            model.note_toolbar.len(),
            self.navigation_expand_rect,
        );
        let new_note_rect =
            new_note_button_rect(card_header.toolbar_rect, dpi, model.new_note_control);
        if model.show_new_document_menu {
            let metrics =
                ui::settings::UiMetrics::from_settings(&ui::settings::Settings::new(), dpi);
            let menu = ui::sidebar::build_new_document_menu(
                new_note_rect,
                (layout.overlay_rect.right(), layout.overlay_rect.bottom()),
                &metrics,
                model.new_note_control != NewNoteControlState::EnabledForFiles,
            );
            self.synchronize_new_document_menu(menu);
        } else {
            self.new_document_menu_rect = Rect::ZERO;
            self.new_document_menu = None;
        }
        let splitters_enabled = layout.responsive_mode == ResponsiveLayoutMode::ThreePane;
        self.navigation_splitter.set_input(SplitterInput {
            logical_position: layout.navigation_width_logical,
            minimum_logical_position: crate::shell::layout::MINIMUM_NAVIGATION_WIDTH_LOGICAL,
            maximum_logical_position: crate::shell::layout::MAXIMUM_NAVIGATION_WIDTH_LOGICAL,
            enabled: splitters_enabled,
        });
        self.card_list_splitter.set_input(SplitterInput {
            logical_position: layout.card_list_width_logical,
            minimum_logical_position: crate::shell::layout::MINIMUM_CARD_LIST_WIDTH_LOGICAL,
            maximum_logical_position: crate::shell::layout::MAXIMUM_CARD_LIST_WIDTH_LOGICAL,
            enabled: layout.responsive_mode != ResponsiveLayoutMode::EditorOverlay,
        });
        let card_content_rect = Rect::new(
            layout.card_list_rect.x + padding,
            card_header.content_top_y,
            (layout.card_list_rect.w - padding * 2.0).max(0.0),
            (layout.card_list_rect.bottom() - card_header.content_top_y - padding).max(0.0),
        );
        self.card_content_rect = card_content_rect;
        let tool_button_height = NOTE_TOOL_BUTTON_HEIGHT_LOGICAL * dpi;
        self.note_toolbar_buttons =
            layout_note_toolbar(card_header.toolbar_rect, dpi, new_note_rect, &model.note_toolbar);
        self.compact_navigation_rect = if !layout.has_immersive_title_bar()
            && layout.responsive_mode != ResponsiveLayoutMode::ThreePane
            && layout.navigation_rect == Rect::ZERO
        {
            Rect::new(
                layout.card_list_rect.x + padding,
                layout.card_list_rect.y + 8.0 * dpi,
                COMPACT_NAVIGATION_BUTTON_WIDTH_LOGICAL * dpi,
                tool_button_height,
            )
        } else {
            Rect::ZERO
        };
        self.compact_back_rect = if layout.responsive_mode == ResponsiveLayoutMode::EditorOverlay
            && layout.editor_rect != Rect::ZERO
        {
            Rect::new(
                layout.editor_rect.x + padding,
                layout.editor_rect.y + 8.0 * dpi,
                COMPACT_BACK_BUTTON_WIDTH_LOGICAL * dpi,
                tool_button_height,
            )
        } else {
            Rect::ZERO
        };
        self.layout_confirmation_overlay(layout.overlay_rect, dpi, model.confirmation.is_some());
        self.layout_save_conflict_overlay(layout.overlay_rect, dpi, model.save_conflict.is_some());
        frame.with_layout_context(|context| {
            self.search_box.set_rect(search_rect, context);
            self.navigation_tree.set_rect(tree_rect, context);
            self.card_list.set_rect(card_content_rect, context);
            self.card_empty_state.set_rect(card_content_rect, context);
            self.editor_empty_state.set_rect(layout.editor_body_rect, context);
            self.navigation_splitter.set_rect(layout.navigation_splitter_rect, context);
            self.card_list_splitter.set_rect(layout.card_list_splitter_rect, context);
            self.new_note_button.set_rect(new_note_rect, context);
            self.synchronize_chrome_buttons(
                model.confirmation.as_ref().map(|confirmation| confirmation.confirm_label.as_str()),
                context,
            );
            if let Some(menu) = self.new_document_menu.as_mut() {
                menu.set_rect(local_rect(self.new_document_menu_rect), context);
            }
            if model.show_settings_overlay {
                self.settings_overlay.set_rect(layout.overlay_rect, context);
            }
            if self.new_workspace_dialog_open {
                let dialog = self
                    .new_workspace_dialog
                    .get_or_insert_with(|| NewWorkspaceDialog::new(context.theme));
                dialog.set_input(self.new_workspace_dialog_input.clone(), true);
                dialog.set_rect(layout.overlay_rect, context);
            } else if let Some(dialog) = self.new_workspace_dialog.as_mut() {
                dialog.set_input(NewWorkspaceDialogInput::default(), false);
            }
            if self.encrypted_note_dialog_open {
                let dialog = self
                    .encrypted_note_dialog
                    .get_or_insert_with(|| EncryptedNoteDialog::new(context.theme));
                dialog.set_input(self.encrypted_note_dialog_input.clone(), true);
                dialog.set_rect(layout.overlay_rect, context);
            } else if let Some(dialog) = self.encrypted_note_dialog.as_mut() {
                dialog.set_input(EncryptedNoteDialogInput::create(), false);
            }
            if self.encrypted_note_unlock_open {
                let unlock = self
                    .encrypted_note_unlock
                    .get_or_insert_with(|| EncryptedNoteUnlock::new(context.theme));
                unlock.set_input(self.encrypted_note_unlock_input.clone(), true);
                unlock.set_rect(layout.editor_body_rect, context);
            } else if let Some(unlock) = self.encrypted_note_unlock.as_mut() {
                unlock.set_input(EncryptedNoteUnlockInput::new(String::new()), false);
            }
        });
        frame.with_underlay_paint_context(|context| {
            let application_theme = context.theme.application_theme();
            context.list.fill(layout.overlay_rect, application_theme.window_surface);
            // 顶部和导航属于同一外壳；中栏比外壳浅，正文保持编辑器底色。
            context.list.fill(layout.navigation_rect, application_theme.window_surface);
            context.list.fill(layout.card_list_rect, application_theme.navigation_surface);
            context.list.fill(layout.editor_rect, application_theme.editor_surface);
            self.editor_pane.paint_underlay(context);
            if layout.navigation_rect != Rect::ZERO {
                self.search_box.paint(context);
                let search_icon_size = SIDEBAR_ICON_SIZE_LOGICAL * context.dpi;
                draw_icon(
                    context.list,
                    "search",
                    search_rect.x + (search_icon_area_width - search_icon_size) * 0.5,
                    search_rect.y + (search_rect.h - search_icon_size) * 0.5,
                    search_icon_size,
                    application_theme.text_secondary,
                );
                self.navigation_tree.paint(context);
                if self.navigation_collapse_rect != Rect::ZERO {
                    self.paint_chrome_button(context, ChromeButtonKey::NavigationCollapse);
                }
                self.paint_chrome_button(context, ChromeButtonKey::Settings);
            }
            self.paint_navigation_splitter(context, layout);
            self.card_list_splitter.paint(context);
            context.text_with_font(
                card_header.title_x,
                card_header.title_baseline_y,
                CARD_HEADER_TITLE_FONT_SIZE_LOGICAL * context.dpi,
                application_theme.text_primary,
                &model.card_list_title,
                None,
                shaping::Weight::SEMIBOLD,
                shaping::Style::Normal,
            );
            if self.compact_navigation_rect != Rect::ZERO {
                self.paint_chrome_button(context, ChromeButtonKey::CompactNavigation);
            }
            if self.navigation_expand_rect != Rect::ZERO {
                self.paint_chrome_button(context, ChromeButtonKey::NavigationExpand);
            }
            self.paint_note_toolbar(context);
            if model.cards.is_empty() {
                self.card_empty_state.paint(context);
            } else {
                self.card_list.paint(context);
            }
        });
        match model.editor_pane {
            EditorPaneState::Empty => {
                frame.paint_editor_with(layout.editor_body_rect, |context| {
                    self.editor_empty_state.paint(context)
                })?;
            }
            EditorPaneState::Active => frame.paint_editor(editor_body_rect)?,
            EditorPaneState::EncryptedUnlock => {
                frame.paint_editor_with(layout.editor_body_rect, |context| {
                    if let Some(unlock) = self.encrypted_note_unlock.as_ref() {
                        unlock.paint(context);
                    }
                })?;
            }
        }
        frame.with_paint_context(|context| {
            self.paint_canvas_scrollbars(context);
            if let Some(search_bar) = document_search {
                search_bar.paint(context);
            }
            self.editor_pane.paint_overlay(context);
        });
        frame.with_paint_context(|context| {
            ui::rounded_surface_frame::RoundedSurfaceFrame {
                rect: layout.content_surface_rect(),
                radius: ui::rounded_surface_frame::SHELL_CORNER_RADIUS_LOGICAL * dpi,
            }
            .paint(context);
        });
        if self.inspector_panel_stamp.is_some() {
            frame.with_paint_context(|context| {
                paint_at(context, self.inspector_panel_rect, |context| {
                    self.inspector_panel.paint(context)
                });
            });
        }
        if layout.responsive_mode != ResponsiveLayoutMode::ThreePane
            && layout.navigation_rect != Rect::ZERO
        {
            frame.with_paint_context(|context| self.paint_compact_navigation_overlay(context));
        }
        if self.compact_back_rect != Rect::ZERO {
            frame.with_paint_context(|context| {
                self.paint_chrome_button(context, ChromeButtonKey::CompactBack);
            });
        }
        if model.show_settings_overlay {
            frame.with_paint_context(|context| {
                context.list.fill_rounded(
                    layout.overlay_rect,
                    context.theme.application_theme().modal_scrim,
                    0.0,
                );
                self.settings_overlay.paint(context);
            });
        }
        if self.new_workspace_dialog_open
            && let Some(dialog) = self.new_workspace_dialog.as_ref()
        {
            frame.with_paint_context(|context| {
                context.list.fill_rounded(
                    layout.overlay_rect,
                    context.theme.application_theme().modal_scrim,
                    0.0,
                );
                dialog.paint(context);
            });
        }
        if self.encrypted_note_dialog_open
            && let Some(dialog) = self.encrypted_note_dialog.as_ref()
        {
            frame.with_paint_context(|context| {
                context.list.fill_rounded(
                    layout.overlay_rect,
                    context.theme.application_theme().modal_scrim,
                    0.0,
                );
                dialog.paint(context);
            });
        }
        if let Some(confirmation) = &model.confirmation {
            frame.with_paint_context(|context| {
                let application_theme = context.theme.application_theme();
                context.list.fill_rounded(layout.overlay_rect, application_theme.modal_scrim, 0.0);
                context.list.fill_rounded(
                    self.confirmation_panel_rect,
                    application_theme.overlay_surface,
                    10.0 * context.dpi,
                );
                context.text(
                    self.confirmation_panel_rect.x + 20.0 * context.dpi,
                    self.confirmation_panel_rect.y + 38.0 * context.dpi,
                    17.0 * context.dpi,
                    application_theme.text_primary,
                    &confirmation.title,
                );
                context.text(
                    self.confirmation_panel_rect.x + 20.0 * context.dpi,
                    self.confirmation_panel_rect.y + 72.0 * context.dpi,
                    13.0 * context.dpi,
                    application_theme.text_secondary,
                    &confirmation.description,
                );
                self.paint_chrome_button(context, ChromeButtonKey::ConfirmationCancel);
                self.paint_chrome_button(context, ChromeButtonKey::ConfirmationConfirm);
            });
        }
        if model.save_conflict.is_some() {
            frame.with_paint_context(|context| {
                context.list.fill_rounded(
                    layout.overlay_rect,
                    context.theme.application_theme().modal_scrim,
                    0.0,
                );
                self.paint_save_conflict_overlay(context);
            });
        }
        if let Some(menu) = self.new_document_menu.as_ref() {
            frame.with_paint_context(|context| {
                paint_at(context, self.new_document_menu_rect, |context| menu.paint(context));
            });
        }
        let product_overlay_visible = model.show_settings_overlay
            || model.new_workspace_dialog.is_some()
            || model.confirmation.is_some()
            || model.save_conflict.is_some()
            || model.show_new_document_menu;
        if !product_overlay_visible && let Some(hint) = self.active_tooltip.as_ref() {
            let (tooltip, tooltip_rect) =
                TooltipWidget::new(hint, layout.dpi, layout.overlay_rect.w, layout.overlay_rect.h);
            frame.with_paint_context(|context| {
                paint_at(context, tooltip_rect, |context| tooltip.paint(context));
            });
        }
        Ok(())
    }

    pub fn set_canvas_scrollbars_input(
        &mut self,
        input: Option<CanvasScrollbarsInput>,
        canvas_rect: Rect,
        context: &mut ui::LayoutCtx<'_>,
    ) {
        self.canvas_scrollbars_input = input;
        self.canvas_rect = canvas_rect;
        self.canvas_scrollbars.set_input(input.unwrap_or_default());
        self.canvas_scrollbars.set_rect(local_rect(canvas_rect), context);
    }

    pub fn paint_canvas_scrollbars(&self, context: &mut ui::PaintCtx<'_>) {
        if self.canvas_scrollbars_input.is_none() {
            return;
        }
        paint_at(context, self.canvas_rect, |context| self.canvas_scrollbars.paint(context));
    }

    fn paint_navigation_splitter(&self, context: &mut ui::PaintCtx<'_>, layout: ShellLayout) {
        let surface = layout.content_surface_rect();
        let radius = (ui::rounded_surface_frame::SHELL_CORNER_RADIUS_LOGICAL * layout.dpi)
            .min(surface.w * 0.5)
            .min(surface.h * 0.5);
        let visual_rect = Rect::new(
            layout.navigation_splitter_rect.x,
            surface.y + radius,
            layout.navigation_splitter_rect.w,
            (surface.h - radius * 2.0).max(0.0),
        );
        self.navigation_splitter.paint_handle(context, visual_rect);
    }

    pub(super) fn synchronize_new_document_menu(&mut self, menu: ui::popup_menu::PopupMenu) {
        let menu_size_changed = self.new_document_menu_rect.w != menu.menu_rect.w
            || self.new_document_menu_rect.h != menu.menu_rect.h;
        self.new_document_menu_rect = menu.menu_rect;
        if self.new_document_menu.is_none() || menu_size_changed {
            self.new_document_menu = Some(PopupMenuWidget::new(menu));
        }
    }

    fn layout_confirmation_overlay(&mut self, overlay_rect: Rect, dpi: f32, visible: bool) {
        if !visible {
            self.confirmation_panel_rect = Rect::ZERO;
            self.confirmation_confirm_rect = Rect::ZERO;
            self.confirmation_cancel_rect = Rect::ZERO;
            return;
        }
        let panel_width = (CONFIRMATION_PANEL_WIDTH_LOGICAL * dpi).min(overlay_rect.w.max(0.0));
        let panel_height = (CONFIRMATION_PANEL_HEIGHT_LOGICAL * dpi).min(overlay_rect.h.max(0.0));
        self.confirmation_panel_rect = Rect::new(
            overlay_rect.x + (overlay_rect.w - panel_width) * 0.5,
            overlay_rect.y + (overlay_rect.h - panel_height) * 0.5,
            panel_width,
            panel_height,
        );
        let button_width = CONFIRMATION_BUTTON_WIDTH_LOGICAL * dpi;
        let cancel_width = CANCEL_BUTTON_WIDTH_LOGICAL * dpi;
        let button_height = CONFIRMATION_BUTTON_HEIGHT_LOGICAL * dpi;
        let button_y = self.confirmation_panel_rect.bottom() - button_height - 16.0 * dpi;
        self.confirmation_confirm_rect = Rect::new(
            self.confirmation_panel_rect.right() - button_width - 20.0 * dpi,
            button_y,
            button_width,
            button_height,
        );
        self.confirmation_cancel_rect = Rect::new(
            self.confirmation_confirm_rect.x
                - cancel_width
                - ui::button::ButtonMetrics::ACTION_GAP * dpi,
            button_y,
            cancel_width,
            button_height,
        );
    }

    fn layout_save_conflict_overlay(&mut self, overlay_rect: Rect, dpi: f32, visible: bool) {
        if !visible {
            self.save_conflict_panel_rect = Rect::ZERO;
            self.save_conflict_button_rects = [Rect::ZERO; 4];
            return;
        }
        let panel_width = (SAVE_CONFLICT_PANEL_WIDTH_LOGICAL * dpi).min(overlay_rect.w.max(0.0));
        let panel_height = (SAVE_CONFLICT_PANEL_HEIGHT_LOGICAL * dpi).min(overlay_rect.h.max(0.0));
        self.save_conflict_panel_rect = Rect::new(
            overlay_rect.x + (overlay_rect.w - panel_width) * 0.5,
            overlay_rect.y + (overlay_rect.h - panel_height) * 0.5,
            panel_width,
            panel_height,
        );
        let gap = ui::button::ButtonMetrics::ACTION_GAP * dpi;
        let horizontal_padding = 20.0 * dpi;
        let button_height = CONFIRMATION_BUTTON_HEIGHT_LOGICAL * dpi;
        let button_width = ((panel_width - horizontal_padding * 2.0 - gap * 3.0) / 4.0)
            .max(0.0)
            .min(ui::button::ButtonMetrics::text_width(4) * dpi);
        let button_y = self.save_conflict_panel_rect.bottom() - button_height - 18.0 * dpi;
        self.save_conflict_button_rects = std::array::from_fn(|index| {
            Rect::new(
                self.save_conflict_panel_rect.right()
                    - horizontal_padding
                    - button_width * 4.0
                    - gap * 3.0
                    + index as f32 * (button_width + gap),
                button_y,
                button_width,
                button_height,
            )
        });
    }

    fn paint_save_conflict_overlay(&self, context: &mut ui::PaintCtx<'_>) {
        let application_theme = context.theme.application_theme();
        context.list.fill_rounded(
            self.save_conflict_panel_rect,
            application_theme.overlay_surface,
            10.0 * context.dpi,
        );
        context.text(
            self.save_conflict_panel_rect.x + 20.0 * context.dpi,
            self.save_conflict_panel_rect.y + 38.0 * context.dpi,
            17.0 * context.dpi,
            application_theme.text_primary,
            "磁盘文件已更改",
        );
        context.text(
            self.save_conflict_panel_rect.x + 20.0 * context.dpi,
            self.save_conflict_panel_rect.y + 72.0 * context.dpi,
            13.0 * context.dpi,
            application_theme.text_secondary,
            "请选择如何处理本地编辑，文件不会被静默覆盖。",
        );
        for index in 0..self.save_conflict_button_rects.len() {
            self.paint_chrome_button(context, ChromeButtonKey::SaveConflict(index));
        }
    }

    fn paint_compact_navigation_overlay(&self, context: &mut ui::PaintCtx<'_>) {
        let application_theme = context.theme.application_theme();
        let panel_rect = Rect::new(
            self.search_rect.x - SHELL_PADDING_LOGICAL * context.dpi,
            0.0,
            self.search_rect.w + SHELL_PADDING_LOGICAL * context.dpi * 2.0,
            self.settings_rect.bottom() + 10.0 * context.dpi,
        );
        context.list.fill(panel_rect, application_theme.window_surface);
        self.search_box.paint(context);
        let search_icon_size = SIDEBAR_ICON_SIZE_LOGICAL * context.dpi;
        draw_icon(
            context.list,
            "search",
            self.search_rect.x
                + (SEARCH_ICON_AREA_WIDTH_LOGICAL * context.dpi - search_icon_size) * 0.5,
            self.search_rect.y + (self.search_rect.h - search_icon_size) * 0.5,
            search_icon_size,
            application_theme.text_secondary,
        );
        self.navigation_tree.paint(context);
        self.paint_chrome_button(context, ChromeButtonKey::Settings);
    }
}

pub(super) fn layout_note_toolbar(
    toolbar_rect: Rect,
    dpi: f32,
    new_note_rect: Rect,
    inputs: &[NoteToolbarButtonInput],
) -> Vec<RenderedToolbarButton> {
    let gap = NOTE_TOOL_BUTTON_GAP_LOGICAL * dpi;
    let trailing_inset = if new_note_rect.w > 0.0 { new_note_rect.w + gap } else { 0.0 };
    let available_width = (toolbar_rect.w - trailing_inset).max(0.0);
    let count = inputs.len();
    let fitted_width = if count == 0 {
        0.0
    } else {
        ((available_width - gap * count.saturating_sub(1) as f32) / count as f32)
            .max(0.0)
            .min(NOTE_TOOL_BUTTON_WIDTH_LOGICAL * dpi)
    };
    inputs
        .iter()
        .rev()
        .enumerate()
        .map(|(index, input)| RenderedToolbarButton {
            rect: Rect::new(
                toolbar_rect.right()
                    - trailing_inset
                    - fitted_width
                    - index as f32 * (fitted_width + gap),
                toolbar_rect.y,
                fitted_width,
                toolbar_rect.h,
            ),
            icon: input.icon,
            label: input.label.clone(),
            action: input.action.clone(),
        })
        .collect()
}

fn navigation_collapse_button_rect(layout: ShellLayout, dpi: f32, padding: f32) -> Rect {
    if layout.has_immersive_title_bar()
        || layout.responsive_mode != ResponsiveLayoutMode::ThreePane
        || layout.navigation_rect == Rect::ZERO
    {
        return Rect::ZERO;
    }
    let button_size = NAVIGATION_VISIBILITY_BUTTON_SIZE_LOGICAL * dpi;
    Rect::new(
        layout.navigation_rect.right() - padding - button_size,
        layout.navigation_rect.y + padding + (SEARCH_BAR_HEIGHT_LOGICAL * dpi - button_size) * 0.5,
        button_size,
        button_size,
    )
}

fn navigation_expand_button_rect(layout: ShellLayout, dpi: f32, padding: f32) -> Rect {
    if layout.has_immersive_title_bar()
        || layout.responsive_mode != ResponsiveLayoutMode::ThreePane
        || layout.navigation_rect != Rect::ZERO
    {
        return Rect::ZERO;
    }
    Rect::new(
        layout.card_list_rect.x + padding,
        layout.card_list_rect.y + 8.0 * dpi,
        NAVIGATION_VISIBILITY_BUTTON_SIZE_LOGICAL * dpi,
        NAVIGATION_VISIBILITY_BUTTON_SIZE_LOGICAL * dpi,
    )
}

pub(super) fn card_header_layout(
    card_list_rect: Rect,
    dpi: f32,
    title: &str,
    new_note_state: NewNoteControlState,
    toolbar_button_count: usize,
    leading_control_rect: Rect,
) -> CardHeaderLayout {
    let padding = SHELL_PADDING_LOGICAL * dpi;
    let gap = NOTE_TOOL_BUTTON_GAP_LOGICAL * dpi;
    let default_title_x = card_list_rect.x + padding;
    let title_x = if leading_control_rect == Rect::ZERO {
        default_title_x
    } else {
        (leading_control_rect.right() + gap).max(default_title_x)
    };
    let leading_control_inset = title_x - default_title_x;
    let title_width = ui::core::text_util::estimate_text_width_px(
        title,
        CARD_HEADER_TITLE_FONT_SIZE_LOGICAL * dpi,
    );
    let toolbar_width = if toolbar_button_count == 0 {
        0.0
    } else {
        NOTE_TOOL_BUTTON_WIDTH_LOGICAL * dpi * toolbar_button_count as f32
            + gap * toolbar_button_count.saturating_sub(1) as f32
    };
    let new_note_width =
        if new_note_state.is_visible() { NEW_NOTE_BUTTON_WIDTH_LOGICAL * dpi } else { 0.0 };
    let control_group_gap = if toolbar_width > 0.0 && new_note_width > 0.0 { gap } else { 0.0 };
    let controls_width = toolbar_width + control_group_gap + new_note_width;
    let title_control_gap = if controls_width > 0.0 { gap } else { 0.0 };
    let inner_width = (card_list_rect.w - padding * 2.0 - leading_control_inset).max(0.0);
    let wraps = title_width + title_control_gap + controls_width > inner_width;
    let control_top_logical = if wraps {
        CARD_HEADER_WRAPPED_CONTROL_TOP_LOGICAL
    } else {
        CARD_HEADER_CONTROL_TOP_LOGICAL
    };
    let content_top_logical = if wraps {
        CARD_HEADER_WRAPPED_CONTENT_TOP_LOGICAL
    } else {
        CARD_HEADER_CONTENT_TOP_LOGICAL
    };

    let fitted_width = controls_width.min((card_list_rect.w - padding * 2.0).max(0.0));
    let toolbar_x = if wraps {
        card_list_rect.x + (card_list_rect.w - fitted_width) * 0.5
    } else {
        card_list_rect.right() - padding - fitted_width
    };
    let toolbar_rect = if fitted_width > 0.0 && card_list_rect.h > 0.0 {
        Rect::new(
            toolbar_x,
            card_list_rect.y + control_top_logical * dpi,
            fitted_width,
            NOTE_TOOL_BUTTON_HEIGHT_LOGICAL * dpi,
        )
    } else {
        Rect::ZERO
    };

    CardHeaderLayout {
        title_x,
        title_baseline_y: card_list_rect.y + CARD_HEADER_TITLE_BASELINE_LOGICAL * dpi,
        toolbar_rect,
        content_top_y: card_list_rect.y + content_top_logical * dpi,
    }
}

pub(super) fn new_note_button_rect(
    toolbar_rect: Rect,
    dpi: f32,
    state: NewNoteControlState,
) -> Rect {
    if !state.is_visible() || toolbar_rect.w <= 0.0 || toolbar_rect.h <= 0.0 {
        return Rect::ZERO;
    }
    let width = (NEW_NOTE_BUTTON_WIDTH_LOGICAL * dpi).min(toolbar_rect.w);
    Rect::new(toolbar_rect.right() - width, toolbar_rect.y, width, toolbar_rect.h)
}

fn local_rect(rect: Rect) -> Rect {
    Rect::new(0.0, 0.0, rect.w, rect.h)
}

fn paint_at(context: &mut ui::PaintCtx<'_>, rect: Rect, paint: impl FnOnce(&mut ui::PaintCtx<'_>)) {
    let saved_offset = context.list.offset;
    context.list.offset = (saved_offset.0 + rect.x, saved_offset.1 + rect.y);
    paint(context);
    context.list.offset = saved_offset;
}
