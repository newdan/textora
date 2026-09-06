use super::tests::{app, install_registered_note};
use super::*;
use crate::FocusTarget;
use ui::KeyCode;
use ui::core::Modifiers;

const PRIMARY: Modifiers = Modifiers { cmd: true, ..Modifiers::NONE };
const PRIMARY_SHIFT: Modifiers = Modifiers { shift: true, ..PRIMARY };
const PRIMARY_ALT: Modifiers = Modifiers { alt: true, ..PRIMARY };

const FORMATTING_SHORTCUTS: &[(char, Modifiers, &str)] = &[
    ('b', PRIMARY, "**正文**"),
    ('i', PRIMARY, "*正文*"),
    ('k', PRIMARY, "[正文](https://)"),
    ('e', PRIMARY, "`正文`"),
    ('x', PRIMARY_SHIFT, "~~正文~~"),
    ('7', PRIMARY_SHIFT, "1. 正文"),
    ('8', PRIMARY_SHIFT, "- 正文"),
    ('9', PRIMARY_SHIFT, "> 正文"),
    ('1', PRIMARY_ALT, "# 正文"),
    ('2', PRIMARY_ALT, "## 正文"),
    ('3', PRIMARY_ALT, "### 正文"),
    ('4', PRIMARY_ALT, "#### 正文"),
    ('5', PRIMARY_ALT, "##### 正文"),
    ('6', PRIMARY_ALT, "###### 正文"),
    ('t', PRIMARY_ALT, "- [ ] 正文"),
    ('c', PRIMARY_ALT, "```\n正文\n```"),
];

#[test]
fn physical_formatting_cannot_bypass_preedit_by_producing_a_copy_character() {
    let (mut runtime, tab_id) = selected_markdown();
    assert!(runtime.update_editor_preedit("拼".to_owned(), Some((0, 1))));
    runtime.handle_key_input(
        KeyCode::Char('c'),
        PRIMARY,
        Some(winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyI)),
    );
    assert_eq!(document_text(&runtime, tab_id), "正文");
}

#[test]
fn ime_lifecycle_clears_composition_before_the_next_shortcut() {
    for event in [winit::event::Ime::Enabled, winit::event::Ime::Disabled] {
        let (mut runtime, tab_id) = selected_markdown();
        assert!(runtime.update_editor_preedit("拼".to_owned(), Some((0, 1))));
        runtime.handle_ime_input(event);
        runtime.handle_key_input(KeyCode::Char('b'), PRIMARY, None);
        assert_eq!(document_text(&runtime, tab_id), "**正文**");
    }
}

#[test]
fn physical_shortcuts_survive_shift_alt_and_keyboard_layout_translation() {
    use winit::keyboard::{KeyCode as PhysicalCode, PhysicalKey};
    for (physical, logical, modifiers, expected) in [
        (PhysicalCode::Digit7, '&', PRIMARY_SHIFT, "1. 正文"),
        (PhysicalCode::Digit8, '*', PRIMARY_SHIFT, "- 正文"),
        (PhysicalCode::Digit9, '(', PRIMARY_SHIFT, "> 正文"),
        (PhysicalCode::Digit2, '™', PRIMARY_ALT, "## 正文"),
        (PhysicalCode::KeyT, '†', PRIMARY_ALT, "- [ ] 正文"),
        (PhysicalCode::KeyC, 'ç', PRIMARY_ALT, "```\n正文\n```"),
        (PhysicalCode::KeyI, 'ш', PRIMARY, "*正文*"),
        (PhysicalCode::KeyI, 'f', PRIMARY, "*正文*"),
    ] {
        let (mut runtime, tab_id) = selected_markdown();
        runtime.handle_key_input(
            KeyCode::Char(logical),
            modifiers,
            Some(PhysicalKey::Code(physical)),
        );
        assert_eq!(document_text(&runtime, tab_id), expected, "{physical:?} / {logical}");
    }
}

fn selected_markdown() -> (NotoraRuntime, appkit_core::workspace::types::TabId) {
    let mut runtime = app();
    let (_, tab_id) = install_registered_note(&mut runtime, "shortcuts.md", "正文");
    runtime.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
    let tab = runtime
        .document_runtime
        .editor_mut()
        .tab_session_mut(tab_id)
        .expect("shortcut fixture should retain its Markdown tab");
    tab.document.cursor_move_to_offset("正文".len());
    tab.document.cursor_mut().selection_anchor = Some(0);
    (runtime, tab_id)
}

fn document_text(runtime: &NotoraRuntime, tab_id: appkit_core::workspace::types::TabId) -> String {
    runtime
        .document_runtime
        .editor()
        .document_text_snapshot(tab_id)
        .expect("shortcut should retain the document")
        .text
}

#[test]
fn all_markdown_shortcuts_format_selected_text_and_support_undo_redo() {
    for &(character, modifiers, expected) in FORMATTING_SHORTCUTS {
        for primary in [PRIMARY, Modifiers { ctrl: true, ..Modifiers::NONE }] {
            let modifiers = Modifiers { cmd: primary.cmd, ctrl: primary.ctrl, ..modifiers };
            let (mut runtime, tab_id) = selected_markdown();
            runtime.handle_key_input(KeyCode::Char(character), modifiers, None);
            assert_eq!(document_text(&runtime, tab_id), expected, "{modifiers:?}+{character}");
            runtime.handle_key_input(KeyCode::Char('z'), primary, None);
            assert_eq!(document_text(&runtime, tab_id), "正文", "undo {character}");
            runtime.handle_key_input(
                KeyCode::Char('Z'),
                Modifiers { shift: true, ..primary },
                None,
            );
            assert_eq!(document_text(&runtime, tab_id), expected, "redo {character}");
        }
    }
}

#[test]
fn preedit_blocks_editing_keys_without_changing_source_or_undo_history() {
    for (key, modifiers) in [
        (KeyCode::Char('a'), Modifiers::NONE),
        (KeyCode::Enter, Modifiers::NONE),
        (KeyCode::Enter, Modifiers { shift: true, ..Modifiers::NONE }),
        (KeyCode::Backspace, Modifiers::NONE),
        (KeyCode::Delete, Modifiers::NONE),
        (KeyCode::Tab, Modifiers::NONE),
        (KeyCode::Char('x'), PRIMARY),
        (KeyCode::Char('v'), PRIMARY),
        (KeyCode::Char('z'), PRIMARY),
        (KeyCode::Char('Z'), PRIMARY_SHIFT),
        (KeyCode::Char('i'), PRIMARY),
    ] {
        let (mut runtime, tab_id) = selected_markdown();
        runtime.commit_editor_text("替换".to_owned());
        assert!(runtime.update_editor_preedit("拼".to_owned(), Some((0, 1))));
        runtime.handle_key_input(key, modifiers, None);
        assert_eq!(document_text(&runtime, tab_id), "替换", "preedit {modifiers:?}+{key:?}");
        assert!(runtime.update_editor_preedit(String::new(), None));
        runtime.handle_key_input(KeyCode::Char('z'), PRIMARY, None);
        assert_eq!(document_text(&runtime, tab_id), "正文", "preedit must not add undo entries");
    }
}

#[test]
fn returning_from_search_clears_stale_preedit_and_allows_formatting() {
    let (mut runtime, tab_id) = selected_markdown();
    assert!(runtime.update_editor_preedit("拼".to_owned(), Some((0, 1))));
    runtime.dispatch_action(NotoraAction::FocusRequested(FocusTarget::NavigationSearch));
    runtime.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
    runtime.handle_key_input(KeyCode::Char('b'), PRIMARY, None);
    assert_eq!(document_text(&runtime, tab_id), "**正文**");
    assert!(runtime.document_runtime.editor().preedit().0.is_empty());
}

#[derive(Clone, Copy, Debug)]
enum FormattingBlocker {
    Search,
    Title,
    Modal,
    Preedit,
    SourceView,
    ReadOnly,
    WindowUnfocused,
}

fn block_formatting(
    runtime: &mut NotoraRuntime,
    tab_id: appkit_core::workspace::types::TabId,
    blocker: FormattingBlocker,
) {
    match blocker {
        FormattingBlocker::Search => {
            runtime.dispatch_action(NotoraAction::FocusRequested(FocusTarget::NavigationSearch))
        }
        FormattingBlocker::Title => {
            runtime.dispatch_action(NotoraAction::FocusRequested(FocusTarget::EditorTitle))
        }
        FormattingBlocker::Modal => runtime.dispatch_action(NotoraAction::OpenSettings),
        FormattingBlocker::Preedit => {
            assert!(runtime.update_editor_preedit("拼".to_owned(), Some((0, 1))));
        }
        FormattingBlocker::SourceView => {
            runtime.dispatch_action(NotoraAction::ToggleSourceViewRequested)
        }
        FormattingBlocker::ReadOnly => runtime
            .document_runtime
            .editor_mut()
            .tab_session_mut(tab_id)
            .expect("read-only fixture should retain its tab")
            .runtime
            .set_editing_access(appkit_shell::tab_runtime::DocumentEditingAccess::ReadOnly),
        FormattingBlocker::WindowUnfocused => runtime.set_window_focused(false),
    }
}

#[test]
fn every_formatting_shortcut_respects_editor_input_ownership_and_read_only_documents() {
    for blocker in [
        FormattingBlocker::Search,
        FormattingBlocker::Title,
        FormattingBlocker::Modal,
        FormattingBlocker::Preedit,
        FormattingBlocker::SourceView,
        FormattingBlocker::ReadOnly,
        FormattingBlocker::WindowUnfocused,
    ] {
        for &(character, modifiers, _) in FORMATTING_SHORTCUTS {
            let (mut runtime, tab_id) = selected_markdown();
            block_formatting(&mut runtime, tab_id, blocker);
            runtime.handle_key_input(KeyCode::Char(character), modifiers, None);
            assert_eq!(
                document_text(&runtime, tab_id),
                "正文",
                "{blocker:?}: {modifiers:?}+{character}"
            );
        }
    }
}

#[test]
fn physical_keys_preserve_unmodified_text_and_require_matching_shortcut_modifiers() {
    use winit::keyboard::{KeyCode as PhysicalCode, PhysicalKey};
    let (mut runtime, tab_id) = selected_markdown();
    runtime.handle_key_input(
        KeyCode::Char('ш'),
        Modifiers::NONE,
        Some(PhysicalKey::Code(PhysicalCode::KeyI)),
    );
    assert_eq!(document_text(&runtime, tab_id), "ш");

    for modifiers in [PRIMARY_SHIFT, PRIMARY_ALT] {
        let (mut runtime, tab_id) = selected_markdown();
        runtime.handle_key_input(
            KeyCode::Char('i'),
            modifiers,
            Some(PhysicalKey::Code(PhysicalCode::KeyI)),
        );
        assert_eq!(document_text(&runtime, tab_id), "正文");
    }
}

#[test]
fn formatting_edge_cases_execute_as_real_transactions() {
    for (source, selection_end, character, modifiers, expected) in [
        ("正文\r\n后文", "正文".len(), '9', PRIMARY_SHIFT, "> 正文\n后文"),
        ("正文\r\n后文", "正文".len(), 'c', PRIMARY_ALT, "```\n正文\n```\n后文"),
        ("", 0, '8', PRIMARY_SHIFT, "- "),
        ("", 0, '2', PRIMARY_ALT, "## "),
        ("", 0, 'c', PRIMARY_ALT, "```\n\n```"),
        ("*", 1, 'i', PRIMARY, "***"),
        ("```\n```", "```\n```".len(), 'c', PRIMARY_ALT, ""),
    ] {
        let mut runtime = app();
        let (_, tab_id) = install_registered_note(&mut runtime, "edges.md", source);
        let loaded_source = document_text(&runtime, tab_id);
        runtime.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
        let tab = runtime
            .document_runtime
            .editor_mut()
            .tab_session_mut(tab_id)
            .expect("edge case should retain its document");
        tab.document.cursor_move_to_offset(selection_end);
        tab.document.cursor_mut().selection_anchor = Some(0);
        runtime.handle_key_input(KeyCode::Char(character), modifiers, None);
        assert_eq!(document_text(&runtime, tab_id), expected, "{source:?}: {character}");
        runtime.handle_key_input(KeyCode::Char('z'), PRIMARY, None);
        assert_eq!(document_text(&runtime, tab_id), loaded_source, "edge-case undo");
    }
}
