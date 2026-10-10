use appkit_shell::editor_runtime::EditorRuntime;
use appkit_shell::tab_session::TabSessionMut;
use ui::plugin::PluginMessage;

const MAXIMUM_QUERY_BYTES: usize = 2_048;

pub(crate) fn open(editor: &mut EditorRuntime) -> bool {
    let Some(tab_id) = editor.active_tab_id() else {
        return false;
    };
    let Some(mut tab) = editor.tab_session_mut(tab_id) else {
        return false;
    };
    if tab.is_canvas() {
        return false;
    }
    let selected_query = tab
        .document
        .selection_range()
        .filter(|(start, end)| end - start <= MAXIMUM_QUERY_BYTES)
        .and_then(|_| tab.document.extract_selected_text())
        .and_then(|selected| String::from_utf8(selected).ok())
        .filter(|selected| !selected.contains(['\n', '\r']));
    let search = tab.search_state_mut();
    search.panel_visible = true;
    search.options = Default::default();
    if let Some(query) = selected_query {
        search.query = query;
    }
    search.active_match_idx = 0;
    recompute_matches(&mut tab);
    reveal_active_match(editor);
    true
}

pub(crate) fn close(editor: &mut EditorRuntime) {
    let Some(tab_id) = editor.active_tab_id() else {
        return;
    };
    let Some(mut tab) = editor.tab_session_mut(tab_id) else {
        return;
    };
    let search = tab.search_state_mut();
    search.panel_visible = false;
    search.matches.clear();
    search.active_match_idx = 0;
}

pub(crate) fn set_query(editor: &mut EditorRuntime, query: String) {
    let Some(tab_id) = editor.active_tab_id() else {
        return;
    };
    let Some(mut tab) = editor.tab_session_mut(tab_id) else {
        return;
    };
    if !tab.search_state().panel_visible {
        return;
    }
    let query_end = query.floor_char_boundary(query.len().min(MAXIMUM_QUERY_BYTES));
    let search = tab.search_state_mut();
    search.query = query[..query_end].to_owned();
    search.active_match_idx = 0;
    recompute_matches(&mut tab);
    reveal_active_match(editor);
}

pub(crate) fn refresh(editor: &mut EditorRuntime) -> bool {
    let Some(tab_id) = editor.active_tab_id() else {
        return false;
    };
    let Some(mut tab) = editor.tab_session_mut(tab_id) else {
        return false;
    };
    let search = tab.search_state();
    if !search.panel_visible
        || search.buffer_generation == tab.document.tb.gap_buffer().generation()
    {
        return false;
    }
    recompute_matches(&mut tab);
    true
}

pub(crate) fn navigate(editor: &mut EditorRuntime, backwards: bool) {
    refresh(editor);
    let Some(tab_id) = editor.active_tab_id() else {
        return;
    };
    let Some(mut tab) = editor.tab_session_mut(tab_id) else {
        return;
    };
    if !tab.search_state().panel_visible {
        return;
    }
    if backwards {
        tab.search_state_mut().prev_match();
    } else {
        tab.search_state_mut().next_match();
    }
    reveal_active_match(editor);
}

fn recompute_matches(tab: &mut TabSessionMut<'_>) {
    let source = tab.document.full_text();
    let matches = core::buffer::simd_search::find_all_case_insensitive_ascii(
        tab.search_state().query.as_bytes(),
        source.as_bytes(),
    );
    let generation = tab.document.tb.gap_buffer().generation();
    tab.search_state_mut().update_matches(matches, generation);
}

fn reveal_active_match(editor: &mut EditorRuntime) {
    let line_height =
        ui::UiMetrics::from_settings(&editor.settings_snapshot(), editor.scale_factor() as f32)
            .line_height;
    let Some(tab_id) = editor.active_tab_id() else {
        return;
    };
    let Some(mut tab) = editor.tab_session_mut(tab_id) else {
        return;
    };
    let Some(matched) = tab.search_state().active_match() else {
        return;
    };
    tab.document.set_cursor_offset_synced(matched.end);
    tab.document.cursor_mut().selection_anchor = Some(matched.start);
    tab.send_message(PluginMessage::SetCursorByte(matched.end));
    tab.send_message(PluginMessage::SetSelAnchorByte(Some(matched.start)));
    tab.send_message(PluginMessage::SetSelCursorByte(Some(matched.end)));
    tab.ensure_cursor_visible(line_height);
}

#[cfg(test)]
mod tests {
    use super::*;
    use appkit_core::document::DocumentModel;
    use appkit_shell::editor_runtime::{EditorRuntimeConfig, OpenDisposition};
    use appkit_shell::prepared_tab::PreparedTab;
    use appkit_shell::tab_runtime::TabRuntime;
    use core::buffer::TextBuffer;

    fn editor(text: &str) -> EditorRuntime {
        let (plugin_registry, view_routes) =
            crate::editor_adapter::build_editor_plugins().expect("valid product routes");
        let mut editor = EditorRuntime::new(EditorRuntimeConfig {
            plugin_registry,
            view_routes,
            initial_settings: ui::Settings::new(),
            initial_theme: ui::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark()),
            snapshots_directory: std::env::temp_dir().join("notora-document-search-tests"),
        })
        .expect("headless runtime constructs");
        install(&mut editor, text);
        editor
    }

    fn install(editor: &mut EditorRuntime, text: &str) {
        let mut buffer = TextBuffer::new(false).expect("writable test buffer");
        buffer.write_raw(text.as_bytes());
        editor.install_prepared_tab(
            PreparedTab::new(
                DocumentModel::new(buffer),
                TabRuntime::new(editor.create_plugin_by_name(ui::plugin::PLUGIN_EDITOR)),
            ),
            None,
            OpenDisposition::Persistent,
        );
    }

    fn search(editor: &EditorRuntime) -> appkit_core::document::SearchState {
        editor
            .tab_session(editor.active_tab_id().expect("active test tab"))
            .expect("test tab session")
            .search_state()
            .clone()
    }

    #[test]
    fn document_search_counts_chinese_and_cycles_selection() {
        let mut editor = editor("中文 abc 中文 ABC");
        assert!(open(&mut editor));
        set_query(&mut editor, "中文".into());
        assert_eq!(search(&editor).matches, vec![0..6, 11..17]);
        navigate(&mut editor, true);
        assert_eq!(search(&editor).active_match_idx, 1);
        navigate(&mut editor, false);
        assert_eq!(search(&editor).active_match_idx, 0);
        set_query(&mut editor, "abc".into());
        assert_eq!(search(&editor).match_count(), 2);
        assert_eq!(search(&editor).active_match_idx, 0);
    }

    #[test]
    fn document_search_refresh_and_close_preserve_query() {
        let mut editor = editor("one one");
        open(&mut editor);
        set_query(&mut editor, "one".into());
        assert!(!refresh(&mut editor));
        let tab_id = editor.active_tab_id().expect("active test tab");
        editor
            .tab_session_mut(tab_id)
            .expect("test tab session")
            .document
            .replace_range(0..3, "two");
        assert!(refresh(&mut editor));
        assert_eq!(search(&editor).matches, vec![4..7]);
        close(&mut editor);
        assert_eq!(search(&editor).query, "one");
        assert!(!search(&editor).panel_visible);
        assert!(search(&editor).matches.is_empty());
        editor
            .tab_session_mut(tab_id)
            .expect("test tab session")
            .document
            .cursor_mut()
            .selection_anchor = None;
        open(&mut editor);
        assert_eq!(search(&editor).matches, vec![4..7]);
        set_query(&mut editor, "absent".into());
        assert!(search(&editor).matches.is_empty());
        navigate(&mut editor, true);
        set_query(&mut editor, String::new());
        assert!(search(&editor).matches.is_empty());
    }
    #[test]
    fn document_search_tabs_read_only_and_single_line_selection() {
        let mut editor = editor("one two\nthree");
        let first = editor.active_tab_id().expect("first tab");
        {
            let tab = editor.tab_session_mut(first).expect("first session");
            tab.document.set_cursor_offset_synced(3);
            tab.document.cursor_mut().selection_anchor = Some(0);
            tab.runtime
                .set_editing_access(appkit_shell::tab_runtime::DocumentEditingAccess::ReadOnly);
        }
        assert!(open(&mut editor));
        assert_eq!(search(&editor).query, "one");
        assert_eq!(search(&editor).matches, vec![0..3]);
        assert_eq!(
            editor.tab_session(first).expect("first session").document.full_text(),
            "one two\nthree"
        );
        install(&mut editor, "two two");
        assert!(open(&mut editor));
        set_query(&mut editor, "two".into());
        assert_eq!(search(&editor).match_count(), 2);
        editor.activate(first);
        assert_eq!(search(&editor).query, "one");
        {
            let tab = editor.tab_session_mut(first).expect("first session");
            tab.document.set_cursor_offset_synced(9);
            tab.document.cursor_mut().selection_anchor = Some(0);
        }
        open(&mut editor);
        assert_eq!(search(&editor).query, "one");
    }

    #[test]
    fn document_search_matches_across_gap_once() {
        let mut editor = editor("ABC abc abc");
        let tab_id = editor.active_tab_id().expect("active test tab");
        editor.tab_session_mut(tab_id).expect("test session").document.replace_range(5..5, "x");
        open(&mut editor);
        set_query(&mut editor, "axbc".into());
        assert_eq!(search(&editor).matches, vec![4..8]);
        set_query(&mut editor, "abc".into());
        assert_eq!(search(&editor).matches, vec![0..3, 9..12]);
    }

    #[test]
    fn document_search_rejects_mindmap_and_large_selection() {
        let mut editor = editor(&"a".repeat(2049));
        let tab_id = editor.active_tab_id().expect("active test tab");
        {
            let tab = editor.tab_session_mut(tab_id).expect("test session");
            tab.document.set_cursor_offset_synced(2049);
            tab.document.cursor_mut().selection_anchor = Some(0);
        }
        assert!(open(&mut editor));
        assert!(search(&editor).query.is_empty());
        let plugin = editor.create_plugin_by_name(ui::plugin::PLUGIN_MINDMAP);
        editor.tab_session_mut(tab_id).expect("test session").runtime.plugin = plugin;
        assert!(!open(&mut editor));
    }
}
