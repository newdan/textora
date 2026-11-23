use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::encrypted_note_dialog::{EncryptedNoteDialog, EncryptedNoteDialogInput};
use crate::encrypted_note_unlock::{EncryptedNoteUnlock, EncryptedNoteUnlockInput};
use appkit_shell::editor_runtime::EditorFrame;
use ui::button::{ButtonStyle, ButtonVisualState};
use ui::canvas_scrollbars::CanvasScrollbarsInput;
use ui::core::widget::{ControlAction, TextPayload, WidgetId};
use ui::icon::draw_icon;
use ui::popup_menu::{PopupMenuAction, PopupOutcome};
use ui::sidebar::NewDocumentKind;
use ui::split_button::{SplitButtonInput, SplitButtonWidget};
use ui::splitter::SplitterInput;
use ui::tree_list::{TreeListAction, TreeRowEditorInput, TreeRowExpansion, TreeRowKey};
use ui::virtual_card_list::{
    CardInput, CardKey, CardSelection, VirtualCardListAction, VirtualCardListInput,
};
use ui::{Event, EventCtx, Rect, Widget, WidgetAction};

use crate::action::{MetadataMutation, NotoraAction, TrashOperation};
use crate::editor_pane::{EditorPaneInput, EditorPaneMode, EditorPaneRects};
use crate::shell::layout::ShellLayout;
use crate::state::DirectoryCreationState;
use crate::{FocusTarget, OverlayState, WorkspaceRootState};

use super::card_model::{
    document_icon, format_created_time, format_modified_time, format_modified_timestamp_at,
    note_toolbar_buttons,
};
use super::editor_model::{
    EDITOR_ROOT_DIRECTORY_ROW_KEY, editor_command_actions, editor_pane_input,
    history_toolbar_input, markdown_toolbar_input, mindmap_toolbar_input,
};
use super::event_routing::{event_pointer_position, translate_event};
use super::model::{
    DIRECTORY_EDITOR_KEY, NEW_DIRECTORY_ACTION_KEY, NEW_WORKSPACE_ACTION_KEY,
    OPEN_WORKSPACE_ACTION_KEY, TAG_ROOT_NAVIGATION_KEY, confirmation_overlay_input,
};
use super::painting::{
    NEW_NOTE_BUTTON_WIDTH_LOGICAL, NOTE_TOOL_BUTTON_GAP_LOGICAL, NOTE_TOOL_BUTTON_HEIGHT_LOGICAL,
    NOTE_TOOL_BUTTON_WIDTH_LOGICAL, SEARCH_BAR_HEIGHT_LOGICAL, SHELL_PADDING_LOGICAL,
    card_header_layout, layout_note_toolbar, new_note_button_rect,
};
use super::widget_actions::new_document_menu_action;
use super::{
    CanvasViewCommand, GLOBAL_SEARCH_BOX_ID, NEW_NOTE_BUTTON_ID, NOTE_TOOL_ICON_SIZE_LOGICAL,
    NewNoteControlState, NoteToolbarButtonInput, NotoraRenderModel, NotoraShell, RenderCard,
    SEARCH_ICON_AREA_WIDTH_LOGICAL, SET_WORKSPACE_ROOT_BUTTON_ID, SIDEBAR_ICON_SIZE_LOGICAL,
    add_compact_editor_toolbar_commands, add_source_toggle_command,
    editor_toolbar_input_for_plugin,
};

#[test]
fn inspector_dock_reserves_the_scaled_right_edge_of_the_editor_body() {
    use ui::inspector_panel::PANEL_WIDTH_LOGICAL;

    let layout = ShellLayout {
        dpi: 1.5,
        editor_body_rect: Rect::new(100.0, 150.0, 600.0, 480.0),
        ..ShellLayout::default()
    };
    let (editor, panel) = super::editor_inspector_rects(layout, true);
    assert_eq!(panel.w, PANEL_WIDTH_LOGICAL * layout.dpi);
    assert_eq!(panel.right(), layout.editor_body_rect.right());
    assert_eq!(panel.y, layout.editor_body_rect.y);
    assert_eq!(panel.h, layout.editor_body_rect.h);
    assert_eq!(editor.right(), panel.x);
    assert_eq!(editor.x, layout.editor_body_rect.x);
    assert_eq!(super::editor_inspector_rects(layout, false), (layout.editor_body_rect, Rect::ZERO));

    let narrow = ShellLayout { editor_body_rect: Rect::new(100.0, 150.0, 100.0, 480.0), ..layout };
    let (editor, panel) = super::editor_inspector_rects(narrow, true);
    assert_eq!(editor.w, 0.0);
    assert_eq!(panel, narrow.editor_body_rect);
}

#[test]
fn three_panes_share_rounded_surfaces_with_native_or_immersive_title_bar() {
    let config_directory = tempfile::tempdir().expect("surface test config should be created");
    let paths = crate::NotoraPaths::from_config_directory(config_directory.path())
        .expect("surface test paths should be created");
    let mut app =
        crate::NotoraApp::with_paths(paths).expect("surface test should create a headless app");
    let mut shell = NotoraShell::new();
    let model = NotoraRenderModel::from_state(&crate::NotoraState::default());
    for definition in
        [ui::theme::ThemeDefinition::default_light(), ui::theme::ThemeDefinition::default_dark()]
    {
        let theme = ui::Theme::from_definition(&definition);
        let surfaces = theme.application_theme();
        assert_ne!(surfaces.window_surface, surfaces.navigation_surface);
        assert_ne!(surfaces.navigation_surface, surfaces.editor_surface);
        for dpi in [1.0, 1.25, 1.5, 1.75, 2.0] {
            for frame_state in [
                ui::window_frame::WindowFrameState::Native,
                ui::window_frame::WindowFrameState::MacOs,
                ui::window_frame::WindowFrameState::Restored,
            ] {
                let title_height = frame_state.title_height(dpi);
                let layout = ShellLayout::compute_below_title_bar(
                    crate::shell::layout::ShellLayoutInput {
                        window_width_px: 1200.0 * dpi,
                        window_height_px: 800.0 * dpi,
                        dpi,
                        navigation_width_logical: 220.0,
                        card_list_width_logical: 340.0,
                        navigation_pane_visibility: crate::NavigationPaneVisibility::Expanded,
                        compact_content: crate::CompactContent::CardList,
                        compact_navigation: crate::CompactNavigation::Hidden,
                        editor_property_row_visible: false,
                        editor_header_visible: false,
                    },
                    title_height,
                );
                let runtime = app.runtime_mut().editor_runtime_mut();
                runtime.update_theme(theme.clone());
                runtime.set_scale_factor(f64::from(dpi));
                let mut frame = runtime.begin_frame().expect("surface test frame should begin");
                frame.with_layout_context(|context| {
                    shell.set_canvas_scrollbars_input(
                        Some(CanvasScrollbarsInput {
                            horizontal: None,
                            vertical: Some(ui::scrollbar::ScrollbarInput {
                                viewport_height_px: f64::from(layout.editor_body_rect.h),
                                total_display_rows: 2400,
                                scroll_top_rows: 2400.0,
                            }),
                        }),
                        layout.editor_body_rect,
                        context,
                    );
                });
                shell.render(&mut frame, layout, &model).expect("surface test should render");
                if layout.has_immersive_title_bar() {
                    assert_eq!(shell.navigation_collapse_rect, Rect::ZERO);
                    assert_eq!(shell.navigation_expand_rect, Rect::ZERO);
                    assert_eq!(
                        shell.search_rect.w,
                        layout.navigation_rect.w - SHELL_PADDING_LOGICAL * dpi * 2.0
                    );
                } else {
                    assert_ne!(shell.navigation_collapse_rect, Rect::ZERO);
                }
                frame.with_paint_context(|context| {
                let mut thumb_color = context.theme.editor.scrollbar_thumb;
                thumb_color[3] *= 0.6;
                let scrollbar_index = context.list.cmds.iter().position(|command| matches!(command,
                    ui::DrawCmd::FillRect { rect, color, .. }
                        if *color == thumb_color && rect.right() == layout.editor_rect.right()
                )).expect("editor scrollbar must be part of the content overlay");
                let corner_index = context.list.cmds.iter().position(|command| matches!(command,
                    ui::DrawCmd::TaperedMesh { color, .. } if *color == surfaces.window_surface
                )).expect("content corners must be masked");
                assert!(scrollbar_index < corner_index, "rounded corners must cover scrollbar endpoints");
                assert!(context.list.cmds.iter().any(|command| matches!(command,
                    ui::DrawCmd::StrokeRect { rect, radius, .. }
                        if *rect == layout.content_surface_rect()
                            && *radius == ui::rounded_surface_frame::SHELL_CORNER_RADIUS_LOGICAL * dpi
                )));
            });
                frame.with_underlay_paint_context(|context| {
                let surface = layout.content_surface_rect();
                let corner_probe = (surface.x - dpi * 0.25, surface.y + dpi);
                assert!(!context.list.cmds.iter().any(|command| matches!(command,
                    ui::DrawCmd::FillRect { rect, color, .. }
                        if *color == surfaces.divider && rect.contains(corner_probe.0, corner_probe.1)
                )), "navigation divider must not protrude beside the rounded corner");
                for (expected_rect, expected_color) in [
                    (layout.navigation_rect, surfaces.window_surface),
                    (layout.card_list_rect, surfaces.navigation_surface),
                    (layout.editor_rect, surfaces.editor_surface),
                ] {
                    assert!(context.list.cmds.iter().any(|command| matches!(command,
                        ui::DrawCmd::FillRect { rect, color, .. }
                            if *rect == expected_rect && *color == expected_color
                    )));
                }
            });
            }
        }
    }
}

struct TestClipboard(String);

impl ui::core::Clipboard for TestClipboard {
    fn read_text(&mut self) -> Option<String> {
        Some(self.0.clone())
    }

    fn write_text(&mut self, text: &str) -> bool {
        self.0 = text.to_owned();
        true
    }
}

#[test]
fn lifecycle_events_have_no_pointer_position_and_survive_coordinate_translation() {
    for event in [Event::PointerLeave, Event::InteractionCancel] {
        assert_eq!(event_pointer_position(&event), None);
        assert_eq!(translate_event(&event, 10.0, 20.0), event);
    }
}
use crate::NotoraState;
use crate::action::CardQuery;
use crate::state::CardPageState;
use notora_core::{CatalogCard, DocumentIdentity, DocumentKind, NavigationScope, NoteId};

#[test]
fn encrypted_unlock_is_rendered_inline_instead_of_as_a_product_dialog() {
    let note_id = NoteId::generate();
    let request = crate::action::DocumentLoadRequest {
        identity: DocumentIdentity::Note(note_id),
        selection_generation: 1,
    };
    let mut state = NotoraState::default();
    state.library.selected_card = Some(request.identity);
    state.library.selected_document_generation = request.selection_generation;
    let _ = state.reduce(NotoraAction::EncryptedNoteUnlockRequired {
        request,
        title: "私密笔记".to_owned(),
        metadata: notora_core::NoteEditorMetadata {
            note_id,
            created_at: SystemTime::UNIX_EPOCH,
            modified_at: SystemTime::UNIX_EPOCH,
            encryption: notora_core::NoteEncryption::Encrypted,
            title_initialization: notora_core::TitleInitialization::Independent,
            file_name_binding: notora_core::NoteFileNameBinding::TitleBound { disambiguator: 1 },
            title_revision: 0,
        },
        tags: Vec::new(),
    });

    let model = NotoraRenderModel::from_state(&state);

    assert!(model.encrypted_note_dialog.is_none());
    assert_eq!(
        model.encrypted_note_unlock.as_ref().map(|input| input.title.as_str()),
        Some("私密笔记")
    );
    assert_eq!(state.layout.overlay, OverlayState::None);
}

#[test]
fn collapsed_navigation_does_not_paint_search_over_the_expand_button() {
    let directory = tempfile::tempdir().expect("navigation paint test needs isolated paths");
    let paths = crate::NotoraPaths::from_config_directory(directory.path().join("notora"))
        .expect("navigation paint test paths should be created");
    let mut app = crate::NotoraApp::with_paths(paths)
        .expect("navigation paint test should create a headless app");
    let mut shell = NotoraShell::new();
    let theme = ui::theme::test_theme();
    let model = NotoraRenderModel::from_state(&NotoraState::default());
    for dpi in [1.0, 1.5, 2.0] {
        for visibility in [
            crate::NavigationPaneVisibility::Collapsed,
            crate::NavigationPaneVisibility::Expanded,
            crate::NavigationPaneVisibility::Collapsed,
        ] {
            let layout = ShellLayout::compute(crate::shell::layout::ShellLayoutInput {
                window_width_px: 1000.0 * dpi,
                window_height_px: 600.0 * dpi,
                dpi,
                navigation_width_logical: 220.0,
                card_list_width_logical: 340.0,
                navigation_pane_visibility: visibility,
                compact_content: crate::CompactContent::CardList,
                compact_navigation: crate::CompactNavigation::Hidden,
                editor_property_row_visible: true,
                editor_header_visible: true,
            });
            let runtime = app.runtime_mut().editor_runtime_mut();
            runtime.set_scale_factor(f64::from(dpi));
            let mut frame = runtime.begin_frame().expect("navigation test frame should begin");
            shell.render(&mut frame, layout, &model).expect("navigation frame should render");
            assert_navigation_search_visibility(&mut frame, visibility);
            if visibility == crate::NavigationPaneVisibility::Collapsed {
                let button = shell.navigation_expand_rect;
                let press = Event::MouseDown {
                    px: button.x + button.w * 0.5,
                    py: button.y + button.h * 0.5,
                    button: ui::core::MouseButton::Left,
                };
                let release = Event::MouseUp {
                    px: button.x + button.w * 0.5,
                    py: button.y + button.h * 0.5,
                    button: ui::core::MouseButton::Left,
                };
                assert!(
                    shell
                        .route_event(&press, FocusTarget::CardList, &theme, dpi)
                        .actions
                        .is_empty()
                );
                assert_eq!(
                    shell.route_event(&release, FocusTarget::CardList, &theme, dpi).actions,
                    vec![NotoraAction::NavigationPaneVisibilityToggled]
                );
            }
        }
    }
}

fn assert_navigation_search_visibility(
    frame: &mut EditorFrame,
    visibility: crate::NavigationPaneVisibility,
) {
    frame.with_underlay_paint_context(|context| {
        let mut search_icon = ui::DrawList::new();
        let dpi = context.dpi;
        let size = SIDEBAR_ICON_SIZE_LOGICAL * dpi;
        draw_icon(
            &mut search_icon,
            "search",
            SHELL_PADDING_LOGICAL * dpi + (SEARCH_ICON_AREA_WIDTH_LOGICAL * dpi - size) * 0.5,
            SHELL_PADDING_LOGICAL * dpi + (SEARCH_BAR_HEIGHT_LOGICAL * dpi - size) * 0.5,
            size,
            context.theme.application_theme().text_secondary,
        );
        assert!(!search_icon.cmds.is_empty(), "search icon fixture must contain geometry");
        let search_is_painted = context
            .list
            .cmds
            .windows(search_icon.cmds.len())
            .any(|commands| commands == search_icon.cmds);
        assert_eq!(
            search_is_painted,
            visibility == crate::NavigationPaneVisibility::Expanded,
            "搜索图标应随侧栏显隐，不能叠到展开按钮上：{visibility:?}，DPI={dpi}"
        );
    });
}

#[test]
fn builds_a_static_render_model() {
    let model = NotoraRenderModel::from_state(&NotoraState::default());
    assert_eq!(model.navigation_rows.len(), 5);
    assert_eq!(model.navigation_rows[0].icon.as_deref(), Some("folder-open"));
    assert_eq!(model.navigation_rows[0].label, "工作区");
    assert_eq!(model.navigation_rows[1].label, "星标");
    assert_eq!(model.navigation_rows[2].label, "标签");
    assert_eq!(model.navigation_rows[3].label, "回收站");
    assert_eq!(model.navigation_rows[4].label, "文件");
    assert_eq!(model.card_list_title, "工作区");
    assert!(model.cards.is_empty());
}

#[test]
fn editor_times_use_calendar_dates_and_bounded_relative_labels() {
    let now = UNIX_EPOCH + std::time::Duration::from_secs(4 * 86_400);

    assert_eq!(format_created_time(UNIX_EPOCH), "创建 1970/01/01 UTC");
    assert_eq!(format_modified_time(now, now), "修改 刚刚");
    assert_eq!(
        format_modified_time(now - std::time::Duration::from_secs(5 * 60), now),
        "修改 5 分钟前"
    );
    assert_eq!(
        format_modified_time(now - std::time::Duration::from_secs(2 * 3_600), now),
        "修改 2 小时前"
    );
    assert_eq!(
        format_modified_time(now - std::time::Duration::from_secs(3 * 86_400), now),
        "修改 3 天前"
    );
    assert_eq!(format_modified_time(now + std::time::Duration::from_secs(60), now), "修改 刚刚");
}

#[test]
fn catalog_modified_timestamp_uses_the_same_relative_label_as_the_editor() {
    let now = UNIX_EPOCH + std::time::Duration::from_secs(10 * 60);
    let modified_nanoseconds = 5 * 60 * 1_000_000_000;

    assert_eq!(format_modified_timestamp_at(modified_nanoseconds, now), "修改 5 分钟前");
}

#[test]
fn document_kinds_use_icons_registered_by_the_ui_renderer() {
    assert_eq!(document_icon(DocumentKind::Text), "notebook-text");
    assert_eq!(document_icon(DocumentKind::Markdown), "file-code-corner");
    assert_eq!(document_icon(DocumentKind::Mindmap), "workflow");
}

#[test]
fn new_note_control_is_hidden_for_non_creation_scopes() {
    for scope in [
        NavigationScope::Search { query: "roadmap".to_owned() },
        NavigationScope::Starred,
        NavigationScope::Trash,
        NavigationScope::Tag { tag_id: notora_core::TagId::generate() },
    ] {
        let mut state = NotoraState::default();
        state.library.navigation_scope = scope;

        assert_eq!(
            NotoraRenderModel::from_state(&state).new_note_control,
            NewNoteControlState::Hidden
        );
    }
}

#[test]
fn missing_workspace_disables_new_note_and_exposes_a_separate_root_action() {
    let model = NotoraRenderModel::from_state(&NotoraState::default());

    assert_eq!(model.new_note_control, NewNoteControlState::Disabled);
    assert_eq!(model.card_empty_state.title, "尚未设置工作区根目录");
    assert_eq!(model.card_empty_state.action_label.as_deref(), Some("设置根目录"));

    let mut shell = NotoraShell::new();
    shell.update_model(&model);
    assert_eq!(
        shell.translate_widget_action(&WidgetAction::Control(ControlAction::Activated {
            id: SET_WORKSPACE_ROOT_BUTTON_ID,
        })),
        Some(NotoraAction::WorkspaceRootSelectionRequested)
    );
}

#[test]
fn new_note_control_is_laid_out_inside_the_middle_column() {
    let navigation_rect = Rect::new(0.0, 0.0, 220.0, 600.0);
    let card_list_rect = Rect::new(228.0, 0.0, 340.0, 600.0);

    let header = card_header_layout(
        card_list_rect,
        1.0,
        "文件",
        NewNoteControlState::Enabled,
        0,
        Rect::ZERO,
    );
    let button_rect = new_note_button_rect(header.toolbar_rect, 1.0, NewNoteControlState::Enabled);

    assert!(button_rect != Rect::ZERO);
    assert!(card_list_rect.contains(button_rect.x, button_rect.y));
    assert!(card_list_rect.contains(button_rect.right(), button_rect.bottom()));
    assert!(!navigation_rect.contains(button_rect.x, button_rect.y));
    assert_eq!(
        new_note_button_rect(header.toolbar_rect, 1.0, NewNoteControlState::Hidden,),
        Rect::ZERO
    );
}

#[test]
fn wrapped_file_toolbar_is_centered_across_column_widths_and_dpi() {
    let buttons = note_toolbar_buttons(&NavigationScope::ExternalFiles, None, true);
    for dpi in [1.0, 1.25, 1.5, 2.0] {
        for width in [252.0, 260.0, 268.0] {
            let column = Rect::new(228.0 * dpi, 20.0 * dpi, width * dpi, 600.0 * dpi);
            let header = card_header_layout(
                column,
                dpi,
                "文件",
                NewNoteControlState::EnabledForFiles,
                buttons.len(),
                Rect::ZERO,
            );
            let new_rect = new_note_button_rect(
                header.toolbar_rect,
                dpi,
                NewNoteControlState::EnabledForFiles,
            );
            let toolbar = layout_note_toolbar(header.toolbar_rect, dpi, new_rect, &buttons);
            let left = toolbar.iter().map(|button| button.rect.x).fold(new_rect.x, f32::min);
            let left_margin = left - column.x;
            let right_margin = column.right() - new_rect.right();
            assert!(
                (left_margin - right_margin).abs() < f32::EPSILON,
                "独占一行的工具栏应居中：栏宽={width}，DPI={dpi}，左留白={left_margin}，右留白={right_margin}"
            );
        }
    }
}

#[test]
fn narrow_card_header_keeps_the_title_clear_of_toolbar_buttons() {
    let card_list_rect = Rect::new(228.0, 0.0, 268.0, 600.0);
    let padding = SHELL_PADDING_LOGICAL;
    let header = card_header_layout(
        card_list_rect,
        1.0,
        "一个很长的中栏标题",
        NewNoteControlState::Enabled,
        1,
        Rect::ZERO,
    );
    let new_note_rect =
        new_note_button_rect(header.toolbar_rect, 1.0, NewNoteControlState::Enabled);
    let toolbar = layout_note_toolbar(
        header.toolbar_rect,
        1.0,
        new_note_rect,
        &[NoteToolbarButtonInput {
            icon: None,
            label: "操作".to_owned(),
            action: NotoraAction::SettingsViewChanged,
        }],
    );
    let title_right = card_list_rect.x
        + padding
        + ui::core::text_util::estimate_text_width_px("一个很长的中栏标题", 16.0);
    let title_bottom = card_list_rect.y + 36.0;
    let first_button = &toolbar[0].rect;

    assert!(
        first_button.x >= title_right + NOTE_TOOL_BUTTON_GAP_LOGICAL
            || first_button.y >= title_bottom
    );
    assert!(header.content_top_y >= first_button.bottom());
}

#[test]
fn collapsed_navigation_keeps_card_header_title_clear_of_expand_button() {
    let card_list_rect = Rect::new(0.0, 0.0, 340.0, 600.0);
    let expand_rect = Rect::new(12.0, 8.0, 28.0, 28.0);
    let header = card_header_layout(
        card_list_rect,
        1.0,
        "笔记区",
        NewNoteControlState::Enabled,
        0,
        expand_rect,
    );

    assert!(header.title_x >= expand_rect.right() + NOTE_TOOL_BUTTON_GAP_LOGICAL);
}

#[test]
fn files_scope_offers_creation_without_a_workspace() {
    let mut state = NotoraState::default();
    state.library.navigation_scope = NavigationScope::ExternalFiles;
    let model = NotoraRenderModel::from_state(&state);
    assert!(model.new_note_control.is_enabled());
    assert_eq!(model.card_empty_state.title, "暂无文件");
    assert!(model.card_empty_state.action_id.is_none());
}

#[test]
fn files_toolbar_exposes_open_external_file_action() {
    let mut state = NotoraState::default();
    state.library.navigation_scope = NavigationScope::ExternalFiles;

    let model = NotoraRenderModel::from_state(&state);

    assert_eq!(
        model.note_toolbar,
        vec![NoteToolbarButtonInput {
            icon: Some("folder-open"),
            label: "打开".to_owned(),
            action: NotoraAction::OpenExternalFileDialogRequested,
        }]
    );
}

#[test]
fn files_toolbar_exposes_clear_all_when_external_records_exist() {
    let mut state = NotoraState::default();
    state.library.navigation_scope = NavigationScope::ExternalFiles;
    let _ = state.external_files.create_untitled(DocumentKind::Markdown);

    let model = NotoraRenderModel::from_state(&state);

    assert_eq!(
        model.note_toolbar,
        vec![
            NoteToolbarButtonInput {
                icon: Some("folder-open"),
                label: "打开".to_owned(),
                action: NotoraAction::OpenExternalFileDialogRequested,
            },
            NoteToolbarButtonInput {
                icon: Some("x"),
                label: "清空".to_owned(),
                action: NotoraAction::ExternalFilesClearRequested,
            },
        ]
    );
}

#[test]
fn search_toolbar_exposes_clear_without_a_new_note_control() {
    let mut state = NotoraState::default();
    state.library.navigation_scope = NavigationScope::Search { query: "roadmap".to_owned() };

    let model = NotoraRenderModel::from_state(&state);

    assert_eq!(model.new_note_control, NewNoteControlState::Hidden);
    assert_eq!(
        model.note_toolbar,
        vec![NoteToolbarButtonInput {
            icon: None,
            label: "清除".to_owned(),
            action: NotoraAction::SearchCommitted { query: String::new(), search_generation: None },
        }]
    );
}

#[test]
fn file_actions_share_one_toolbar_surface_and_outline() {
    use ui::core::paint::{DrawCmd, DrawList};

    let mut shaper = shaping::Shaper::new().expect("toolbar appearance test requires fonts");
    for (mode, system_theme) in [
        (ui::settings::ThemeMode::Light, winit::window::Theme::Light),
        (ui::settings::ThemeMode::Dark, winit::window::Theme::Dark),
    ] {
        let theme = ui::Theme::resolve_builtin(mode, system_theme);
        for dpi in [1.0, 1.5, 2.0] {
            for alpha in [1.0, 0.5] {
                let mut shell = NotoraShell::new();
                shell
                    .new_note_button
                    .set_input(SplitButtonInput { label: "新建".to_owned(), enabled: true });
                let card_rect = Rect::new(0.0, 0.0, 320.0 * dpi, 200.0 * dpi);
                let buttons = note_toolbar_buttons(&NavigationScope::ExternalFiles, None, true);
                let header = card_header_layout(
                    card_rect,
                    dpi,
                    "文件",
                    NewNoteControlState::EnabledForFiles,
                    buttons.len(),
                    Rect::ZERO,
                );
                let new_rect = new_note_button_rect(
                    header.toolbar_rect,
                    dpi,
                    NewNoteControlState::EnabledForFiles,
                );
                let mut measure = ui::NoopMeasure;
                let mut layout_context =
                    ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi };
                shell.new_note_button.set_rect(new_rect, &mut layout_context);
                shell.note_toolbar_buttons =
                    layout_note_toolbar(header.toolbar_rect, dpi, new_rect, &buttons);
                shell.synchronize_chrome_buttons(None, &mut layout_context);
                let mut actual = DrawList::new();
                let mut context = ui::PaintCtx::new(&mut actual, &theme, dpi);
                context.global_alpha = alpha;
                context.shaper = Some(&mut shaper);
                shell.paint_note_toolbar(&mut context);
                let first_x = shell
                    .note_toolbar_buttons
                    .iter()
                    .map(|button| button.rect.x)
                    .fold(new_rect.x, f32::min);
                let toolbar_rect =
                    Rect::new(first_x, new_rect.y, new_rect.right() - first_x, new_rect.h);
                let outlines: Vec<_> = actual
                    .cmds
                    .iter()
                    .filter_map(|command| match command {
                        DrawCmd::StrokeRect { rect, .. } => Some(*rect),
                        _ => None,
                    })
                    .collect();
                assert_eq!(outlines, vec![toolbar_rect], "文件操作应只绘制一个工具栏外框");
                let style = ButtonStyle::from_theme(&theme);
                let foreground = style.foreground_color(ButtonVisualState::Normal, alpha);
                let labels: Vec<_> = actual
                    .cmds
                    .iter()
                    .filter_map(|command| match command {
                        DrawCmd::TextLayout { layout, color, .. } => {
                            assert_eq!(*color, foreground, "工具栏标签必须保留主题色与透明度");
                            Some(layout.text.as_str())
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(labels.len(), 3);
                for label in ["新建", "打开", "清空"] {
                    assert!(labels.contains(&label), "工具栏必须保留{label}入口");
                }
            }
        }
    }
}

#[test]
fn file_toolbar_labels_match_new_button_typography_at_each_dpi() {
    use ui::core::paint::{DrawCmd, DrawList};

    let theme = ui::theme::test_theme();
    let mut shaper = shaping::Shaper::new().expect("toolbar text test requires a shaper");
    for dpi in [1.0, 1.5, 2.0] {
        let mut measure = ui::NoopMeasure;
        let mut layout_context =
            ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi };
        let mut new_button = SplitButtonWidget::new();
        new_button.set_input(SplitButtonInput { label: "新建".to_owned(), enabled: true });
        new_button.set_rect(
            Rect::new(
                0.0,
                0.0,
                NEW_NOTE_BUTTON_WIDTH_LOGICAL * dpi,
                NOTE_TOOL_BUTTON_HEIGHT_LOGICAL * dpi,
            ),
            &mut layout_context,
        );
        let mut draw_list = DrawList::new();
        let mut paint_context = ui::PaintCtx::new(&mut draw_list, &theme, dpi);
        paint_context.shaper = Some(&mut shaper);
        new_button.paint(&mut paint_context);
        for (index, input) in
            note_toolbar_buttons(&NavigationScope::ExternalFiles, None, true).iter().enumerate()
        {
            let mut button = ui::button::Button::new(
                WidgetId(index as u64),
                ButtonStyle::from_theme(&theme).toolbar_item(),
            );
            button.set_text(Some(input.label.clone()));
            button.set_icon(input.icon.map(str::to_owned));
            button.set_icon_size(NOTE_TOOL_ICON_SIZE_LOGICAL);
            button.set_rect(
                Rect::new(
                    0.0,
                    0.0,
                    NOTE_TOOL_BUTTON_WIDTH_LOGICAL * dpi,
                    NOTE_TOOL_BUTTON_HEIGHT_LOGICAL * dpi,
                ),
                &mut layout_context,
            );
            button.paint(&mut paint_context);
        }

        let typography: Vec<_> = draw_list
            .cmds
            .iter()
            .filter_map(|command| match command {
                DrawCmd::TextLayout { layout, y_baseline, .. } => {
                    Some((layout.font_size, *y_baseline))
                }
                _ => None,
            })
            .collect();
        assert_eq!(typography.len(), 3, "应绘制新建、打开和清空三个标签");
        for actual in &typography[1..] {
            assert_eq!(*actual, typography[0], "同一工具栏的字号与基线应一致，DPI={dpi}");
        }
    }
}

#[test]
fn search_box_focus_request_paints_the_caret_after_state_sync() {
    use ui::core::paint::{DrawCmd, DrawList};

    let mut shell = NotoraShell::new();
    let theme = ui::theme::test_theme();
    let mut measure = ui::NoopMeasure;
    let mut layout_context =
        ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
    shell.search_rect = Rect::new(8.0, 8.0, 240.0, 32.0);
    shell.search_box.set_rect(shell.search_rect, &mut layout_context);
    let click = Event::MouseDown { px: 24.0, py: 24.0, button: ui::core::MouseButton::Left };

    let route = shell.route_event(&click, FocusTarget::Editor, &theme, 1.0);
    assert_eq!(route.actions, vec![NotoraAction::FocusRequested(FocusTarget::NavigationSearch)]);
    shell.synchronize_focus(FocusTarget::NavigationSearch, Instant::now());
    assert!(shell.search_box.is_focused());

    let mut draw_list = DrawList::new();
    let mut paint_context = ui::PaintCtx::new(&mut draw_list, &theme, 1.0);
    shell.search_box.paint(&mut paint_context);

    assert!(
        draw_list.cmds.iter().any(|command| {
            matches!(command, DrawCmd::FillRect { radius, .. } if *radius == 0.0)
        })
    );
}

#[test]
fn notora_shell_routes_context_clipboard_to_search_and_editor_chrome() {
    let mut shell = NotoraShell::new();
    let theme = ui::theme::test_theme();
    let mut clipboard = TestClipboard("剪贴板内容".to_owned());
    let mut event_context = EventCtx::with_clipboard(&theme, 1.0, &mut clipboard);
    let command = ui::core::Modifiers { cmd: true, ..ui::core::Modifiers::NONE };

    shell.synchronize_focus(FocusTarget::NavigationSearch, Instant::now());
    let search_route = shell.route_event_with_context(
        &Event::KeyDown(ui::KeyCode::Char('v'), command),
        FocusTarget::NavigationSearch,
        None,
        &mut event_context,
    );
    assert_eq!(
        search_route.actions,
        vec![NotoraAction::SearchTextChanged("剪贴板内容".to_owned())]
    );

    shell.editor_pane.set_input(EditorPaneInput {
        mode: EditorPaneMode::WorkspaceNote,
        header: ui::editor_header::EditorHeaderInput {
            title: "旧标题".to_owned(),
            title_editable: true,
            ..ui::editor_header::EditorHeaderInput::default()
        },
        ..EditorPaneInput::default()
    });
    shell.synchronize_focus(FocusTarget::EditorTitle, Instant::now());
    let title_route = shell.route_event_with_context(
        &Event::KeyDown(ui::KeyCode::Char('v'), command),
        FocusTarget::EditorTitle,
        None,
        &mut event_context,
    );
    assert_eq!(shell.editor_title_text(), "剪贴板内容旧标题");
    assert_eq!(
        title_route.actions,
        vec![NotoraAction::TitleTextChanged("剪贴板内容旧标题".to_owned())]
    );
}

#[test]
fn focused_title_exposes_a_window_space_ime_cursor_rect() {
    let mut shell = NotoraShell::new();
    let editor_input = EditorPaneInput {
        mode: EditorPaneMode::WorkspaceNote,
        header: ui::editor_header::EditorHeaderInput {
            title_editable: true,
            ..ui::editor_header::EditorHeaderInput::default()
        },
        ..EditorPaneInput::default()
    };
    shell.editor_pane.set_input(editor_input);
    let theme = ui::theme::test_theme();
    let mut measure = ui::NoopMeasure;
    let mut layout_context =
        ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
    shell.editor_pane.set_rects(
        EditorPaneRects {
            header: Rect::new(420.0, 48.0, 640.0, 108.0),
            toolbar: Rect::new(420.0, 156.0, 640.0, 40.0),
            body: Rect::new(420.0, 196.0, 640.0, 400.0),
        },
        &mut layout_context,
    );
    shell.synchronize_focus(FocusTarget::EditorTitle, Instant::now());

    let ime_rect = shell
        .focused_text_input_ime_cursor_rect()
        .expect("focused title must provide an IME candidate anchor");

    assert!(ime_rect.x >= 420.0 + 16.0);
    assert!(ime_rect.y >= 48.0);
}

#[test]
fn focused_tag_exposes_an_ime_cursor_rect_until_focus_moves_away() {
    let mut shell = NotoraShell::new();
    shell.editor_pane.set_input(EditorPaneInput {
        mode: EditorPaneMode::WorkspaceNote,
        tags: ui::tag_editor::TagEditorInput {
            enabled: true,
            ..ui::tag_editor::TagEditorInput::default()
        },
        ..EditorPaneInput::default()
    });
    let theme = ui::theme::test_theme();
    let mut measure = ui::NoopMeasure;
    let mut layout_context =
        ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
    shell.editor_pane.set_rects(
        EditorPaneRects {
            header: Rect::new(420.0, 48.0, 640.0, 108.0),
            toolbar: Rect::new(420.0, 156.0, 640.0, 40.0),
            body: Rect::new(420.0, 196.0, 640.0, 400.0),
        },
        &mut layout_context,
    );

    shell.synchronize_focus(FocusTarget::EditorTag, Instant::now());
    let ime_rect = shell
        .focused_text_input_ime_cursor_rect()
        .expect("focused tag should provide an IME candidate anchor");
    assert!(ime_rect.x >= 420.0);
    assert!(ime_rect.y >= 48.0);

    shell.synchronize_focus(FocusTarget::NavigationTree, Instant::now());
    assert!(!shell.editor_pane.tag_editor_has_keyboard_focus());
    assert!(shell.focused_text_input_ime_cursor_rect().is_none());
}

#[test]
fn encrypted_password_inputs_supply_blink_deadlines_and_ime_policy() {
    use ui::core::paint::{DrawCmd, DrawList};

    let theme = ui::theme::test_theme();
    let mut measure = ui::NoopMeasure;
    let mut layout_context =
        ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
    for (focus, confirmation) in
        [(FocusTarget::Overlay, false), (FocusTarget::Overlay, true), (FocusTarget::Editor, false)]
    {
        let mut shell = NotoraShell::new();
        let mut dialog = EncryptedNoteDialog::new(&theme);
        dialog.set_input(EncryptedNoteDialogInput::create(), true);
        dialog.set_rect(Rect::new(0.0, 0.0, 800.0, 600.0), &mut layout_context);
        if confirmation {
            dialog.route_event(
                &Event::KeyDown(ui::KeyCode::Tab, ui::core::Modifiers::NONE),
                &mut EventCtx::new(&theme, 1.0),
            );
        }
        shell.encrypted_note_dialog = Some(dialog);
        shell.encrypted_note_dialog_open = true;
        let mut unlock = EncryptedNoteUnlock::new(&theme);
        unlock.set_input(EncryptedNoteUnlockInput::new(String::new()), true);
        unlock.set_rect(Rect::new(0.0, 0.0, 800.0, 600.0), &mut layout_context);
        shell.encrypted_note_unlock = Some(unlock);
        shell.encrypted_note_unlock_open = true;
        let focused_at = Instant::now();
        shell.synchronize_focus(focus, focused_at);
        assert!(!shell.focused_text_input_ime_allowed());

        for (phase, visible) in [true, false, true].into_iter().enumerate() {
            if phase > 0 {
                assert!(shell.advance_text_cursor_blink(
                    shell.next_text_cursor_blink_at().expect("focused input schedules blinking")
                ));
            }
            let mut draw_list = DrawList::new();
            let mut context = ui::PaintCtx::new(&mut draw_list, &theme, 1.0);
            if focus == FocusTarget::Overlay {
                shell.encrypted_note_dialog.as_ref().expect("dialog exists").paint(&mut context);
            } else {
                shell.encrypted_note_unlock.as_ref().expect("unlock exists").paint(&mut context);
            }
            let paints_caret = draw_list.cmds.iter().any(|command| {
                matches!(command, DrawCmd::FillRect { rect, radius, .. }
                    if *radius == 0.0 && rect.w == 2.0)
            });
            assert_eq!(paints_caret, visible, "focus={focus:?}, phase={phase}");
        }
        shell.synchronize_focus(FocusTarget::NavigationSearch, Instant::now());
        assert!(shell.focused_text_input_ime_allowed());
    }
}

#[test]
fn focused_search_box_toggles_the_caret_every_blink_interval() {
    use ui::core::paint::{DrawCmd, DrawList};

    fn paints_caret(shell: &NotoraShell, theme: &ui::Theme) -> bool {
        let mut draw_list = DrawList::new();
        let mut paint_context = ui::PaintCtx::new(&mut draw_list, theme, 1.0);
        shell.search_box.paint(&mut paint_context);
        draw_list
            .cmds
            .iter()
            .any(|command| matches!(command, DrawCmd::FillRect { radius, .. } if *radius == 0.0))
    }

    let mut shell = NotoraShell::new();
    let theme = ui::theme::test_theme();
    let mut measure = ui::NoopMeasure;
    let mut layout_context =
        ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
    shell.search_box.set_rect(Rect::new(8.0, 8.0, 240.0, 32.0), &mut layout_context);
    let focused_at = Instant::now();

    shell.synchronize_focus(FocusTarget::NavigationSearch, focused_at);
    assert_eq!(shell.next_text_cursor_blink_at(), shell.search_box.next_cursor_blink_at());
    let first_deadline =
        shell.next_text_cursor_blink_at().expect("focused search schedules blinking");
    assert!(first_deadline >= focused_at);
    assert!(paints_caret(&shell, &theme));

    assert!(shell.advance_text_cursor_blink(first_deadline));
    assert!(!paints_caret(&shell, &theme));

    let second_deadline = shell.next_text_cursor_blink_at().expect("blink schedules next phase");
    assert!(second_deadline > first_deadline);
    assert!(shell.advance_text_cursor_blink(second_deadline));
    assert!(paints_caret(&shell, &theme));
}

#[test]
fn direction_keys_do_not_resize_splitters_without_splitter_focus() {
    let mut shell = NotoraShell::new();
    shell.navigation_splitter.set_input(SplitterInput {
        logical_position: 220.0,
        minimum_logical_position: 180.0,
        maximum_logical_position: 320.0,
        enabled: true,
    });
    shell.card_list_splitter.set_input(SplitterInput {
        logical_position: 340.0,
        minimum_logical_position: 280.0,
        maximum_logical_position: 480.0,
        enabled: true,
    });
    let theme = ui::theme::test_theme();

    let route = shell.route_event(
        &Event::KeyDown(ui::KeyCode::Left, ui::core::Modifiers::NONE),
        FocusTarget::Editor,
        &theme,
        1.0,
    );

    assert!(!route.consumed);
    assert!(route.actions.is_empty());
    assert_eq!(shell.navigation_splitter.logical_position(), 220.0);
    assert_eq!(shell.card_list_splitter.logical_position(), 340.0);
}

#[test]
fn confirmation_overlay_blocks_keyboard_and_background_pointer_input() {
    let mut shell = NotoraShell::new();
    shell.navigation_splitter.set_input(SplitterInput {
        logical_position: 220.0,
        minimum_logical_position: 180.0,
        maximum_logical_position: 320.0,
        enabled: true,
    });
    shell.card_list_splitter.set_input(SplitterInput {
        logical_position: 340.0,
        minimum_logical_position: 280.0,
        maximum_logical_position: 480.0,
        enabled: true,
    });
    shell.confirmation_action = Some(NotoraAction::OverlayDismissed);
    shell.confirmation_panel_rect = Rect::new(200.0, 100.0, 400.0, 240.0);
    let theme = ui::theme::test_theme();

    let keyboard_route = shell.route_event_with_overlay(
        &Event::KeyDown(ui::KeyCode::Right, ui::core::Modifiers::NONE),
        FocusTarget::Editor,
        OverlayState::TrashPermanentDeletionConfirmation {
            operation: crate::action::TrashOperation::Empty,
        },
        &theme,
        1.0,
    );
    let pointer_route = shell.route_event_with_overlay(
        &Event::MouseDown { px: 300.0, py: 180.0, button: ui::MouseButton::Left },
        FocusTarget::Editor,
        OverlayState::TrashPermanentDeletionConfirmation {
            operation: crate::action::TrashOperation::Empty,
        },
        &theme,
        1.0,
    );

    assert!(keyboard_route.consumed);
    assert!(pointer_route.consumed);
    assert_eq!(shell.navigation_splitter.logical_position(), 220.0);
    assert_eq!(shell.card_list_splitter.logical_position(), 340.0);
}

#[test]
fn new_document_menu_blocks_compact_layout_controls() {
    let mut shell = NotoraShell::new();
    shell.new_document_menu_open = true;
    shell.compact_navigation_rect = Rect::new(8.0, 8.0, 40.0, 40.0);
    let theme = ui::theme::test_theme();

    let route = shell.route_event_with_overlay(
        &Event::MouseDown { px: 24.0, py: 24.0, button: ui::MouseButton::Left },
        FocusTarget::Overlay,
        OverlayState::NewDocumentMenu,
        &theme,
        1.0,
    );

    assert!(route.consumed);
    assert!(route.actions.is_empty());
}

#[test]
fn navigation_pane_controls_dispatch_the_same_visibility_toggle() {
    let collapse_rect = Rect::new(180.0, 12.0, 28.0, 28.0);
    let expand_rect = Rect::new(12.0, 8.0, 28.0, 28.0);
    let theme = ui::theme::test_theme();

    for (px, py) in [(194.0, 26.0), (26.0, 22.0)] {
        let mut shell = NotoraShell::new();
        shell.navigation_collapse_rect = collapse_rect;
        shell.navigation_expand_rect = expand_rect;
        let mut measure = ui::NoopMeasure;
        let mut layout_context =
            ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        shell.synchronize_chrome_buttons(None, &mut layout_context);
        let press = shell.route_event(
            &Event::MouseDown { px, py, button: ui::MouseButton::Left },
            FocusTarget::CardList,
            &theme,
            1.0,
        );
        assert!(press.actions.is_empty());
        let route = shell.route_event(
            &Event::MouseUp { px, py, button: ui::MouseButton::Left },
            FocusTarget::CardList,
            &theme,
            1.0,
        );

        assert_eq!(route.actions, vec![NotoraAction::NavigationPaneVisibilityToggled]);
    }
}

#[test]
fn editor_popup_blocks_canvas_scrollbar_pointer_input() {
    use ui::canvas_scrollbars::CanvasScrollbarsInput;
    use ui::scrollbar::ScrollbarInput;

    let mut shell = NotoraShell::new();
    shell.editor_pane.set_input(EditorPaneInput {
        mode: EditorPaneMode::WorkspaceNote,
        tags: ui::tag_editor::TagEditorInput {
            enabled: true,
            suggestions_open: true,
            ..ui::tag_editor::TagEditorInput::default()
        },
        ..EditorPaneInput::default()
    });
    let theme = ui::theme::test_theme();
    let mut measure = ui::NoopMeasure;
    let mut layout_context =
        ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
    let editor_rect = Rect::new(200.0, 80.0, 800.0, 600.0);
    shell.set_canvas_scrollbars_input(
        Some(CanvasScrollbarsInput {
            horizontal: None,
            vertical: Some(ScrollbarInput {
                viewport_height_px: 600.0,
                total_display_rows: 2_400,
                scroll_top_rows: 0.0,
            }),
        }),
        editor_rect,
        &mut layout_context,
    );

    let route = shell.route_event(
        &Event::MouseDown {
            px: editor_rect.right() - 2.0,
            py: editor_rect.y + 24.0,
            button: ui::MouseButton::Left,
        },
        FocusTarget::EditorTag,
        &theme,
        1.0,
    );

    assert!(route.consumed);
    assert_eq!(route.canvas_scrollbar_action, None);
}

#[test]
fn canvas_scrollbar_captures_drag_and_reports_axis_action() {
    use ui::canvas::CanvasAxis;
    use ui::canvas_scrollbars::CanvasScrollbarsInput;
    use ui::scrollbar::{ScrollbarAction, ScrollbarInput};

    let mut shell = NotoraShell::new();
    let theme = ui::theme::test_theme();
    let mut measure = ui::NoopMeasure;
    let mut layout_context =
        ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
    let editor_rect = Rect::new(200.0, 80.0, 800.0, 600.0);
    shell.set_canvas_scrollbars_input(
        Some(CanvasScrollbarsInput {
            horizontal: None,
            vertical: Some(ScrollbarInput {
                viewport_height_px: 600.0,
                total_display_rows: 2_400,
                scroll_top_rows: 0.0,
            }),
        }),
        editor_rect,
        &mut layout_context,
    );

    let hover = Event::MouseMove { px: editor_rect.right() - 2.0, py: editor_rect.y + 24.0 };
    let route = shell.route_event(&hover, FocusTarget::Editor, &theme, 1.0);
    assert_eq!(route.cursor_hint, Some(winit::window::CursorIcon::Default));

    let press = Event::MouseDown {
        px: editor_rect.right() - 2.0,
        py: editor_rect.y + 24.0,
        button: ui::MouseButton::Left,
    };
    let route = shell.route_event(&press, FocusTarget::Editor, &theme, 1.0);

    assert!(route.consumed);
    assert_eq!(
        route.canvas_scrollbar_action,
        Some(ui::canvas_scrollbars::CanvasScrollbarsAction {
            axis: CanvasAxis::Vertical,
            action: ScrollbarAction::StartDrag,
        })
    );

    let release = Event::MouseUp {
        px: editor_rect.x - 200.0,
        py: editor_rect.bottom() + 200.0,
        button: ui::MouseButton::Left,
    };
    let route = shell.route_event(&release, FocusTarget::Editor, &theme, 1.0);
    assert_eq!(
        route.canvas_scrollbar_action,
        Some(ui::canvas_scrollbars::CanvasScrollbarsAction {
            axis: CanvasAxis::Vertical,
            action: ScrollbarAction::EndDrag,
        })
    );

    let wheel = Event::Wheel {
        dx: 0.0,
        dy: -40.0,
        px: editor_rect.right() - 2.0,
        py: editor_rect.y + 120.0,
    };
    let route = shell.route_event(&wheel, FocusTarget::Editor, &theme, 1.0);
    assert!(!route.consumed, "wheel over the track must fall through to canvas panning");
}

#[test]
fn canvas_scrollbar_paints_over_the_editor_surface() {
    use ui::canvas_scrollbars::CanvasScrollbarsInput;
    use ui::scrollbar::ScrollbarInput;

    let mut shell = NotoraShell::new();
    let theme = ui::theme::test_theme();
    let mut measure = ui::NoopMeasure;
    let mut layout_context =
        ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
    let editor_rect = Rect::new(200.0, 80.0, 800.0, 600.0);
    shell.set_canvas_scrollbars_input(
        Some(CanvasScrollbarsInput {
            horizontal: Some(ScrollbarInput {
                viewport_height_px: 800.0,
                total_display_rows: 3_200,
                scroll_top_rows: 0.0,
            }),
            vertical: None,
        }),
        editor_rect,
        &mut layout_context,
    );
    let mut draw_list = ui::DrawList::new();
    let mut paint_context = ui::PaintCtx::new(&mut draw_list, &theme, 1.0);

    shell.paint_canvas_scrollbars(&mut paint_context);

    assert!(!draw_list.cmds.is_empty());
}

#[test]
fn new_document_menu_exposes_only_direct_creation_actions() {
    let mut state =
        NotoraState { workspace_root: WorkspaceRootState::Active, ..NotoraState::default() };
    state.library.navigation_tree.directories = vec!["plans".into()];
    let _ = state.reduce(NotoraAction::OpenNewDocumentMenu);

    let model = NotoraRenderModel::from_state(&state);

    assert!(model.show_new_document_menu);

    let metrics = ui::settings::UiMetrics::from_settings(&ui::settings::Settings::new(), 1.0);
    let button_rect = Rect::new(300.0, 20.0, 128.0, 28.0);
    let menu = ui::sidebar::build_new_document_menu(button_rect, (800.0, 600.0), &metrics, true);
    let labels: Vec<&str> = menu.items.iter().map(|item| item.label.as_str()).collect();

    assert_eq!(labels, vec!["新建 TXT", "新建 MMAP", "新建 MD", "加密笔记"]);
    assert!(menu.menu_rect.y > button_rect.bottom());
}

#[test]
fn new_document_menu_hover_survives_the_following_render_synchronization() {
    use ui::core::paint::{DrawCmd, DrawList};

    fn build_menu() -> ui::popup_menu::PopupMenu {
        let settings = ui::settings::Settings::new();
        let metrics = ui::settings::UiMetrics::from_settings(&settings, 1.0);
        ui::sidebar::build_new_document_menu(
            Rect::new(300.0, 20.0, 128.0, 28.0),
            (800.0, 600.0),
            &metrics,
            true,
        )
    }

    fn paints_second_item_hover(shell: &NotoraShell, theme: &ui::Theme) -> bool {
        let menu = shell.new_document_menu.as_ref().expect("new document menu should exist");
        let expected_rect = menu.menu().item_rects[1].shrink(1.0, 1.0, 1.0, 1.0);
        let mut draw_list = DrawList::new();
        let mut paint_context = ui::PaintCtx::new(&mut draw_list, theme, 1.0);
        menu.paint(&mut paint_context);
        draw_list.cmds.iter().any(|command| {
            matches!(
                command,
                DrawCmd::FillRect { rect, color, .. }
                    if *rect == expected_rect && *color == theme.application_theme().button_hover_surface
            )
        })
    }

    let mut shell = NotoraShell::new();
    let theme = ui::theme::test_theme();
    shell.synchronize_new_document_menu(build_menu());
    let menu_rect = shell.new_document_menu_rect;
    let second_item_rect =
        shell.new_document_menu.as_ref().expect("new document menu should exist").menu().item_rects
            [1];
    let mut event_context = EventCtx::new(&theme, 1.0);

    shell.route_new_document_menu_event(
        &Event::MouseMove {
            px: menu_rect.x + second_item_rect.x + second_item_rect.w * 0.5,
            py: menu_rect.y + second_item_rect.y + second_item_rect.h * 0.5,
        },
        &mut event_context,
    );
    assert!(paints_second_item_hover(&shell, &theme));

    shell.synchronize_new_document_menu(build_menu());

    assert!(paints_second_item_hover(&shell, &theme));
}

#[test]
fn new_button_uses_plain_text_for_files_and_markdown_for_workspace() {
    let mut shell = NotoraShell::new();
    let mut state =
        NotoraState { workspace_root: WorkspaceRootState::Active, ..NotoraState::default() };
    for (scope, expected_kind) in [
        (NavigationScope::ExternalFiles, DocumentKind::Text),
        (NavigationScope::WorkspaceRoot, DocumentKind::Markdown),
        (NavigationScope::ExternalFiles, DocumentKind::Text),
    ] {
        state.library.navigation_scope = scope;
        shell.update_model(&NotoraRenderModel::from_state(&state));
        assert_eq!(
            shell.translate_widget_action(&WidgetAction::Control(ControlAction::Activated {
                id: NEW_NOTE_BUTTON_ID,
            })),
            Some(NotoraAction::CreateRequested(expected_kind))
        );
    }
}

#[test]
fn new_note_main_button_creates_markdown_directly() {
    let shell = NotoraShell::new();

    assert_eq!(
        shell.translate_widget_action(&WidgetAction::Control(ControlAction::Activated {
            id: NEW_NOTE_BUTTON_ID,
        })),
        Some(NotoraAction::CreateRequested(DocumentKind::Markdown))
    );
}

#[test]
fn popup_menu_actions_map_to_typed_creation_requests() {
    for (kind, expected) in [
        (NewDocumentKind::Text, DocumentKind::Text),
        (NewDocumentKind::Mindmap, DocumentKind::Mindmap),
        (NewDocumentKind::Markdown, DocumentKind::Markdown),
    ] {
        let action =
            WidgetAction::Popup(PopupOutcome::Selected(PopupMenuAction::NewDocument(kind)));

        assert_eq!(
            new_document_menu_action(&action),
            Some(NotoraAction::CreateRequested(expected))
        );
    }
    let encrypted_action = WidgetAction::Popup(PopupOutcome::Selected(
        PopupMenuAction::NewDocument(NewDocumentKind::EncryptedMarkdown),
    ));
    assert_eq!(
        new_document_menu_action(&encrypted_action),
        Some(NotoraAction::BeginEncryptedNoteCreation)
    );
}

#[test]
fn dynamic_navigation_rows_keep_domain_values_out_of_the_ui_widget_keys() {
    let tag_id = notora_core::TagId::generate();
    let unused_tag_id = notora_core::TagId::generate();
    let mut state =
        NotoraState { workspace_root: WorkspaceRootState::Active, ..NotoraState::default() };
    state.library.navigation_tree.directories = vec!["plans".into(), "plans/q3".into()];
    state.library.navigation_tree.expanded_directories.insert("plans".into());
    state.library.navigation_tree.tag_root_expanded = true;
    state.library.navigation_tree.tags = vec![
        notora_core::TagWithActiveNoteCount {
            tag_id,
            display_name: "Plan".to_owned(),
            active_note_count: 2,
        },
        notora_core::TagWithActiveNoteCount {
            tag_id: unused_tag_id,
            display_name: "Unused".to_owned(),
            active_note_count: 0,
        },
    ];

    let model = NotoraRenderModel::from_state(&state);
    assert_eq!(model.navigation_rows.len(), 8);
    assert_eq!(model.navigation_rows[1].label, "plans");
    assert_eq!(model.navigation_rows[1].depth, 1);
    assert_eq!(model.navigation_rows[2].depth, 2);
    assert_eq!(model.navigation_rows[4].label, "标签");
    assert_eq!(model.navigation_rows[4].icon.as_deref(), Some("tag"));
    assert_eq!(model.navigation_rows[4].expansion, TreeRowExpansion::Expanded);
    assert_eq!(
        model.navigation_actions.get(&model.navigation_rows[4].key),
        Some(&NotoraAction::TagRootExpansionToggled)
    );
    assert_eq!(model.navigation_rows[5].depth, 1);
    assert_eq!(model.navigation_rows[5].badge, Some(2));
    assert!(model.navigation_rows.iter().all(|row| row.label != "Unused"));
    assert!(matches!(
        model.navigation_actions.get(&model.navigation_rows[5].key),
        Some(NotoraAction::NavigationSelected(NavigationScope::Tag { tag_id: selected_tag_id }))
            if *selected_tag_id == tag_id
    ));
}

#[test]
fn workspace_root_is_the_expandable_parent_of_directory_rows() {
    let mut state =
        NotoraState { workspace_root: WorkspaceRootState::Active, ..NotoraState::default() };
    state.workspace_root_path = Some("/tmp/notora-workspace".into());
    state.library.navigation_tree.directories = vec!["docs".into(), "docs/plans".into()];
    state.library.navigation_tree.expanded_directories.insert("docs".into());

    let expanded_model = NotoraRenderModel::from_state(&state);
    assert_eq!(expanded_model.navigation_rows[0].label, "工作区");
    assert_eq!(expanded_model.navigation_rows[0].tooltip.as_deref(), Some("/tmp/notora-workspace"));
    assert_eq!(expanded_model.navigation_rows[0].depth, 0);
    assert_eq!(expanded_model.navigation_rows[0].expansion, TreeRowExpansion::Expanded);
    assert_eq!(expanded_model.navigation_rows[1].depth, 1);
    assert_eq!(expanded_model.navigation_rows[2].depth, 2);

    state.library.navigation_tree.workspace_root_expanded = false;
    let collapsed_model = NotoraRenderModel::from_state(&state);
    assert_eq!(collapsed_model.navigation_rows[0].expansion, TreeRowExpansion::Collapsed);
    assert!(collapsed_model.navigation_rows.iter().all(|row| row.depth == 0));
}

#[test]
fn hovering_workspace_actions_exposes_their_tooltips_in_the_product_shell() {
    let mut state =
        NotoraState { workspace_root: WorkspaceRootState::Active, ..NotoraState::default() };
    state.library.navigation_tree.directories = vec!["docs".into()];
    let model = NotoraRenderModel::from_state(&state);
    let mut shell = NotoraShell::new();
    shell.update_model(&model);

    let theme = ui::theme::test_theme();
    let mut measure = ui::NoopMeasure;
    let mut layout_context =
        ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
    let tree_rect = Rect::new(12.0, 52.0, 240.0, 240.0);
    shell.navigation_tree_rect = tree_rect;
    shell.navigation_tree.set_rect(tree_rect, &mut layout_context);
    for expected_label in ["新建工作区", "打开工作区", "新建目录"] {
        let pointer_position = (tree_rect.x as usize..tree_rect.right() as usize)
            .flat_map(|px| {
                (tree_rect.y as usize..tree_rect.bottom() as usize)
                    .map(move |py| (px as f32, py as f32))
            })
            .find(|(px, py)| {
                shell
                    .navigation_tree
                    .tooltip_at(*px, *py)
                    .is_some_and(|hint| hint.label == expected_label)
            })
            .expect("workspace action should have a tooltip target");

        shell.route_event(
            &Event::MouseMove { px: pointer_position.0, py: pointer_position.1 },
            FocusTarget::Editor,
            &theme,
            1.0,
        );

        assert_eq!(
            shell.active_tooltip.as_ref().map(|hint| hint.label.as_str()),
            Some(expected_label)
        );
    }

    shell.route_event(
        &Event::MouseMove { px: tree_rect.x, py: tree_rect.bottom() - 1.0 },
        FocusTarget::Editor,
        &theme,
        1.0,
    );
    assert_eq!(shell.active_tooltip, None);
}

#[test]
fn hovering_a_closable_card_exposes_its_tooltip_in_the_product_shell() {
    let mut shell = NotoraShell::new();
    shell.card_list.set_input(VirtualCardListInput {
        cards: vec![CardInput {
            key: CardKey(7),
            title: "outside.md".to_owned(),
            excerpt: String::new(),
            timestamp: "外部文件".to_owned(),
            icon: Some("file-text".to_owned()),
            tag_summary: String::new(),
            selection: CardSelection::Unselected,
            closable: true,
        }],
        scroll_offset_px: 0.0,
    });
    let theme = ui::theme::test_theme();
    let mut measure = ui::NoopMeasure;
    let mut layout_context =
        ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
    shell.card_content_rect = Rect::new(240.0, 60.0, 320.0, 400.0);
    shell.card_list.set_rect(shell.card_content_rect, &mut layout_context);
    let close_rect = shell.card_list.layout().card_geometry(0).close_rect;
    let (px, py) = (close_rect.x + close_rect.w * 0.5, close_rect.y + close_rect.h * 0.5);

    shell.route_event(&Event::MouseMove { px, py }, FocusTarget::Editor, &theme, 1.0);

    assert_eq!(
        shell.active_tooltip.as_ref().map(|hint| hint.label.as_str()),
        Some("关闭 outside.md")
    );
}

#[test]
fn product_shell_collects_editor_header_and_split_button_tooltips() {
    let mut shell = NotoraShell::new();
    shell.editor_pane.set_input(EditorPaneInput {
        mode: EditorPaneMode::WorkspaceNote,
        header: ui::editor_header::EditorHeaderInput {
            title: "路线图".to_owned(),
            star_enabled: true,
            ..ui::editor_header::EditorHeaderInput::default()
        },
        ..EditorPaneInput::default()
    });
    shell
        .new_note_button
        .set_input(SplitButtonInput { label: "新建笔记".to_owned(), enabled: true });
    let theme = ui::theme::test_theme();
    let mut measure = ui::NoopMeasure;
    let mut layout_context =
        ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
    shell.editor_rect = Rect::new(200.0, 0.0, 640.0, 600.0);
    shell.editor_pane.set_rects(
        EditorPaneRects {
            header: Rect::new(200.0, 0.0, 640.0, 108.0),
            toolbar: Rect::new(200.0, 108.0, 640.0, 40.0),
            body: Rect::new(200.0, 148.0, 640.0, 452.0),
        },
        &mut layout_context,
    );
    shell.new_note_button.set_rect(Rect::new(40.0, 20.0, 128.0, 28.0), &mut layout_context);

    let star_pointer = (200..840)
        .flat_map(|px| (0..108).map(move |py| (px as f32, py as f32)))
        .find(|(px, py)| {
            shell.editor_pane.tooltip_at(*px, *py).is_some_and(|hint| hint.label == "添加星标")
        })
        .expect("editor star action should have a tooltip target");
    shell.route_event(
        &Event::MouseMove { px: star_pointer.0, py: star_pointer.1 },
        FocusTarget::Editor,
        &theme,
        1.0,
    );
    assert_eq!(shell.active_tooltip.as_ref().map(|hint| hint.label.as_str()), Some("添加星标"));

    let menu_rect = shell.new_note_button.menu_rect();
    shell.route_event(
        &Event::MouseMove {
            px: menu_rect.x + menu_rect.w * 0.5,
            py: menu_rect.y + menu_rect.h * 0.5,
        },
        FocusTarget::Editor,
        &theme,
        1.0,
    );
    assert_eq!(
        shell.active_tooltip.as_ref().map(|hint| hint.label.as_str()),
        Some("更多新建笔记选项")
    );
}

#[test]
fn directory_rows_expose_typed_creation_actions_and_inline_editor_input() {
    let mut state =
        NotoraState { workspace_root: WorkspaceRootState::Active, ..NotoraState::default() };
    state.library.navigation_tree.directories = vec!["docs".into()];
    state.directory_creation = DirectoryCreationState::Editing {
        parent_relative_path: "docs".into(),
        draft_name: "plans".to_owned(),
    };

    let model = NotoraRenderModel::from_state(&state);
    let root_row = &model.navigation_rows[0];
    let directory_row = &model.navigation_rows[1];

    assert_eq!(root_row.trailing_actions.len(), 3);
    assert_eq!(root_row.trailing_actions[0].key, NEW_WORKSPACE_ACTION_KEY);
    assert_eq!(root_row.trailing_actions[1].key, OPEN_WORKSPACE_ACTION_KEY);
    assert_eq!(root_row.trailing_actions[2].key, NEW_DIRECTORY_ACTION_KEY);
    assert_eq!(directory_row.trailing_actions.len(), 1);
    assert_eq!(directory_row.trailing_actions[0].key, NEW_DIRECTORY_ACTION_KEY);
    assert_eq!(
        model.navigation_trailing_actions.get(&(directory_row.key, NEW_DIRECTORY_ACTION_KEY)),
        Some(&NotoraAction::BeginDirectoryCreation { parent_relative_path: "docs".into() })
    );
    assert_eq!(
        model.navigation_editor,
        Some(TreeRowEditorInput {
            key: DIRECTORY_EDITOR_KEY,
            parent_key: directory_row.key,
            depth: 2,
            value: "plans".to_owned(),
            placeholder: "新目录名称".to_owned(),
        })
    );
}

#[test]
fn missing_workspace_keeps_the_root_directory_action_disabled() {
    let model = NotoraRenderModel::from_state(&NotoraState::default());
    let root_row = &model.navigation_rows[0];

    assert_eq!(root_row.label, "工作区");
    assert_eq!(root_row.trailing_actions.len(), 3);
    assert!(root_row.trailing_actions[0].enabled);
    assert!(root_row.trailing_actions[1].enabled);
    assert!(!root_row.trailing_actions[2].enabled);
    assert_eq!(
        model.navigation_trailing_actions.get(&(root_row.key, NEW_WORKSPACE_ACTION_KEY)),
        Some(&NotoraAction::OpenWorkspaceCreationRequested)
    );
    assert_eq!(
        model.navigation_trailing_actions.get(&(root_row.key, OPEN_WORKSPACE_ACTION_KEY)),
        Some(&NotoraAction::WorkspaceRootSelectionRequested)
    );
    assert!(
        !model.navigation_trailing_actions.contains_key(&(root_row.key, NEW_DIRECTORY_ACTION_KEY))
    );
}

#[test]
fn current_frame_keys_map_to_typed_navigation_actions() {
    let model = NotoraRenderModel::default();
    let mut shell = NotoraShell::new();
    shell.update_model(&model);
    shell
        .navigation_actions
        .insert(TreeRowKey(1), NotoraAction::NavigationSelected(NavigationScope::WorkspaceRoot));

    assert_eq!(
        shell.translate_widget_action(&WidgetAction::TreeList(TreeListAction::Selected(
            TreeRowKey(1),
        ))),
        Some(NotoraAction::NavigationSelected(NavigationScope::WorkspaceRoot))
    );
    assert_eq!(
        shell.translate_widget_action(&WidgetAction::TreeList(TreeListAction::ExpansionToggled(
            TreeRowKey(TAG_ROOT_NAVIGATION_KEY)
        ),)),
        Some(NotoraAction::TagRootExpansionToggled)
    );
    assert_eq!(
        shell.translate_widget_action(&WidgetAction::Control(
            ui::core::widget::ControlAction::Activated { id: shell.settings_button_id() },
        )),
        Some(NotoraAction::OpenSettings)
    );
    assert_eq!(
        shell.translate_widget_action(&WidgetAction::Control(ControlAction::TextEdited {
            id: GLOBAL_SEARCH_BOX_ID,
            value: TextPayload::Plain("roadmap".to_owned()),
        })),
        Some(NotoraAction::SearchTextChanged("roadmap".to_owned()))
    );
}

#[test]
fn tree_widget_directory_events_map_to_product_actions() {
    let mut shell = NotoraShell::new();
    shell.navigation_trailing_actions.insert(
        (TreeRowKey(7), NEW_DIRECTORY_ACTION_KEY),
        NotoraAction::BeginDirectoryCreation { parent_relative_path: "docs".into() },
    );

    assert_eq!(
        shell.translate_widget_action(&WidgetAction::TreeList(
            TreeListAction::TrailingActionActivated {
                row_key: TreeRowKey(7),
                action_key: NEW_DIRECTORY_ACTION_KEY,
            }
        )),
        Some(NotoraAction::BeginDirectoryCreation { parent_relative_path: "docs".into() })
    );
    assert_eq!(
        shell.translate_widget_action(&WidgetAction::TreeList(TreeListAction::EditorTextChanged {
            key: DIRECTORY_EDITOR_KEY,
            value: "plans".to_owned(),
        })),
        Some(NotoraAction::DirectoryCreationTextChanged("plans".to_owned()))
    );
    assert_eq!(
        shell.translate_widget_action(&WidgetAction::TreeList(
            TreeListAction::EditorCommitRequested {
                key: DIRECTORY_EDITOR_KEY,
                value: "plans".to_owned(),
            }
        )),
        Some(NotoraAction::DirectoryCreationCommitRequested)
    );
    assert_eq!(
        shell.translate_widget_action(&WidgetAction::TreeList(TreeListAction::EditorCancelled {
            key: DIRECTORY_EDITOR_KEY
        })),
        Some(NotoraAction::DirectoryCreationCancelled)
    );
}

#[test]
fn confirmation_overlay_maps_only_to_product_confirmation_actions() {
    let trash_confirmation =
        confirmation_overlay_input(OverlayState::TrashPermanentDeletionConfirmation {
            operation: crate::action::TrashOperation::Empty,
        })
        .expect("empty Trash confirmation should render");
    assert_eq!(trash_confirmation.confirm_action, NotoraAction::TrashPermanentDeletionConfirmed);

    let restore_conflict =
        confirmation_overlay_input(OverlayState::TrashRestoreConflictConfirmation {
            note_id: notora_core::NoteId::generate(),
        })
        .expect("restore conflict should render a decision overlay");
    assert_eq!(restore_conflict.confirm_action, NotoraAction::TrashRestoreWithRenamedPathConfirmed);
}

#[test]
fn save_conflict_is_exposed_as_a_four_way_product_decision() {
    let identity = DocumentIdentity::Note(NoteId::generate());
    let mut state = NotoraState::default();
    let _ = state.reduce(NotoraAction::SaveConflictDetected { identity, content_revision: 9 });

    let model = NotoraRenderModel::from_state(&state);
    let conflict = model.save_conflict.expect("save conflict overlay should render");
    assert_eq!(conflict.identity, identity);
    assert_eq!(
        conflict.actions,
        [
            NotoraAction::SaveConflictResolutionRequested(
                crate::action::ConflictResolution::ReloadFromDisk,
            ),
            NotoraAction::SaveConflictResolutionRequested(
                crate::action::ConflictResolution::SaveCopy,
            ),
            NotoraAction::SaveConflictResolutionRequested(
                crate::action::ConflictResolution::RetrySave,
            ),
            NotoraAction::SaveConflictResolutionRequested(
                crate::action::ConflictResolution::Cancel,
            ),
        ]
    );
}

#[test]
fn virtual_cards_map_catalog_dtos_with_stable_keys_and_selection() {
    let note_id = NoteId::generate();
    let identity = DocumentIdentity::Note(note_id);
    let mut state = NotoraState::default();
    state.library.selected_card = Some(identity);
    state.library.card_scroll_offset_px = 36.0;
    state.library.card_page = CardPageState::Ready {
        query: CardQuery::from(NavigationScope::WorkspaceRoot),
        cards: vec![CatalogCard {
            note_id,
            relative_path: "notes/roadmap.md".into(),
            kind: DocumentKind::Markdown,
            encryption: notora_core::NoteEncryption::Unencrypted,
            title: "Roadmap".to_owned(),
            excerpt: "Ship virtual cards".to_owned(),
            modified_nanoseconds: 42,
            starred: true,
            tags: vec!["plan".to_owned()],
        }],
        next_cursor: None,
    };
    let model = NotoraRenderModel::from_state(&state);
    assert_eq!(model.cards.len(), 1);
    assert_eq!(model.cards[0].tag_summary, "★ #plan");

    let mut shell = NotoraShell::new();
    shell.update_model(&model);
    let first_key = shell.card_list.input().cards[0].key;
    assert_eq!(shell.card_list.input().scroll_offset_px, 36.0);
    assert_eq!(shell.card_list.input().cards[0].selection, CardSelection::Selected);
    assert!(!shell.card_list.input().cards[0].closable);
    assert_eq!(
        shell.translate_widget_action(&WidgetAction::VirtualCardList(
            VirtualCardListAction::Selected(first_key),
        )),
        Some(NotoraAction::CardSelected(identity))
    );
    assert_eq!(
        shell.translate_widget_action(&WidgetAction::VirtualCardList(
            VirtualCardListAction::Activated(first_key),
        )),
        Some(NotoraAction::CardActivated(identity))
    );

    shell.update_model(&model);
    assert_eq!(shell.card_list.input().cards[0].key, first_key);
}

#[test]
fn external_file_cards_expose_close_actions_without_leaking_domain_state_to_ui() {
    let mut state = NotoraState::default();
    state.library.navigation_scope = NavigationScope::ExternalFiles;
    let identity = state.external_files.create_untitled(DocumentKind::Markdown);
    let DocumentIdentity::ExternalFile(external_file_id) = identity else {
        panic!("external card fixture must have an external identity");
    };
    let model = NotoraRenderModel::from_state(&state);
    let mut shell = NotoraShell::new();

    shell.update_model(&model);

    let card = &shell.card_list.input().cards[0];
    assert!(card.closable);
    assert_eq!(
        shell.translate_widget_action(&WidgetAction::VirtualCardList(
            VirtualCardListAction::CloseRequested(card.key),
        )),
        Some(NotoraAction::ExternalFileCloseRequested(external_file_id))
    );
}

#[test]
fn newly_opened_and_created_files_appear_at_the_top_of_the_file_list() {
    let directory = tempfile::tempdir().expect("external file directory should exist");
    let path = directory.path().join("opened.md");
    std::fs::write(&path, "# Opened").expect("external file fixture should be written");
    let canonical_path = crate::external_files::CanonicalExternalPath::canonicalize(&path)
        .expect("external file fixture should canonicalize");
    let mut state = NotoraState::default();
    state.library.navigation_scope = NavigationScope::ExternalFiles;
    let first = state.external_files.create_untitled(DocumentKind::Text);
    let opened = state.external_files.open_existing(canonical_path).identity();
    let newest = state.external_files.create_untitled(DocumentKind::Markdown);

    let identities = NotoraRenderModel::from_state(&state)
        .cards
        .iter()
        .map(|card| card.identity)
        .collect::<Vec<_>>();

    assert_eq!(identities, vec![newest, opened, first]);
}

#[test]
fn title_edit_updates_the_selected_middle_pane_card_before_catalog_refresh() {
    let note_id = NoteId::generate();
    let identity = DocumentIdentity::Note(note_id);
    let mut state = NotoraState::default();
    state.library.selected_card = Some(identity);
    state.library.card_page = CardPageState::Ready {
        query: CardQuery::from(NavigationScope::WorkspaceRoot),
        cards: vec![CatalogCard {
            note_id,
            relative_path: "notes/old-title.md".into(),
            kind: DocumentKind::Markdown,
            encryption: notora_core::NoteEncryption::Unencrypted,
            title: "旧标题".to_owned(),
            excerpt: "正文摘要".to_owned(),
            modified_nanoseconds: 42,
            starred: false,
            tags: Vec::new(),
        }],
        next_cursor: None,
    };

    let _ = state.reduce(NotoraAction::TitleTextChanged("编辑中的标题".to_owned()));

    let editing_model = NotoraRenderModel::from_state(&state);
    assert_eq!(editing_model.cards[0].title, "编辑中的标题");

    let _ = state.reduce(NotoraAction::TitleCommitRequested("新标题".to_owned()));

    let pending_model = NotoraRenderModel::from_state(&state);
    assert_eq!(pending_model.cards[0].title, "新标题");
}

#[test]
fn moving_pointer_to_other_pane_clears_card_hover() {
    use ui::core::paint::{DrawCmd, DrawList};

    fn paints_card_hover(shell: &NotoraShell, theme: &ui::Theme) -> bool {
        let mut draw_list = DrawList::new();
        let mut paint_context = ui::PaintCtx::new(&mut draw_list, theme, 1.0);
        shell.card_list.paint(&mut paint_context);
        draw_list.cmds.iter().any(|command| {
            matches!(
                command,
                DrawCmd::FillRect { color, .. }
                    if *color == theme.palette.sidebar_hover_bg
            )
        })
    }

    let mut shell = NotoraShell::new();
    let theme = ui::theme::test_theme();
    let mut measure = ui::NoopMeasure;
    let mut layout_context =
        ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
    shell.navigation_tree_rect = Rect::new(0.0, 0.0, 200.0, 600.0);
    shell.card_content_rect = Rect::new(220.0, 0.0, 300.0, 600.0);
    shell.editor_rect = Rect::new(540.0, 0.0, 600.0, 600.0);
    shell.card_list.set_input(VirtualCardListInput {
        cards: vec![CardInput {
            key: CardKey(1),
            title: "Card".to_owned(),
            excerpt: String::new(),
            timestamp: String::new(),
            icon: None,
            tag_summary: String::new(),
            selection: CardSelection::Unselected,
            closable: false,
        }],
        scroll_offset_px: 0.0,
    });
    shell.card_list.set_rect(shell.card_content_rect, &mut layout_context);
    let card_rect = shell.card_list.layout().card_geometry(0).card_rect;
    let hover_card = Event::MouseMove {
        px: card_rect.x + card_rect.w * 0.5,
        py: card_rect.y + card_rect.h * 0.5,
    };

    for destination in
        [Event::MouseMove { px: 100.0, py: 100.0 }, Event::MouseMove { px: 700.0, py: 100.0 }]
    {
        shell.route_event(&hover_card, FocusTarget::Editor, &theme, 1.0);
        assert!(paints_card_hover(&shell, &theme));

        shell.route_event(&destination, FocusTarget::Editor, &theme, 1.0);
        assert!(!paints_card_hover(&shell, &theme));
    }
}

#[test]
fn workspace_note_toolbar_has_no_note_level_commands() {
    let note_id = NoteId::generate();
    assert!(note_toolbar_buttons(&NavigationScope::WorkspaceRoot, Some(note_id), false).is_empty());
}

#[test]
fn tag_scope_has_no_manual_tag_mutation_toolbar_actions() {
    let tag_id = notora_core::TagId::generate();
    let note_id = NoteId::generate();
    let tag_buttons = note_toolbar_buttons(&NavigationScope::Tag { tag_id }, Some(note_id), false);
    assert!(tag_buttons.is_empty());
}

#[test]
fn trash_toolbar_ignores_a_selection_that_is_not_in_the_current_card_page() {
    let note_id = NoteId::generate();
    let mut state = NotoraState::default();
    state.library.navigation_scope = NavigationScope::Trash;
    state.library.selected_card = Some(DocumentIdentity::Note(note_id));

    let model = NotoraRenderModel::from_state(&state);
    assert_eq!(model.note_toolbar.len(), 1);
    assert_eq!(
        model.note_toolbar[0].action,
        NotoraAction::TrashOperationRequested(TrashOperation::Empty)
    );
}

#[test]
fn editor_pane_input_matrix_keeps_external_and_trash_metadata_read_only() {
    let note_id = NoteId::generate();
    let external_id = notora_core::ExternalFileId::generate();

    let empty_state = NotoraState::default();
    assert_eq!(editor_pane_input(&empty_state, &[]).mode, EditorPaneMode::Empty);

    let mut note_state = NotoraState::default();
    note_state.library.selected_card = Some(DocumentIdentity::Note(note_id));
    let note_input = editor_pane_input(
        &note_state,
        &[RenderCard {
            identity: DocumentIdentity::Note(note_id),
            title: "路线图".to_owned(),
            excerpt: String::new(),
            timestamp: String::new(),
            icon: None,
            tag_summary: "★ #产品/Notora".to_owned(),
            closable: false,
        }],
    );
    assert_eq!(note_input.mode, EditorPaneMode::WorkspaceNote);
    assert!(note_input.header.title_editable);
    assert!(note_input.header.star_enabled);
    assert_eq!(note_input.tags.chips[0].label, "产品/Notora");

    note_state.library.navigation_scope = NavigationScope::Trash;
    let trash_input = editor_pane_input(&note_state, &[]);
    assert_eq!(trash_input.mode, EditorPaneMode::TrashNote);
    assert!(!trash_input.header.title_editable);
    assert!(!trash_input.tags.enabled);

    let mut external_state = NotoraState::default();
    external_state.library.selected_card = Some(DocumentIdentity::ExternalFile(external_id));
    let external_input = editor_pane_input(&external_state, &[]);
    assert_eq!(external_input.mode, EditorPaneMode::ExternalFile);
    assert!(!external_input.header.title_editable);
    assert_eq!(external_input.header.encryption, ui::editor_header::EncryptionStatusInput::Hidden);
    assert!(!external_input.location.open);
}

#[test]
fn canvas_toolbar_routes_viewport_commands_without_document_actions() {
    let shell = NotoraShell::new();
    for (key, expected) in [
        ("canvas_zoom_in", CanvasViewCommand::ZoomIn),
        ("canvas_zoom_out", CanvasViewCommand::ZoomOut),
        ("canvas_fit", CanvasViewCommand::Fit),
    ] {
        let route = shell.route_editor_widget_action(&WidgetAction::Control(
            ControlAction::TextCommitted {
                id: ui::editor_toolbar::EDITOR_TOOLBAR_COMMAND_ID,
                value: TextPayload::Plain(key.to_owned()),
            },
        ));
        assert!(route.consumed);
        assert!(route.actions.is_empty());
        assert_eq!(route.canvas_view_command, Some(expected));
    }
    let viewport = Rect::new(200.0, 100.0, 600.0, 400.0);
    let appkit_shell::canvas_viewport::CanvasViewportAction::ZoomBy { factor, screen_anchor } =
        CanvasViewCommand::ZoomIn.viewport_action(viewport)
    else {
        panic!("zoom command must zoom");
    };
    assert!(factor > 1.0);
    assert_eq!(screen_anchor, ui::canvas::CanvasPoint::new(500.0, 300.0));
    assert_eq!(
        CanvasViewCommand::Fit.viewport_action(viewport),
        appkit_shell::canvas_viewport::CanvasViewportAction::ResetView
    );
}

#[test]
fn mindmap_toolbar_exposes_view_controls_without_editing_commands() {
    let toolbar = mindmap_toolbar_input(None);
    let keys = toolbar
        .groups
        .iter()
        .flat_map(|group| &group.commands)
        .map(|command| command.command_key.as_str())
        .collect::<Vec<_>>();
    for expected in ["canvas_zoom_out", "canvas_zoom_in", "canvas_fit"] {
        assert!(keys.contains(&expected), "missing view control {expected}");
    }
    assert!(!keys.contains(&"bold"));
    assert!(markdown_toolbar_input().groups.len() >= 4);
}

#[test]
fn editor_toolbar_commands_follow_the_active_plugin_capabilities() {
    let command_keys = |input: ui::editor_toolbar::EditorToolbarInput| {
        input
            .groups
            .into_iter()
            .flat_map(|group| group.commands)
            .map(|command| command.command_key)
            .collect::<Vec<_>>()
    };
    assert!(
        command_keys(editor_toolbar_input_for_plugin(
            EditorPaneMode::WorkspaceNote,
            ui::plugin::PLUGIN_MARKDOWN_EDITOR,
            false,
        ))
        .contains(&"bold".to_owned())
    );
    assert_eq!(
        command_keys(editor_toolbar_input_for_plugin(
            EditorPaneMode::WorkspaceNote,
            ui::plugin::PLUGIN_EDITOR,
            false,
        )),
        vec!["undo".to_owned(), "redo".to_owned()]
    );
    assert_eq!(
        command_keys(editor_toolbar_input_for_plugin(
            EditorPaneMode::WorkspaceNote,
            ui::plugin::PLUGIN_MINDMAP,
            true,
        )),
        vec![
            "undo".to_owned(),
            "redo".to_owned(),
            "inspector_panel".to_owned(),
            "promote".to_owned(),
            "demote".to_owned(),
            "canvas_zoom_out".to_owned(),
            "canvas_zoom_in".to_owned(),
            "canvas_fit".to_owned(),
        ]
    );
    let mut compact_toolbar = editor_toolbar_input_for_plugin(
        EditorPaneMode::WorkspaceNote,
        ui::plugin::PLUGIN_MARKDOWN_EDITOR,
        false,
    );
    add_compact_editor_toolbar_commands(&mut compact_toolbar);
    assert!(command_keys(compact_toolbar).contains(&"delete".to_owned()));
}

#[test]
fn source_toggle_command_stays_visible_and_reflects_the_current_view() {
    let mut visual_toolbar = editor_toolbar_input_for_plugin(
        EditorPaneMode::WorkspaceNote,
        ui::plugin::PLUGIN_MARKDOWN_EDITOR,
        false,
    );
    add_source_toggle_command(&mut visual_toolbar, false);
    let source_command = visual_toolbar.groups[0]
        .commands
        .iter()
        .find(|command| command.command_key == "toggle_source")
        .expect("source toggle should be present");
    assert_eq!(source_command.command_key, "toggle_source");
    assert_eq!(source_command.label, "源码");
    assert_eq!(source_command.overflow_priority, 0);
    assert_eq!(visual_toolbar.groups[0].commands[2].command_key, "toggle_source");

    let mut source_toolbar = history_toolbar_input();
    add_source_toggle_command(&mut source_toolbar, true);
    assert_eq!(
        source_toolbar.groups[0]
            .commands
            .iter()
            .find(|command| command.command_key == "toggle_source")
            .expect("visual toggle should be present")
            .label,
        "可视化"
    );
}

#[test]
fn document_search_toolbar_action_opens_search_without_editing() {
    assert_eq!(
        editor_command_actions().get("document_search"),
        Some(&NotoraAction::DocumentSearchRequested)
    );
}

#[test]
fn editor_widget_keys_translate_to_typed_note_actions() {
    let note_id = NoteId::generate();
    let tag_id = notora_core::TagId::generate();
    let suggestion_tag_id = notora_core::TagId::generate();
    let mut state = NotoraState::default();
    state.library.selected_card = Some(DocumentIdentity::Note(note_id));
    state.library.selected_document_generation = 4;
    state.library.active_editor_metadata = Some(crate::state::ActiveEditorMetadata {
        identity: DocumentIdentity::Note(note_id),
        selection_generation: 4,
        metadata: notora_core::NoteEditorMetadata {
            note_id,
            created_at: SystemTime::UNIX_EPOCH,
            modified_at: SystemTime::UNIX_EPOCH,
            encryption: notora_core::NoteEncryption::Unencrypted,
            title_initialization: notora_core::TitleInitialization::Independent,
            file_name_binding: notora_core::NoteFileNameBinding::TitleBound { disambiguator: 1 },
            title_revision: 0,
        },
        tags: vec![notora_core::TagSummary { tag_id, display_name: "产品/Notora".to_owned() }],
    });
    state.library.navigation_tree.tags = vec![
        notora_core::TagWithActiveNoteCount {
            tag_id,
            display_name: "产品/Notora".to_owned(),
            active_note_count: 1,
        },
        notora_core::TagWithActiveNoteCount {
            tag_id: suggestion_tag_id,
            display_name: "设计/UI".to_owned(),
            active_note_count: 2,
        },
    ];
    let model = NotoraRenderModel::from_state(&state);
    assert_eq!(
        model.editor_chrome.tags.suggestions,
        vec![ui::tag_editor::TagSuggestionInput {
            option_key: format!("suggestion:{suggestion_tag_id}"),
            label: "设计/UI".to_owned(),
            enabled: true,
        }]
    );
    let mut shell = NotoraShell::new();
    shell.update_model(&model);

    assert_eq!(
        shell.translate_widget_action(&WidgetAction::Control(ControlAction::TextCommitted {
            id: ui::tag_editor::TAG_EDITOR_SUBMIT_ID,
            value: TextPayload::Plain("设计/UI".to_owned()),
        })),
        Some(NotoraAction::MetadataMutationRequested(MetadataMutation::AttachTagByName {
            note_id,
            display_name: "设计/UI".to_owned(),
        }))
    );
    assert_eq!(
        shell.translate_widget_action(&WidgetAction::Control(ControlAction::FocusRequested {
            id: ui::editor_header::EDITOR_HEADER_TITLE_ID,
        })),
        Some(NotoraAction::FocusRequested(FocusTarget::EditorTitle))
    );
    assert_eq!(
        shell.translate_widget_action(&WidgetAction::Control(ControlAction::TextCommitted {
            id: ui::editor_toolbar::EDITOR_TOOLBAR_COMMAND_ID,
            value: TextPayload::Plain("inspector_panel".to_owned()),
        })),
        Some(NotoraAction::ToggleInspectorPanelRequested)
    );
    assert_eq!(
        shell.translate_widget_action(&WidgetAction::Control(ControlAction::TextEdited {
            id: ui::editor_header::EDITOR_HEADER_TITLE_ID,
            value: TextPayload::Plain("新的标题".to_owned()),
        })),
        Some(NotoraAction::TitleTextChanged("新的标题".to_owned()))
    );
    assert_eq!(
        shell.translate_widget_action(&WidgetAction::Control(ControlAction::FocusRequested {
            id: ui::tag_editor::TAG_EDITOR_INPUT_ID,
        })),
        Some(NotoraAction::FocusRequested(FocusTarget::EditorTag))
    );
    assert_eq!(
        shell.translate_widget_action(&WidgetAction::Control(ControlAction::Activated {
            id: ui::tag_editor::TAG_EDITOR_CANCEL_ID,
        })),
        Some(NotoraAction::FocusRequested(FocusTarget::Editor))
    );
    assert_eq!(
        shell.translate_widget_action(&WidgetAction::Control(ControlAction::TextCommitted {
            id: ui::tag_editor::TAG_EDITOR_SUGGESTION_ID,
            value: TextPayload::Plain(format!("suggestion:{suggestion_tag_id}")),
        })),
        Some(NotoraAction::MetadataMutationRequested(MetadataMutation::AttachTagByName {
            note_id,
            display_name: "设计/UI".to_owned(),
        }))
    );
    assert_eq!(
        shell.translate_widget_action(&WidgetAction::Control(ControlAction::TextCommitted {
            id: ui::tag_editor::TAG_EDITOR_REMOVE_ID,
            value: TextPayload::Plain(tag_id.to_string()),
        })),
        Some(NotoraAction::MetadataMutationRequested(MetadataMutation::DetachTag {
            note_id,
            tag_id,
        }))
    );
    assert_eq!(
        shell.translate_widget_action(&WidgetAction::Control(ControlAction::TextCommitted {
            id: ui::location_picker::LOCATION_PICKER_SELECT_ID,
            value: TextPayload::Plain(EDITOR_ROOT_DIRECTORY_ROW_KEY.to_owned()),
        })),
        Some(NotoraAction::MoveRequested { note_id, target_directory: std::path::PathBuf::new() })
    );
    assert_eq!(
        shell.translate_widget_action(&WidgetAction::Control(ControlAction::TextCommitted {
            id: ui::editor_toolbar::EDITOR_TOOLBAR_COMMAND_ID,
            value: TextPayload::Plain("bold".to_owned()),
        })),
        Some(NotoraAction::SemanticEditRequested(ui::plugin::SemanticEditCommand::ToggleBold,))
    );
    assert_eq!(
        shell.translate_widget_action(&WidgetAction::Control(ControlAction::TextCommitted {
            id: ui::editor_toolbar::EDITOR_TOOLBAR_COMMAND_ID,
            value: TextPayload::Plain("toggle_source".to_owned()),
        })),
        Some(NotoraAction::ToggleSourceViewRequested)
    );
}
