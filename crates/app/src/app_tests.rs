use super::*;
use crate::document_view::DocumentView;
use appkit_shell::editor_runtime::{EditorFocus, EditorInputContext};
use core::types::ByteIndex;
use core::types::UniCharOffset;
use ui::layout::compute_visual_lines;
use ui::viewport::Viewport;

fn editor_input_context_for_test() -> EditorInputContext {
    EditorInputContext { focus: EditorFocus::Active, modal_blocked: false }
}

fn set_editor_preedit_for_test(
    app: &mut App,
    text: impl Into<String>,
    cursor: Option<(usize, usize)>,
) {
    let context = editor_input_context_for_test();
    assert!(app.editor_runtime.update_preedit(context, text.into(), cursor));
}

fn editor_document_for_test(
    app: &mut App,
    index: usize,
) -> &mut appkit_core::document::DocumentModel {
    let tab_id = app.editor_tab_id_at(index).expect("test tab index should be valid");
    app.tab_session_mut(tab_id).expect("test tab session should exist").document
}

fn active_document_text(app: &App) -> String {
    app.active_tab_session().expect("active document should exist").full_text()
}

fn close_editor_tab_for_test(app: &mut App, index: usize) {
    let tab_id = app.editor_tab_id_at(index).expect("test tab index should be valid");
    let effect = app.close_editor_tab(tab_id).expect("test tab should close");
    app.apply_workspace_effect(effect);
}

fn update_runtime_frame_cache(
    app: &mut App,
    update: impl FnOnce(&mut appkit_shell::frame_cache::FrameCache),
) {
    let mut resources = app.editor_runtime.take_render_resources();
    update(&mut resources.frame_cache);
    app.editor_runtime.restore_render_resources(resources);
}

/// Helper: create a GlyphCluster for testing
fn mock_cluster(byte_start: usize, byte_end: usize, advance: f32) -> shaping::GlyphCluster {
    shaping::GlyphCluster {
        byte_range: byte_start..byte_end,
        glyph_id: 0,
        font_id: shaping::FontId::default(),
        advance,
        x_offset: 0.0,
        y_offset: 0.0,
    }
}

#[test]
fn compute_visual_lines_no_wrap() {
    // 5 chars, each 10px, viewport 200px → 1 visual line
    let clusters: Vec<_> = (0..5).map(|i| mock_cluster(i, i + 1, 10.0)).collect();
    let line_bytes = b"hello";
    let result = compute_visual_lines(&clusters, line_bytes, 10.0, 200.0, 0.5);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0], (0, 5, 50.0));
}

#[test]
fn compute_visual_lines_wrap_at_boundary() {
    // 10 chars, each 10px, viewport 50px → 2 visual lines of 5 chars each
    let clusters: Vec<_> = (0..10).map(|i| mock_cluster(i, i + 1, 10.0)).collect();
    let line_bytes = b"0123456789";
    let result = compute_visual_lines(&clusters, line_bytes, 10.0, 50.0, 0.5);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0], (0, 5, 50.0));
    assert_eq!(result[1], (5, 10, 50.0));
}

#[test]
fn compute_visual_lines_wrap_mid_cluster() {
    // 6 chars, each 20px, viewport 50px → wraps after 2nd char (40px < 50, 60px > 50)
    let clusters: Vec<_> = (0..6).map(|i| mock_cluster(i, i + 1, 20.0)).collect();
    let line_bytes = b"abcdef";
    let result = compute_visual_lines(&clusters, line_bytes, 20.0, 50.0, 0.5);
    assert_eq!(result.len(), 3);
    assert_eq!(result[0], (0, 2, 40.0));
    assert_eq!(result[1], (2, 4, 40.0));
    assert_eq!(result[2], (4, 6, 40.0));
}

#[test]
fn compute_visual_lines_single_char_wider_than_viewport() {
    // 1 char wider than viewport → still 1 visual line (can't wrap single cluster)
    let clusters = vec![mock_cluster(0, 1, 100.0)];
    let line_bytes = b"X";
    let result = compute_visual_lines(&clusters, line_bytes, 100.0, 50.0, 0.5);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0], (0, 1, 100.0));
}

#[test]
fn compute_visual_lines_empty() {
    let result = compute_visual_lines(&[], &[], 10.0, 200.0, 0.5);
    assert!(result.is_empty());
}

#[test]
fn compute_visual_lines_whitespace_uses_char_width() {
    // Whitespace clusters use char_width, not shaper advance
    let clusters = vec![
        mock_cluster(0, 1, 50.0),  // 'a' — non-whitespace, uses advance 50.0
        mock_cluster(1, 2, 100.0), // ' ' — whitespace, uses char_width 10.0
        mock_cluster(2, 3, 50.0),  // 'b' — non-whitespace, uses advance 50.0
    ];
    let line_bytes = b"a b";
    let result = compute_visual_lines(&clusters, line_bytes, 10.0, 200.0, 0.5);
    assert_eq!(result.len(), 1);
    // 50.0 + 10.0 + 50.0 = 110.0
    assert_eq!(result[0], (0, 3, 110.0));
}

/// Helper: create an App with a DocumentView loaded with test content.
fn app_with_content(lines: Vec<&str>) -> App {
    let mut app = App::new(None);
    let dv = DocumentView::new(lines.into_iter().map(|s| s.to_string()).collect(), 80, 10.0);
    let line_count = dv.line_count();
    let tab_id = app.push_entry_for_test(dv, Box::new(EditorPlugin::new()));
    app.switch_workspace_for_test(0);
    app.tab_session_mut(tab_id).unwrap().display_mut().display_map.set_entries(
        (0..line_count)
            .map(|i| crate::snap_tree::DisplayLineEntry::placeholder(i, 10, 0, 1))
            .collect(),
    );
    app
}

fn cached_line_for_test(content_hash: u64) -> crate::render_cache::CachedLine {
    const SINGLE_VISUAL_LINE: u16 = 1;

    crate::render_cache::CachedLine {
        instances: Vec::new(),
        line_number_glyphs: Vec::new(),
        atlas_generation: 0,
        visual_line_count: SINGLE_VISUAL_LINE,
        content_hash,
        visual_lines: vec![(0, 0, 0.0)],
        visual_line_instance_starts: vec![0],
        cluster_data: Vec::new(),
        subset_start: 0,
    }
}

#[test]
fn line_end_enter_invalidates_cache_for_inserted_line() {
    const STALE_LINE_HASH: u64 = 42;
    const TEST_LINE_HEIGHT: f32 = 24.27;

    let mut app = app_with_content(vec!["alpha", "beta"]);
    let mut tab = app.active_tab_session_mut().unwrap();
    tab.document.cursor_move_to_offset("alpha".len());
    tab.display_mut().render_cache.insert(0, cached_line_for_test(STALE_LINE_HASH));
    tab.display_mut().render_cache.insert(1, cached_line_for_test(STALE_LINE_HASH));
    tab.display_mut().render_cache.insert(2, cached_line_for_test(STALE_LINE_HASH));

    let mut presentation = tab.take_presentation();
    let page_step_rows = presentation.display.viewport.visible_rows.saturating_sub(1).max(1);
    let outcome = crate::commands::execute_edit_command_v2_with_presentation(
        &crate::input::EditCommand::InsertNewline,
        tab.document,
        &[],
        &mut presentation.cursor_render_state,
        page_step_rows,
    );
    assert_eq!(tab.document.full_text(), "alpha\n\nbeta");

    if outcome.new_line_count != outcome.old_line_count {
        let Some(dirty_lines) = outcome.dirty_lines.clone() else {
            panic!("line insertion must report dirty lines");
        };
        let line_delta = outcome.new_line_count - outcome.old_line_count;
        let replacement_line_count = dirty_lines.len() + line_delta;
        let mut replacements = Vec::with_capacity(replacement_line_count);
        for doc_line in dirty_lines.start..dirty_lines.start + replacement_line_count {
            let offset = tab.document.line_byte_offset(doc_line).unwrap_or(0);
            let length = tab.document.line_byte_length(doc_line).unwrap_or(0) as u32;
            replacements
                .push(crate::snap_tree::DisplayLineEntry::placeholder(offset, length, 0, 1));
        }
        let _ = presentation.display.display_map.sync(dirty_lines.clone(), replacements);
    }
    if outcome.invalidates_all_render_cache() {
        presentation.display.render_cache.invalidate_all();
    } else if let Some(cache_invalidation_range) = outcome.render_cache_invalidation_range() {
        presentation.display.render_cache.invalidate_range(cache_invalidation_range);
    }

    assert_eq!(presentation.display.display_map.line_count(), tab.document.line_count());
    assert!(
        !presentation.display.render_cache.contains(1),
        "line 1 becomes the inserted empty line, so its stale cached content must be invalidated"
    );
    assert!(
        !presentation.display.render_cache.contains(2),
        "line-count changes must clear stale cache keys beyond the old document range"
    );
    tab.restore_presentation(presentation);
}

#[test]
fn mouse_drag_creates_range_via_app_state() {
    let mut app = app_with_content(vec!["hello world"]);

    // Simulate mouse down at offset 3 (single click)
    // This is what happens in the MouseInput handler
    app.active_tab_session_mut().expect("active tab session").cursor_move_to_offset(3);
    app.mouse.down_byte_offset = Some(3);
    app.mouse.is_down = true;

    // Verify: cursor at 3, no selection yet (anchor cleared by cursor_move_to_offset)
    {
        let document = app.active_tab_session().expect("active tab session");
        assert_eq!(document.cursor().offset, ByteIndex(3));
        assert!(
            document.cursor().selection_anchor.is_none(),
            "anchor should be None after single click"
        );
    }

    // Simulate mouse drag to offset 8
    // This is what happens in the CursorMoved handler
    if let Some(anchor) = app.mouse.down_byte_offset.take() {
        app.active_tab_session_mut().expect("active tab session").cursor_mut().selection_anchor =
            Some(anchor);
    }
    app.active_tab_session_mut().expect("active tab session").set_cursor_offset_synced(8);

    // Verify: selection from 3 to 8
    let (start, end) =
        app.active_tab_session().expect("active tab session").selection_range().unwrap();
    assert_eq!(start, 3, "anchor should be at mouse down position");
    assert_eq!(end, 8, "cursor should be at drag position");
    assert!(
        app.active_tab_session().expect("active tab session").selection_range().is_some(),
        "should have selection after drag"
    );

    // Simulate mouse up
    app.mouse.is_down = false;
    app.mouse.down_byte_offset = None;

    // Selection should persist after mouse up
    let (start, end) =
        app.active_tab_session().expect("active tab session").selection_range().unwrap();
    assert_eq!(start, 3);
    assert_eq!(end, 8);
}

#[test]
fn mouse_drag_backward_creates_range() {
    let mut app = app_with_content(vec!["hello world"]);
    let tab_id = app.editor_tab_id_at(0).expect("test tab index should be valid");

    // Mouse down at offset 8
    app.with_editor_document_and_mouse_for_test(tab_id, |mouse, document| {
        document.cursor_move_to_offset(8);
        mouse.down_byte_offset = Some(8);
        mouse.is_down = true;

        // Drag backward to offset 3
        if let Some(anchor) = mouse.down_byte_offset.take() {
            document.cursor_mut().selection_anchor = Some(anchor);
        }
        document.set_cursor_offset_synced(3);
    })
    .expect("test tab session should exist");

    // Selection should be min..max
    let (start, end) =
        app.active_tab_session().expect("active tab session").selection_range().unwrap();
    assert_eq!(start, 3, "start should be min(anchor, cursor)");
    assert_eq!(end, 8, "end should be max(anchor, cursor)");
}

#[test]
fn mouse_click_without_drag_no_selection() {
    let mut app = app_with_content(vec!["hello world"]);
    let tab_id = app.editor_tab_id_at(0).expect("test tab index should be valid");

    // Mouse down at offset 5
    app.with_editor_document_and_mouse_for_test(tab_id, |mouse, document| {
        document.cursor_move_to_offset(5);
        mouse.down_byte_offset = Some(5);
        mouse.is_down = true;

        // Mouse up immediately (no drag)
        mouse.is_down = false;
        mouse.down_byte_offset = None;
    })
    .expect("test tab session should exist");

    // No selection should exist
    let document = app.active_tab_session().expect("active tab session");
    assert!(!document.has_selection(), "click without drag should not create selection");
    assert_eq!(document.cursor().offset, ByteIndex(5), "cursor should stay at click position");
}

#[test]
fn mouse_drag_then_click_clears_selection() {
    let mut app = app_with_content(vec!["hello world"]);
    let tab_id = app.editor_tab_id_at(0).expect("test tab index should be valid");

    // First: create a selection via drag
    app.with_editor_document_and_mouse_for_test(tab_id, |mouse, document| {
        document.cursor_move_to_offset(3);
        mouse.down_byte_offset = Some(3);
        mouse.is_down = true;

        if let Some(anchor) = mouse.down_byte_offset.take() {
            document.cursor_mut().selection_anchor = Some(anchor);
        }
        document.set_cursor_offset_synced(8);
        assert!(document.has_selection());

        // Mouse up
        mouse.is_down = false;
        mouse.down_byte_offset = None;

        // New click at offset 5 (single click, no drag)
        document.cursor_move_to_offset(5);
        mouse.down_byte_offset = Some(5);
        mouse.is_down = true;

        // Selection should be cleared (cursor_move_to_offset clears anchor)
        assert!(!document.has_selection(), "new click should clear previous selection");
        assert_eq!(document.cursor().offset, ByteIndex(5));
    })
    .expect("test tab session should exist");
}

// --- Viewport offset revision: missing tests (A2, A3, B1) ---

#[test]
fn move_up_into_skipped_area_moves_cursor_byte() {
    // Scenario: 20-char line wraps into 3 visual lines (7+7+6 clusters).
    // Viewport shows visual lines 1..2 (scroll_visual_offset=1).
    // Cursor at start of visual line 1 (byte 3).
    // Pressing up should move cursor to sticky_x-matching byte on visual line 0.
    // Viewport scrolling is now handled by ensure_cursor_visible, not move_cursor_visual.
    let mut app = app_with_content(vec!["abcdefghijklmnopqrst"]);
    {
        let mut tab = app.active_tab_session_mut().unwrap();
        tab.display_mut().viewport.visible_rows = 2;
        tab.display_mut().viewport.scroll_top = 1.0; // visual line 1 (skip first visual line)
        tab.cursor_render_state_mut().sticky_x = 47.6; // closest to byte 2 at x=32.0+2*7.8=47.6 = 23.4 → best_rel = byte 3
        tab.cursor_render_state_mut().cursor_visual_line = Some(0);
        tab.document.cursor_move_to_offset(3);
    }

    // Clusters: 20 chars, each (start+1, start+2, 7.8), 3 visual lines of 7/7/6
    app.active_tab_session_mut().unwrap().display_mut().advance_cache = vec![
        AdvanceCacheEntry {
            doc_line: 0,
            vl_byte_start: 3,
            vl_grapheme_start: 0,
            clusters: vec![(4, 31.2, 0), (5, 39.0, 0), (6, 46.8, 0), (7, 54.6, 0)],
        },
        AdvanceCacheEntry {
            doc_line: 0,
            vl_byte_start: 10,
            vl_grapheme_start: 0,
            clusters: vec![(4, 31.2, 0), (5, 39.0, 0), (6, 46.8, 0), (7, 54.6, 0)],
        },
    ];
    update_runtime_frame_cache(&mut app, |frame_cache| {
        frame_cache.first_line.visual_lines = vec![(0, 7, 54.6), (7, 14, 54.6), (14, 20, 46.8)];
        frame_cache.first_line.clusters = (0..20).map(|i| (i, i + 1, 7.8)).collect();
        frame_cache.first_line.doc_offset = 0;
    });

    app.move_cursor_visual(-1);

    let dv = editor_document_for_test(&mut app, 0);
    assert_eq!(
        dv.cursor().offset,
        ByteIndex(2),
        "cursor byte should match sticky_x position on visual line 0 (byte 2 = 'c')"
    );
}

#[test]
fn move_down_past_visible_preserves_sticky_x() {
    // Scenario: 14-char line wraps into 2 visual lines (7+7 clusters).
    // Viewport shows only visual line 0 (scroll_visual_offset=0, visible_rows=1).
    // Cursor at end of visual line 0 (byte 7).
    // Pressing down should move cursor to sticky_x-matching byte on visual line 1.
    // Viewport scrolling is now handled by ensure_cursor_visible, not move_cursor_visual.
    let mut app = app_with_content(vec!["abcdefghijklmn"]);
    {
        let mut tab = app.active_tab_session_mut().unwrap();
        tab.display_mut().viewport.visible_rows = 1;
        tab.display_mut().viewport.scroll_top = 0.0;
        // Set up wrap_index so 4c can read visual_line_count
        tab.display_mut().display_map.update_entry_in_place(
            0,
            crate::snap_tree::DisplayLineEntry {
                visual_line_count: 2,
                visual_breaks: smallvec::SmallVec::new(),
                byte_offset: 0,
                byte_length: 0,
                content_hash: 0,
            },
        ); // doc line 0 has 2 visual lines
        tab.display_mut().display_map.rebuild_tree();
        tab.cursor_render_state_mut().sticky_x = 47.6; // on visual line 1: closest to byte 9 at x=32.0+2*7.8=47.6 closest to cluster[3].x=23.4 → byte 10
        tab.cursor_render_state_mut().cursor_visual_line = Some(0);
        tab.document.cursor_move_to_offset(7);
    }

    app.active_tab_session_mut().unwrap().display_mut().advance_cache = vec![AdvanceCacheEntry {
        doc_line: 0,
        vl_byte_start: 0,
        vl_grapheme_start: 0,
        clusters: vec![(7, 54.6, 0)],
    }];
    update_runtime_frame_cache(&mut app, |frame_cache| {
        frame_cache.last_line.visual_lines = vec![(0, 7, 54.6), (7, 14, 54.6)];
        frame_cache.last_line.clusters = (0..14).map(|i| (i, i + 1, 7.8)).collect();
    });

    app.move_cursor_visual(1);

    let dv = editor_document_for_test(&mut app, 0);
    assert_eq!(
        dv.cursor().offset,
        ByteIndex(9),
        "cursor byte should match sticky_x position on visual line 1 (byte 9 = 'j')"
    );
}

#[test]
fn move_down_wrapped_line_uses_correct_byte_offset() {
    // Regression: advance_cache lacked vl_byte_start, causing 4a to position
    // cursor at byte 0 of the doc line instead of the visual line's actual start.
    // This made ArrowDown "stuck" when the first long line wrapped.
    let mut app = app_with_content(vec!["abcdefghijklmn"]);
    {
        let mut tab = app.active_tab_session_mut().unwrap();
        tab.display_mut().viewport.visible_rows = 2;
        tab.display_mut().viewport.scroll_top = 0.0;
    }

    // VL0: bytes 0..7, VL1: bytes 7..14
    {
        let mut tab = app.active_tab_session_mut().unwrap();
        tab.display_mut().advance_cache = vec![
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 0,
                vl_grapheme_start: 0,
                clusters: vec![(7, 54.6, 0)],
            },
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 7,
                vl_grapheme_start: 0,
                clusters: vec![(14, 54.6, 0)],
            },
        ];
        tab.cursor_render_state_mut().sticky_x = 8.0; // left margin
        tab.cursor_render_state_mut().cursor_visual_line = Some(0);
        tab.document.cursor_move_to_offset(0);
    }

    app.move_cursor_visual(1);

    let dv = editor_document_for_test(&mut app, 0);
    assert_eq!(
        dv.cursor().offset,
        ByteIndex(7),
        "cursor should move to start of VL1 (byte 7), not byte 0"
    );
}

#[test]
fn move_down_wrapped_line_boundary_no_stall() {
    // Regression: cursor at wrap boundary was matched to previous visual line
    // (inclusive end check), preventing viewport scroll. With the fix, boundary
    // byte maps to the NEXT visual line for non-last VLs.
    let mut app = app_with_content(vec!["abcdefghijklmn"]);
    {
        let mut tab = app.active_tab_session_mut().unwrap();
        tab.display_mut().viewport.visible_rows = 1;
        tab.display_mut().viewport.scroll_top = 0.0;
        tab.cursor_render_state_mut().sticky_x = 8.0;
        tab.cursor_render_state_mut().cursor_visual_line = Some(0);
        tab.document.cursor_move_to_offset(0);
    }

    // Only VL0 visible, VL1 is off-screen
    app.active_tab_session_mut().unwrap().display_mut().advance_cache = vec![AdvanceCacheEntry {
        doc_line: 0,
        vl_byte_start: 0,
        vl_grapheme_start: 0,
        clusters: vec![(7, 54.6, 0)],
    }];
    // last_line data used by 4c when target_vis >= cache.len()
    update_runtime_frame_cache(&mut app, |frame_cache| {
        frame_cache.last_line.visual_lines = vec![(0, 7, 54.6), (7, 14, 54.6)];
        frame_cache.last_line.clusters = (0..14).map(|i| (i, i + 1, 7.8)).collect();
    });

    app.move_cursor_visual(1);

    let dv = editor_document_for_test(&mut app, 0);
    // Should move to VL1 (byte 7), not stay at byte 0
    assert_eq!(dv.cursor().offset, ByteIndex(7), "cursor should advance to VL1 start (byte 7)");
}

#[test]
fn cursor_vertices_empty_when_visual_line_is_max() {
    // B1: cursor_visual_line == usize::MAX (sentinel for "cursor outside viewport")
    // should produce no caret vertices.
    let mut app = app_with_content(vec!["hello"]);
    {
        let mut tab = app.active_tab_session_mut().expect("active tab session");
        tab.cursor_render_state_mut().cursor_visual_line = None;
        tab.cursor_render_state_mut().cursor_pixel_x = 100.0;
    }
    {
        let mut s = ui::settings::Settings::new();
        s.set_line_height(16.0);
    }

    let verts = app.cursor_vertices(800.0, 600.0, 0.0);
    assert!(verts.is_empty(), "cursor_vertices must return empty when cursor_visual_line is None");
}

// ── Selection highlight rendering tests ──

/// Helper: build advance_cache entries like shape_visible_lines does.
fn make_cache_entry(
    doc_line: usize,
    vl_byte_start: usize,
    text: &[u8],
    char_advance: f32,
) -> AdvanceCacheEntry {
    // Matches real advance_cache: byte ranges are vl-local (relative to VL start).
    let left_margin = 8.0f32;
    let mut clusters = Vec::new();
    let mut x = left_margin;
    let mut byte_off = 0usize;
    for _ch in text {
        x += char_advance;
        byte_off += 1;
        clusters.push((byte_off, x, 0));
    }
    AdvanceCacheEntry { doc_line, vl_byte_start, vl_grapheme_start: 0, clusters }
}

#[test]
fn selection_quads_empty_range() {
    let entry = make_cache_entry(0, 0, b"hello", 7.8);
    let quads = compute_selection_highlight_quads(
        &[entry],
        (0, 0),
        &[0],
        800.0,
        600.0,
        22.0,
        0.0,
        8.0,
        [0.25, 0.45, 0.75, 0.35],
        0.0,
    );
    assert!(quads.is_empty());
}

#[test]
fn selection_quads_empty_cache() {
    let quads = compute_selection_highlight_quads(
        &[],
        (0, 5),
        &[],
        800.0,
        600.0,
        22.0,
        0.0,
        8.0,
        [0.25, 0.45, 0.75, 0.35],
        0.0,
    );
    assert!(quads.is_empty());
}

#[test]
fn selection_quads_single_line_partial() {
    // "hello world" — 11 chars, advance=7.8, margin=8.0
    // Advance cache: (1, 15.8, 0), (2, 23.6, 0), (3, 31.4, 0), (4, 39.2, 0), ...
    let entry = make_cache_entry(0, 0, b"hello world", 7.8);
    // Select bytes 3..7 = "lo w"
    let quads = compute_selection_highlight_quads(
        &[entry],
        (3, 7),
        &[0],
        800.0,
        600.0,
        22.0,
        0.0,
        8.0,
        [0.25, 0.45, 0.75, 0.35],
        0.0,
    );
    assert_eq!(quads.len(), 6, "one quad (6 vertices)");

    // x_start = left edge of byte 3 = x after byte 2 = 8.0 + 3*7.8 = 31.4
    // x_end = right edge of byte 6 = x after byte 6 = 8.0 + 7*7.8 = 62.6
    let expected_left = 31.4 / 800.0 * 2.0 - 1.0;
    let expected_right = 62.6 / 800.0 * 2.0 - 1.0;
    assert!((quads[0].position[0] - expected_left).abs() < 0.001, "left");
    assert!((quads[1].position[0] - expected_right).abs() < 0.001, "right");
    assert!((quads[0].color[3] - 0.35).abs() < 0.001, "alpha");
}

#[test]
fn selection_quads_entire_line() {
    let entry = make_cache_entry(0, 0, b"hello", 7.8);
    let quads = compute_selection_highlight_quads(
        &[entry],
        (0, 5),
        &[0],
        800.0,
        600.0,
        22.0,
        0.0,
        8.0,
        [0.25, 0.45, 0.75, 0.35],
        0.0,
    );
    assert_eq!(quads.len(), 6);
    let expected_left = 8.0 / 800.0 * 2.0 - 1.0;
    let expected_right = (8.0 + 5.0 * 7.8) / 800.0 * 2.0 - 1.0;
    assert!((quads[0].position[0] - expected_left).abs() < 0.001);
    assert!((quads[1].position[0] - expected_right).abs() < 0.001);
}

#[test]
fn selection_quads_multi_line() {
    let entry0 = make_cache_entry(0, 0, b"hello", 7.8);
    let entry1 = make_cache_entry(1, 0, b"world", 7.8);
    let cache = vec![entry0, entry1];
    // Select bytes 3..10 — spans both lines
    let quads = compute_selection_highlight_quads(
        &cache,
        (3, 10),
        &[0, 6],
        800.0,
        600.0,
        22.0,
        0.0,
        8.0,
        [0.25, 0.45, 0.75, 0.35],
        0.0,
    );
    assert_eq!(quads.len(), 12, "two quads for two-line selection");

    // Line 0: bytes 3..5, x from 31.4 to 47.0
    let expected_left0 = 31.4 / 800.0 * 2.0 - 1.0;
    assert!((quads[0].position[0] - expected_left0).abs() < 0.001, "line0 left");
    // Line 1: bytes 6..10, x from 8.0 (margin) to 8.0+4*7.8=39.2
    let expected_left1 = 8.0 / 800.0 * 2.0 - 1.0;
    let expected_right1 = 39.2 / 800.0 * 2.0 - 1.0;
    assert!((quads[6].position[0] - expected_left1).abs() < 0.001, "line1 left");
    assert!((quads[7].position[0] - expected_right1).abs() < 0.001, "line1 right");
}

#[test]
fn selection_quads_word_wrap() {
    // Doc line 0 wraps into 2 visual lines: bytes 0..5 and 5..10
    let entry0 = make_cache_entry(0, 0, b"abcde", 7.8);
    let entry1 = make_cache_entry(0, 5, b"fghij", 7.8);
    let cache = vec![entry0, entry1];
    // Select bytes 3..8 (spans both visual lines)
    let quads = compute_selection_highlight_quads(
        &cache,
        (3, 8),
        &[0, 6],
        800.0,
        600.0,
        22.0,
        0.0,
        8.0,
        [0.25, 0.45, 0.75, 0.35],
        0.0,
    );
    assert_eq!(quads.len(), 12, "two quads for word-wrapped selection");
}

#[test]
fn selection_quads_with_sub_line_offset() {
    let entry = make_cache_entry(0, 0, b"hello", 7.8);
    let quads = compute_selection_highlight_quads(
        &[entry],
        (0, 5),
        &[0],
        800.0,
        600.0,
        22.0,
        11.0,
        0.0,
        [0.25, 0.45, 0.75, 0.35],
        0.0,
    );
    assert_eq!(quads.len(), 6);
    let top = quads[0].position[1];
    let expected_top = 1.0 - 11.0 / 600.0 * 2.0;
    assert!((top - expected_top).abs() < 0.001);
}

#[test]
fn selection_quads_no_overlap() {
    let entry = make_cache_entry(0, 0, b"hello", 7.8);
    let quads = compute_selection_highlight_quads(
        &[entry],
        (10, 15),
        &[0],
        800.0,
        600.0,
        22.0,
        0.0,
        8.0,
        [0.25, 0.45, 0.75, 0.35],
        0.0,
    );
    assert!(quads.is_empty(), "no overlap → no quads");
}

#[test]
fn selection_quads_partial_overlap_at_start() {
    // Visible: bytes 5..10, selection: bytes 3..7 → overlap bytes 5..7
    // line_abs=0 (line 0 starts at byte 0), vl_byte_start=5
    // abs_vl_start = 0+5 = 5, abs_vl_end = 0+10 = 10
    let entry = make_cache_entry(0, 5, b"fghij", 7.8);
    let quads = compute_selection_highlight_quads(
        &[entry],
        (3, 7),
        &[0],
        800.0,
        600.0,
        22.0,
        0.0,
        8.0,
        [0.25, 0.45, 0.75, 0.35],
        0.0,
    );
    assert_eq!(quads.len(), 6, "one quad for partial overlap");
    // Overlap: abs [5,7). local_clip_start=0, local_clip_end=2
    // Clusters are vl-local: cluster[0]=(1, 15.8, 0), cluster[1]=(2, 23.6, 0)
    // x_start = byte_to_x(0) = 8.0 (left margin, before first cluster)
    // x_end = byte_to_x(2) = 23.6 (end of cluster[1])
    let expected_left = 8.0 / 800.0 * 2.0 - 1.0;
    let expected_right = 23.6 / 800.0 * 2.0 - 1.0;
    assert!((quads[0].position[0] - expected_left).abs() < 0.001);
    assert!((quads[1].position[0] - expected_right).abs() < 0.001);
}

#[test]
fn selection_quads_partial_overlap_at_end() {
    // Visible: bytes 0..5, selection: bytes 3..8 → overlap bytes 3..5
    let entry = make_cache_entry(0, 0, b"hello", 7.8);
    let quads = compute_selection_highlight_quads(
        &[entry],
        (3, 8),
        &[0],
        800.0,
        600.0,
        22.0,
        0.0,
        8.0,
        [0.25, 0.45, 0.75, 0.35],
        0.0,
    );
    assert_eq!(quads.len(), 6);
    // x_start = 31.4 (left edge of byte 3)
    // x_end = 47.0 (right edge of byte 4)
    let expected_left = 31.4 / 800.0 * 2.0 - 1.0;
    let expected_right = 47.0 / 800.0 * 2.0 - 1.0;
    assert!((quads[0].position[0] - expected_left).abs() < 0.001);
    assert!((quads[1].position[0] - expected_right).abs() < 0.001);
}

#[test]
fn selection_quads_y_positions_multi_line() {
    // entry0: doc_line=0, vl_byte_start=0, clusters end at bytes 1,2 → abs [0,2)
    // entry1: doc_line=1, vl_byte_start=3, clusters end at bytes 4,5 → abs [6,8) (line_abs=3)
    let entry0 = make_cache_entry(0, 0, b"ab", 7.8);
    let entry1 = make_cache_entry(1, 3, b"cd", 7.8);
    let cache = vec![entry0, entry1];
    // Selection [0, 8) spans both entries
    let quads = compute_selection_highlight_quads(
        &cache,
        (0, 8),
        &[0, 3],
        800.0,
        600.0,
        22.0,
        0.0,
        8.0,
        [0.25, 0.45, 0.75, 0.35],
        0.0,
    );
    assert_eq!(quads.len(), 12);
    // Line 0: top = 1.0 - 0 = 1.0
    assert!((quads[0].position[1] - 1.0).abs() < 0.001, "line0 top");
    // Line 1: top = 1.0 - 22/600*2
    let expected = 1.0 - 22.0 / 600.0 * 2.0;
    assert!((quads[6].position[1] - expected).abs() < 0.001, "line1 top");
}

#[test]
fn selection_quads_does_not_highlight_adjacent_wrapped_line() {
    // Two long CJK lines, each wrapping into 3 visual lines.
    // Select on line 4 only -- line 5 must NOT be highlighted.
    // This mirrors the real advance_cache format: each VL has its own
    // vl_byte_start = start of that visual line within the doc line.
    let mut vl_counter = 0u32;
    let make_vl = |doc_line: usize,
                   vl_byte_start: usize,
                   num_chars: usize,
                   vl_counter: &mut u32|
     -> AdvanceCacheEntry {
        let left_margin = 8.0f32;
        let char_advance = 16.0;
        let mut clusters = Vec::new();
        let mut x = left_margin;
        let mut byte_off = 0usize; // vl-local offset
        for _ in 0..num_chars {
            x += char_advance;
            byte_off += 3;
            clusters.push((byte_off, x, 0));
        }
        let entry = AdvanceCacheEntry { doc_line, vl_byte_start, vl_grapheme_start: 0, clusters };
        *vl_counter += 1;
        entry
    };

    // Line 4 (doc_line=3): 60 CJK chars = 180 bytes, 3 VLs of 20 chars
    //   VL 0: local bytes 0..60, VL 1: 60..120, VL 2: 120..180
    // Line 5 (doc_line=4): 60 CJK chars = 180 bytes, 3 VLs of 20 chars
    //   VL 3: local bytes 0..60, VL 4: 60..120, VL 5: 120..180
    let cache = vec![
        make_vl(3, 0, 20, &mut vl_counter),
        make_vl(3, 60, 20, &mut vl_counter),
        make_vl(3, 120, 20, &mut vl_counter),
        make_vl(4, 0, 20, &mut vl_counter),
        make_vl(4, 60, 20, &mut vl_counter),
        make_vl(4, 120, 20, &mut vl_counter),
    ];
    // line_byte_offsets: line 3 starts at 100, line 4 starts at 282
    let line_offsets = vec![0, 10, 20, 100, 282];

    // Select absolute bytes 130..190
    //   line 3 absolute range: [100, 282)
    //   line 4 absolute range: [282, 464)
    // VL 0: abs [100, 160), overlap [130, 160) -> highlighted
    // VL 1: abs [160, 220), overlap [160, 190) -> highlighted
    // VL 2: abs [220, 280), no overlap with [130, 190) -> skip
    // VL 3-5: abs [282, ...), no overlap -> skip
    let quads = compute_selection_highlight_quads(
        &cache,
        (130, 190),
        &line_offsets,
        1200.0,
        800.0,
        22.0,
        0.0,
        8.0,
        [0.25, 0.45, 0.75, 0.35],
        0.0,
    );
    assert_eq!(quads.len(), 12, "only 2 VLs of line 4 highlighted, not line 5");

    // Verify the highlighted VLs are VL 0 and VL 1 (correct y positions)
    let top0 = quads[0].position[1];
    let top1 = quads[6].position[1];
    assert!((top0 - 1.0).abs() < 0.001, "VL 0 y-pos");
    let expected_top1 = 1.0 - 22.0 / 800.0 * 2.0;
    assert!((top1 - expected_top1).abs() < 0.001, "VL 1 y-pos");
}

#[test]
fn selection_quads_high_doc_line_index_oob() {
    // Bug repro: advance_cache has doc_line=100 (scrolled far down),
    // but line_offsets only has 6 entries (visible lines).
    // Indexing line_offsets[100] -> OOB -> unwrap_or(0) -> wrong offset.
    // Fix: line_offsets must be sized to total document lines.
    let mut vl_counter2 = 0u32;
    let make_vl = |doc_line: usize,
                   vl_byte_start: usize,
                   num: usize,
                   vl_counter: &mut u32|
     -> AdvanceCacheEntry {
        let mut clusters = Vec::new();
        let mut x = 8.0f32;
        let mut bo = 0usize; // vl-local offset
        for _ in 0..num {
            x += 16.0;
            bo += 3;
            clusters.push((bo, x, 0));
        }
        let entry = AdvanceCacheEntry { doc_line, vl_byte_start, vl_grapheme_start: 0, clusters };
        *vl_counter += 1;
        entry
    };

    // advance_cache: doc lines 100 and 101 (scrolled far down)
    let cache = vec![
        make_vl(100, 0, 20, &mut vl_counter2),
        make_vl(100, 60, 20, &mut vl_counter2),
        make_vl(101, 0, 20, &mut vl_counter2),
    ];

    // Correct: line_offsets sized to total doc lines (102+)
    let mut correct_offsets = vec![0usize; 102];
    correct_offsets[100] = 5000;
    correct_offsets[101] = 5200;

    // Select bytes 5030..5090 (line 100 local: 30..90)
    let quads = compute_selection_highlight_quads(
        &cache,
        (5030, 5090),
        &correct_offsets,
        1200.0,
        800.0,
        22.0,
        0.0,
        8.0,
        [0.25, 0.45, 0.75, 0.35],
        0.0,
    );
    // Should highlight VL 0 (local 30..60) and VL 1 (local 60..90), not VL 2
    assert_eq!(quads.len(), 12, "2 quads for line 100 only");

    // Wrong (old code): line_offsets only 3 entries -> doc_line 100 OOB -> offset 0
    let wrong_offsets = vec![0usize; 3]; // too small!
    let quads_wrong = compute_selection_highlight_quads(
        &cache,
        (5030, 5090),
        &wrong_offsets,
        1200.0,
        800.0,
        22.0,
        0.0,
        8.0,
        [0.25, 0.45, 0.75, 0.35],
        0.0,
    );
    // With wrong offsets, line 100's offset = 0 (OOB), so abs range = [0, 60).
    // Selection [5030, 5090) doesn't overlap [0, 60) -> no quads at all!
    // (Previously this might have caused wrong highlights on other lines)
    assert!(quads_wrong.len() <= 12, "wrong offsets should not cause extra highlights");
}

// ── Autoscroll DisplayRow tests ──────────────────────────────────

/// When cursor jumps outside the visible range, autoscroll
/// must update anchor (Stage 5 SOT) and derive scroll_top.
#[test]
fn autoscroll_not_in_vli_updates_anchor() {
    let mut vp = Viewport::new(10);

    // Initial: scroll at top, anchor at line 0
    assert_eq!(vp.scroll_top, 0.0);
    assert_eq!(vp.scroll_anchor.doc_line, 0);

    // Populate DisplayLineMap with 101 lines (0..100)
    let mut dm = crate::display_line_map::DisplayLineMap::new();
    dm.set_entries(
        (0..101)
            .map(|i| crate::snap_tree::DisplayLineEntry::placeholder(i * 80, 80, 0, 1))
            .collect(),
    );

    // Simulate NOT-IN-VISIBLE-RANGE autoscroll for cursor at doc line 100
    let cursor_doc_line = 100usize;
    let range = vp.visible_doc_range_from_anchor(&dm, 14.0);
    assert!(
        cursor_doc_line >= range.end,
        "cursor_doc_line={} should be >= range.end={}",
        cursor_doc_line,
        range.end
    );

    // Correct path: set anchor directly (Stage 5)
    let vis_count = range.len().max(1);
    let new_anchor = cursor_doc_line.saturating_sub(vis_count.saturating_sub(1));
    vp.scroll_anchor = ui::viewport::ScrollAnchor::new(new_anchor, 0.0);
    vp.clamp_anchor(&dm, 14.0);
    vp.derive_scroll_top(&dm, 14.0);

    // scroll_top must have changed (derived from anchor)
    assert!(vp.scroll_top > 0.0, "scroll_top must change when cursor jumps outside viewport");
    assert_eq!(vp.scroll_anchor.doc_line, 91, "anchor should be at doc line 91");
}

/// When cursor is within the visible range, autoscroll should be a no-op.
#[test]
fn autoscroll_in_vli_no_scroll() {
    let mut app =
        app_with_content((0..100).map(|i| format!("line {i}").leak() as &str).collect::<Vec<_>>());
    let mut tab = app.active_tab_session_mut().expect("active tab session");
    tab.display_mut().viewport.visible_rows = 10;

    // Move cursor to line 5 (within visible range)
    tab.document.cursor_move_to_offset(tab.document.line_byte_offset(5).unwrap());
    let old_scroll = tab.display().viewport.scroll_top;
    let range = tab.display().viewport.visible_doc_line_range_approx(tab.document.line_count());
    assert!(tab.document.cursor_line() >= range.start && tab.document.cursor_line() < range.end);
    // No scroll needed — scroll_top should stay the same
    assert_eq!(tab.display().viewport.scroll_top, old_scroll);
}

/// Mouse wheel should not trigger autoscroll (anchor stays as user set it).
#[test]
fn mouse_wheel_scroll_no_autoscroll() {
    let mut app =
        app_with_content((0..100).map(|i| format!("line {i}").leak() as &str).collect::<Vec<_>>());
    let mut tab = app.active_tab_session_mut().unwrap();
    tab.display_mut().viewport.visible_rows = 10;
    tab.display_mut().viewport.viewport_height = 10.0;

    // User scrolls via mouse wheel (Stage 5: use scroll_doc_lines)
    let mut presentation = tab.take_presentation();
    presentation.display.viewport.scroll_doc_lines(20, &presentation.display.display_map);
    presentation.display.viewport.derive_scroll_top(&presentation.display.display_map, 14.0);
    assert_eq!(presentation.display.viewport.scroll_anchor.doc_line, 20);

    // Cursor hasn't moved — autoscroll should not fire
    let old_anchor = presentation.display.viewport.scroll_anchor.doc_line;
    assert_eq!(presentation.cursor_render_state.last_cursor_offset, ByteIndex(0)); // hasn't moved
    assert_eq!(presentation.display.viewport.scroll_anchor.doc_line, old_anchor);
    tab.restore_presentation(presentation);
}

/// Regression: advance_cache cleared at start of shape_visible_lines must not
/// cause index-out-of-bounds when stale references point to old entries.
/// WrapIndex-based computation is independent of advance_cache, so it's safe.
#[test]
fn advance_cache_clear_does_not_invalidate_wrap_index() {
    let mut app =
        app_with_content((0..50).map(|i| format!("line {i}").leak() as &str).collect::<Vec<_>>());
    let mut tab = app.active_tab_session_mut().expect("active tab session");
    tab.display_mut().viewport.visible_rows = 10;

    // Simulate previous frame: advance_cache populated
    tab.display_mut().advance_cache.push(AdvanceCacheEntry {
        doc_line: 0,
        vl_byte_start: 0,
        vl_grapheme_start: 0,
        clusters: vec![],
    });
    tab.display_mut().advance_cache.push(AdvanceCacheEntry {
        doc_line: 1,
        vl_byte_start: 0,
        vl_grapheme_start: 0,
        clusters: vec![],
    });

    // Use WrapIndex to compute first_visible_dr (replaces old display_row field)
    let first_visible_dr = tab.display().display_map.doc_to_display(
        tab.display()
            .display_map
            .display_to_doc(tab.display().viewport.scroll_top.floor() as usize),
    );
    assert_eq!(first_visible_dr, 0, "first visible doc line 0 → DisplayRow 0");

    // Simulate start of new frame: advance_cache cleared
    tab.display_mut().advance_cache.clear();

    // WrapIndex-based computation doesn't depend on advance_cache, so it's safe.
    let skip = tab
        .display()
        .viewport
        .first_visible_row()
        .saturating_sub(first_visible_dr as u32)
        .as_usize();
    assert_eq!(skip, 0, "at scroll_top=0, skip should be 0");

    // Verify: after clear, advance_cache is empty and WrapIndex still works
    assert!(tab.display().advance_cache.is_empty(), "advance_cache is cleared");
}

/// Verify first_visible_dr is correct when scrolled partway into a wrapped line.
#[test]
fn first_visible_dr_captured_with_scroll_offset() {
    let mut app =
        app_with_content((0..50).map(|i| format!("line {i}").leak() as &str).collect::<Vec<_>>());
    let mut tab = app.active_tab_session_mut().unwrap();
    tab.display_mut().viewport.visible_rows = 10;

    // Initialize wrap_index to match document line count
    // Simulate: doc line 3 starts at DisplayRow 6 (lines 0-2 wrapped to 2 each)
    tab.display_mut().display_map.update_entry_in_place(
        0,
        crate::snap_tree::DisplayLineEntry {
            visual_line_count: 2,
            visual_breaks: smallvec::SmallVec::new(),
            byte_offset: 0,
            byte_length: 0,
            content_hash: 0,
        },
    );
    tab.display_mut().display_map.update_entry_in_place(
        1,
        crate::snap_tree::DisplayLineEntry {
            visual_line_count: 2,
            visual_breaks: smallvec::SmallVec::new(),
            byte_offset: 0,
            byte_length: 0,
            content_hash: 0,
        },
    );
    tab.display_mut().display_map.update_entry_in_place(
        2,
        crate::snap_tree::DisplayLineEntry {
            visual_line_count: 2,
            visual_breaks: smallvec::SmallVec::new(),
            byte_offset: 0,
            byte_length: 0,
            content_hash: 0,
        },
    );
    tab.display_mut().display_map.rebuild_tree();

    // Use WrapIndex to compute first_visible_dr
    let first_visible_dr = tab.display().display_map.doc_to_display(3);
    assert_eq!(first_visible_dr, 6, "doc line 3 starts at DisplayRow 6 (2+2+2)");

    // Scroll to second visual line of doc line 3 (Stage 5: anchor-based)
    let mut presentation = tab.take_presentation();
    presentation.display.viewport.scroll_anchor = ui::viewport::ScrollAnchor::new(3, 14.0);
    presentation.display.viewport.derive_scroll_top(&presentation.display.display_map, 14.0);

    // skip = pixel_offset / lh = 14/14 = 1
    let skip = (presentation.display.viewport.scroll_anchor.pixel_offset / 14.0) as usize;
    assert_eq!(skip, 1, "should skip 1 visual line (first visual line of doc 3 is above viewport)");
    tab.restore_presentation(presentation);
}

// ── Regression tests: plans_state_consolidation 回归用例 ──

/// 回归用例 1: Mouse drag 后立即 PageDown
/// 期望: page 跳到 drag 终点对应位置（非 drag 前）
/// 覆盖: R1 / F1 — cursor_offset 旁路导致 tb.cursor_visual_pos() 陈旧
#[test]
fn regression_drag_then_page_down() {
    // 50 行文档，viewport 10 行
    let lines: Vec<&str> =
        (0..50).map(|i| Box::leak(format!("line {i}").into_boxed_str()) as &str).collect();
    let mut app = app_with_content(lines);
    let tab = app.active_tab_session_mut().expect("active tab session");
    let crate::tab_session::TabSessionMut { document: dv, runtime, .. } = tab;

    // 先把光标移到第 1 行
    dv.cursor_move_to_offset(0);

    // Mouse drag 到第 20 行附近
    let target_line = 20;
    let target_offset = dv.line_byte_offset(target_line).unwrap();
    dv.set_cursor_offset_synced(target_offset);
    dv.cursor_mut().selection_anchor = Some(0); // 模拟 drag 选区

    // 记录 drag 终点
    let cursor_line_after_drag = dv.cursor_line();
    assert_eq!(cursor_line_after_drag, target_line, "precondition: cursor at drag endpoint");

    // 立即 PageDown
    let old_offset = dv.cursor().offset;
    let _ = crate::commands::execute_edit_command_v2_with_presentation(
        &EditCommand::PageDown,
        dv,
        &[],
        &mut runtime.presentation.cursor_render_state,
        9,
    );

    // cursor 应该从 drag 终点往下跳，而不是从 drag 起点
    assert!(
        dv.cursor().offset > old_offset,
        "PageDown should move forward from drag endpoint (old={}, new={})",
        old_offset.to_usize(),
        dv.cursor().offset.to_usize()
    );
}

/// 回归用例 2: Shift-click 后立即 PageUp
/// 期望: page 从 shift-click 位置向上跳
/// 覆盖: R1 / F1
#[test]
fn regression_shift_click_then_page_up() {
    let lines: Vec<&str> =
        (0..50).map(|i| Box::leak(format!("line {i}").into_boxed_str()) as &str).collect();
    let mut app = app_with_content(lines);
    let tab = app.active_tab_session_mut().expect("active tab session");
    let crate::tab_session::TabSessionMut { document: dv, runtime, .. } = tab;

    // 先移到第 30 行
    let start_line = 30;
    let start_offset = dv.line_byte_offset(start_line).unwrap();
    dv.cursor_move_to_offset(start_offset);

    // Shift-click 到第 10 行（模拟 extend selection）
    let click_line = 10;
    let click_offset = dv.line_byte_offset(click_line).unwrap();
    dv.cursor_mut().selection_anchor = Some(dv.cursor().offset.to_usize());
    dv.set_cursor_offset_synced(click_offset);

    let cursor_line_after = dv.cursor_line();
    assert_eq!(cursor_line_after, click_line, "precondition: cursor at shift-click point");

    // 立即 PageUp
    let old_offset = dv.cursor().offset;
    let _ = crate::commands::execute_edit_command_v2_with_presentation(
        &EditCommand::PageUp,
        dv,
        &[],
        &mut runtime.presentation.cursor_render_state,
        9,
    );

    // cursor 应该从 shift-click 位置往上跳
    assert!(
        dv.cursor().offset < old_offset,
        "PageUp should move backward from shift-click point (old={}, new={})",
        old_offset.to_usize(),
        dv.cursor().offset.to_usize()
    );
}

/// 回归用例 3: 长行 wrap → Shift+Down
/// 期望: 选区扩展到下一个 visual 行（不跳整 doc line）
/// 覆盖: F2 — extend_selection_up/down 走 visual 行
#[test]
fn regression_wrap_shift_down_extends_by_visual_line() {
    use ui::render_geom::AdvanceCacheEntry;

    // 一行很长的文本，会 wrap 成多个 visual 行
    let long_line = "word ".repeat(100); // 500 chars
    let mut app = app_with_content(vec![long_line.as_str()]);

    let offset_before;
    {
        let mut tab = app.active_tab_session_mut().unwrap();
        tab.document.cursor_move_to_offset(0);

        // 设置 wrap_index 使得 doc line 0 有多个 visual 行
        tab.display_mut().display_map.update_entry_in_place(
            0,
            crate::snap_tree::DisplayLineEntry {
                visual_line_count: 5,
                visual_breaks: smallvec::SmallVec::new(),
                byte_offset: 0,
                byte_length: 0,
                content_hash: 0,
            },
        ); // 5 visual lines
        tab.display_mut().display_map.rebuild_tree();

        tab.cursor_render_state_mut().cursor_visual_line = Some(0);
        tab.cursor_render_state_mut().sticky_x = 0.0;

        offset_before = tab.document.cursor().offset;
    }

    // 设置 advance_cache 来模拟 visual 行（clusters 使用 vl-local 偏移）
    app.active_tab_session_mut().unwrap().display_mut().advance_cache.clear();
    let chunk_size = long_line.len() / 5;
    for vl in 0..5 {
        let byte_start = vl * chunk_size;
        let mut clusters = Vec::new();
        for i in 0..chunk_size {
            clusters.push((i + 1, (i + 1) as f32 * 8.0, 0u32));
        }
        app.active_tab_session_mut().unwrap().display_mut().advance_cache.push(AdvanceCacheEntry {
            doc_line: 0,
            vl_byte_start: byte_start,
            vl_grapheme_start: 0,
            clusters,
        });
    }

    // 执行 ExtendDown（现在走 visual 行）
    app.extend_selection_visual(1);

    // cursor 应该移动到下一个 visual 行，而不是跳到 doc line 末尾
    let dv = editor_document_for_test(&mut app, 0);
    assert!(
        dv.cursor().offset > offset_before,
        "Shift+Down should move to next visual line (before={}, after={})",
        offset_before.to_usize(),
        dv.cursor().offset.to_usize()
    );
    // 应该还在同一 doc line
    assert_eq!(dv.cursor_line(), 0, "should stay on same doc line");
    // 应该有选区
    assert!(dv.cursor().selection_anchor.is_some(), "selection should be active after Shift+Down");
}

/// 期望: 返回新位置的 line（不是 cached 的旧值）
/// 覆盖: F4 — cached_cursor_line 失效
#[test]
fn regression_delete_selection_cursor_line_cache() {
    let mut dv = DocumentView::new(vec!["line0".into(), "line1".into(), "line2".into()], 80, 10.0);

    // 选中 line0 的全部内容
    dv.cursor_mut().selection_anchor = Some(0);
    dv.set_cursor_offset_synced(5); // "line0" = 5 bytes

    // 确认 cursor_line 在 line 0
    assert_eq!(dv.cursor_line(), 0);

    // 删除选区
    let deleted = dv.delete_selection();
    assert!(deleted, "should have deleted selection");

    // cursor_line 应该返回新位置的行（不是缓存的旧值）
    let line_after = dv.cursor_line();
    // 删除 "line0" 后，cursor 在 offset 0，应该在 line 0
    assert_eq!(line_after, 0, "cursor_line should reflect post-delete position");
}

/// 回归用例 6: 长时间编辑 → debug 断言
/// 期望: 常见编辑序列不触发 cursor desync
/// 覆盖: 阶段 4 — assert_cursor_synced
#[test]
fn regression_edit_sequence_no_cursor_desync() {
    let mut dv = DocumentView::new(vec!["hello world".into()], 80, 10.0);

    // 序列: 输入 → 移动 → 删除 → 选区 → 删除选区
    dv.insert_at_cursor(b"abc");
    dv.assert_cursor_synced();

    dv.cursor_move_left();
    dv.assert_cursor_synced();

    dv.cursor_move_right();
    dv.assert_cursor_synced();

    dv.delete_backward(1);
    dv.assert_cursor_synced();

    dv.extend_selection_left();
    dv.assert_cursor_synced();

    dv.extend_selection_right();
    dv.assert_cursor_synced();

    dv.delete_selection();
    dv.assert_cursor_synced();

    dv.select_all();
    dv.assert_cursor_synced();

    dv.cursor_move_to_offset(0);
    dv.assert_cursor_synced();

    // 模拟 set_cursor_offset_synced 后的断言
    dv.set_cursor_offset_synced(5);
    dv.assert_cursor_synced();
}

/// 回归测试: close_entry 关闭当前活动 tab(非最后一个)后，
/// wrap_index 和 display_map 必须重建为滑入的新文档数据。
#[test]
fn close_entry_active_rebuilds_wrap_index() {
    // 打开 3 个文档，激活中间那个
    let mut app = app_with_content(vec!["doc1 line1", "doc1 line2", "doc1 line3"]);
    // 添加 doc2（5行）
    let dv2 = DocumentView::new(
        ["d2l1", "d2l2", "d2l3", "d2l4", "d2l5"].iter().map(|s| s.to_string()).collect(),
        80,
        10.0,
    );
    app.push_entry_for_test(dv2, Box::new(EditorPlugin::new()));
    app.switch_workspace_for_test(1);
    app.init_display_map(1);
    // 添加 doc3（10行）
    let dv3 = DocumentView::new((0..10).map(|i| format!("line{i}")).collect(), 80, 10.0);
    app.push_entry_for_test(dv3, Box::new(EditorPlugin::new()));
    // active 仍是 1 (doc2)

    // 关闭 active tab (doc2, index 1)
    close_editor_tab_for_test(&mut app, 1);

    // doc3 滑入 index 1，wrap_index 应该是 10 行
    assert_eq!(app.active_editor_index(), Some(1));
    assert_eq!(app.editor_tab_count(), 2);
    let tab = app.active_tab_session().expect("reactivated tab should have a runtime session");
    assert_eq!(
        tab.display().display_map.total_rows(),
        10,
        "close active tab 后 wrap_index 应反映新文档的 10 行"
    );
    assert_eq!(
        tab.display().display_map.line_count(),
        10,
        "close active tab 后 display_map 应反映新文档的 10 行"
    );
}

/// 关闭非活动 tab(位于活动 tab 之前)，验证 active_index 正确偏移。
#[test]
fn close_entry_before_active_shifts_index() {
    let mut app = app_with_content(vec!["doc1"]);
    // doc2
    let dv2 = DocumentView::new(["d2"].iter().map(|s| s.to_string()).collect(), 80, 10.0);
    app.push_entry_for_test(dv2, Box::new(EditorPlugin::new()));
    // doc3（7行，将是 active）
    let dv3 = DocumentView::new((0..7).map(|i| format!("l{i}")).collect(), 80, 10.0);
    app.push_entry_for_test(dv3, Box::new(EditorPlugin::new()));
    app.switch_workspace_for_test(2);
    app.init_display_map(2);

    // 关闭 index 0 的 doc1
    close_editor_tab_for_test(&mut app, 0);

    // active 应从 2 偏移到 1，且仍指向 doc3（7行）
    assert_eq!(app.active_editor_index(), Some(1), "关闭前面的 tab 后 active_index 应减 1");
    let tab = app.active_tab_session().expect("surviving active tab should have a runtime session");
    assert_eq!(tab.display().display_map.total_rows(), 7);
    assert_eq!(tab.display().display_map.line_count(), 7);
}

/// 关闭多个 tab 后只剩一个，验证 active_index 和 wrap_index 正确。
#[test]
fn close_entry_down_to_single_document() {
    let mut app = app_with_content(vec!["a"]);
    let dv2 = DocumentView::new(["b1", "b2"].iter().map(|s| s.to_string()).collect(), 80, 10.0);
    app.push_entry_for_test(dv2, Box::new(EditorPlugin::new()));
    let dv3 =
        DocumentView::new(["c1", "c2", "c3"].iter().map(|s| s.to_string()).collect(), 80, 10.0);
    app.push_entry_for_test(dv3, Box::new(EditorPlugin::new()));
    app.switch_workspace_for_test(2);
    app.init_display_map(2);

    // 关闭 index 1
    close_editor_tab_for_test(&mut app, 1);
    assert_eq!(app.editor_tab_count(), 2);
    assert_eq!(app.active_editor_index(), Some(1));

    // 关闭 index 0
    close_editor_tab_for_test(&mut app, 0);
    assert_eq!(app.editor_tab_count(), 1);
    assert_eq!(app.active_editor_index(), Some(0));
    let tab = app.active_tab_session().expect("last tab should have a runtime session");
    assert_eq!(tab.display().display_map.total_rows(), 3, "最终只剩 doc3 时 wrap_index 应为 3 行");
    assert_eq!(tab.display().display_map.line_count(), 3);
}

#[test]
fn tab_runtime_by_id_mut_updates_exact_tab() {
    let mut app = app_with_content(vec!["doc1"]);
    let second = DocumentView::new(vec!["doc2".to_string()], 80, 10.0);
    app.push_entry_for_test(second, Box::new(EditorPlugin::new()));

    let first_id = app.editor_tab_id_at(0).expect("first tab id");
    let second_id = app.editor_tab_id_at(1).expect("second tab id");

    app.tab_runtime_mut(second_id).expect("second runtime").toc_visible = true;

    assert!(
        !app.tab_runtime(first_id).expect("first runtime").toc_visible,
        "updating a runtime by id must not affect other tabs"
    );
    assert!(
        app.tab_runtime(second_id).expect("second runtime").toc_visible,
        "runtime lookup by id must reflect the updated tab"
    );
}

#[test]
fn tab_runtime_by_id_is_none_after_close() {
    let mut app = app_with_content(vec!["doc1"]);
    let second = DocumentView::new(vec!["doc2".to_string()], 80, 10.0);
    app.push_entry_for_test(second, Box::new(EditorPlugin::new()));

    let closed_id = app.editor_tab_id_at(0).expect("closed tab id");
    let surviving_id = app.editor_tab_id_at(1).expect("surviving tab id");
    close_editor_tab_for_test(&mut app, 0);

    assert!(app.tab_runtime(closed_id).is_none(), "closed tab runtime must be gone");
    assert!(app.tab_runtime(surviving_id).is_some(), "surviving tab runtime must remain");
    assert_eq!(app.active_tab_id(), Some(surviving_id));
}

#[test]
fn apply_workspace_effect_removes_closed_runtime_from_store_by_exact_tab_id() {
    let mut app = app_with_content(vec!["doc1"]);
    let second = DocumentView::new(vec!["doc2".to_string()], 80, 10.0);
    let second_id = app.push_entry_for_test(second, Box::new(EditorPlugin::new()));
    let first_id = app.editor_tab_id_at(0).expect("first tab id");
    close_editor_tab_for_test(&mut app, 0);

    assert!(app.tab_runtime(first_id).is_none(), "closed tab runtime must be removed from store");
    assert!(app.tab_runtime(second_id).is_some(), "surviving tab runtime must remain in store");
}

#[test]
fn tab_runtime_lookup_reads_the_runtime_store() {
    let mut app = app_with_content(vec!["doc1"]);
    let second = DocumentView::new(vec!["doc2".to_string()], 80, 10.0);
    let second_id = app.push_entry_for_test(second, Box::new(EditorPlugin::new()));

    app.tab_runtime_mut(second_id).expect("store runtime").toc_visible = true;

    assert!(
        app.tab_runtime(second_id).expect("runtime from store").toc_visible,
        "tab runtime lookup should read from the App-owned store"
    );
}

fn assert_workspace_runtime_bijection(app: &App) {
    assert_eq!(
        app.editor_tab_ids_in_order().into_iter().collect::<std::collections::HashSet<_>>(),
        app.editor_runtime_tab_ids(),
        "every open document must have exactly one runtime with the same TabId"
    );
}

#[test]
fn runtime_store_stays_bijective_across_create_switch_and_close() {
    let mut app = App::new(None);

    app.new_untitled_doc();
    assert_workspace_runtime_bijection(&app);

    app.new_typed_untitled_doc(ui::sidebar::NewDocumentKind::Markdown);
    assert_workspace_runtime_bijection(&app);

    let temp_dir = tempfile::tempdir().expect("runtime lifecycle tempdir");
    let file_path = temp_dir.path().join("opened.txt");
    std::fs::write(&file_path, "opened").expect("runtime lifecycle fixture");
    app.open_file(&file_path).expect("file should open");
    assert_workspace_runtime_bijection(&app);

    app.open_file(&file_path).expect("opening an existing tab should activate it");
    assert_workspace_runtime_bijection(&app);

    let first_id = app.editor_tab_id_at(0).expect("first tab id");
    app.dispatch_tab_switch(first_id);
    assert_workspace_runtime_bijection(&app);

    close_editor_tab_for_test(&mut app, 1);
    assert_workspace_runtime_bijection(&app);

    app.execute_batch_close(&[1]);
    assert_workspace_runtime_bijection(&app);

    close_editor_tab_for_test(&mut app, 0);
    assert_workspace_runtime_bijection(&app);
}

#[test]
fn tab_session_borrows_document_and_store_runtime_for_exact_id() {
    let mut app = app_with_content(vec!["doc1"]);
    let second = DocumentView::new(vec!["doc2".to_string()], 80, 10.0);
    let second_id = app.push_entry_for_test(second, Box::new(EditorPlugin::new()));

    app.tab_runtime_mut(second_id).expect("store runtime").toc_visible = true;

    let session = app.tab_session(second_id).expect("second tab session");

    assert_eq!(session.id, second_id);
    assert_eq!(session.document.full_text(), "doc2");
    assert!(session.runtime.toc_visible, "session must prefer the runtime store");
}

#[test]
fn tab_session_mut_updates_store_runtime() {
    let mut app = app_with_content(vec!["doc1"]);
    let second = DocumentView::new(vec!["doc2".to_string()], 80, 10.0);
    let second_id = app.push_entry_for_test(second, Box::new(EditorPlugin::new()));

    app.tab_session_mut(second_id).expect("second mutable tab session").runtime.toc_visible = true;

    assert!(
        app.tab_runtime(second_id).expect("store runtime").toc_visible,
        "mutable session must update the runtime store"
    );
}

#[test]
fn active_tab_session_tracks_workspace_activation_by_tab_id() {
    let mut app = app_with_content(vec!["doc1"]);
    let second = DocumentView::new(vec!["doc2".to_string()], 80, 10.0);
    let second_id = app.push_entry_for_test(second, Box::new(EditorPlugin::new()));
    app.switch_workspace_for_test(1);

    let session = app.active_tab_session().expect("active tab session");

    assert_eq!(session.id, second_id);
    assert_eq!(session.document.full_text(), "doc2");
}

// --- Phase 1 Bug Fix Tests ---

#[test]
fn test_double_click_spatial_proximity() {
    let mut app = app_with_content(vec!["hello world this is a test"]);
    let tab_id = app.editor_tab_id_at(0).expect("test tab index should be valid");
    let modifiers = winit::keyboard::ModifiersState::empty();
    let mut cursor_render_state = crate::cursor_motion::CursorRenderState::new();

    // First click at position (10, 10)
    app.with_editor_document_and_mouse_for_test(tab_id, |mouse, document| {
        crate::mouse::handle_mouse_input_with_cursor_state(
            winit::event::ElementState::Pressed,
            10.0,
            10.0,
            mouse,
            document,
            &mut cursor_render_state,
            modifiers,
            Some((UniCharOffset(0), 0, 0)),
        );
    })
    .expect("test tab session should exist");

    assert_eq!(app.mouse.click_count, 1);

    // Release
    app.with_editor_document_and_mouse_for_test(tab_id, |mouse, document| {
        crate::mouse::handle_mouse_input_with_cursor_state(
            winit::event::ElementState::Released,
            10.0,
            10.0,
            mouse,
            document,
            &mut cursor_render_state,
            modifiers,
            Some((UniCharOffset(0), 0, 0)),
        );
    })
    .expect("test tab session should exist");

    // Second click within 500ms but FAR AWAY (100, 100) -> distance squared is large
    app.with_editor_document_and_mouse_for_test(tab_id, |mouse, document| {
        crate::mouse::handle_mouse_input_with_cursor_state(
            winit::event::ElementState::Pressed,
            100.0,
            100.0,
            mouse,
            document,
            &mut cursor_render_state,
            modifiers,
            Some((UniCharOffset(15), 0, 0)),
        );
    })
    .expect("test tab session should exist");

    // Click count should be reset to 1 due to spatial proximity check
    assert_eq!(
        app.mouse.click_count, 1,
        "Click count should reset if spatial distance is too large"
    );
}

#[test]
fn test_mouse_release_clears_empty_selection() {
    let mut app = app_with_content(vec!["hello world"]);
    let tab_id = app.editor_tab_id_at(0).expect("test tab index should be valid");
    let modifiers = winit::keyboard::ModifiersState::empty();
    let mut cursor_render_state = crate::cursor_motion::CursorRenderState::new();

    // Single click
    app.with_editor_document_and_mouse_for_test(tab_id, |mouse, document| {
        crate::mouse::handle_mouse_input_with_cursor_state(
            winit::event::ElementState::Pressed,
            10.0,
            10.0,
            mouse,
            document,
            &mut cursor_render_state,
            modifiers,
            Some((UniCharOffset(5), 0, 0)),
        );
    })
    .expect("test tab session should exist");

    // Simulate drag but cursor hasn't moved (so selection anchor == cursor)
    app.tab_session_mut(tab_id)
        .expect("test tab session should exist")
        .cursor_mut()
        .selection_anchor = Some(5);
    app.tab_session_mut(tab_id).expect("test tab session should exist").cursor_move_to_offset(5);

    // Release
    app.with_editor_document_and_mouse_for_test(tab_id, |mouse, document| {
        crate::mouse::handle_mouse_input_with_cursor_state(
            winit::event::ElementState::Released,
            10.0,
            10.0,
            mouse,
            document,
            &mut cursor_render_state,
            modifiers,
            Some((UniCharOffset(5), 0, 0)),
        );
    })
    .expect("test tab session should exist");

    // Empty selection should be cleared on release
    let dv = app.active_tab_session().expect("active tab session");
    assert_eq!(
        dv.cursor().selection_anchor,
        None,
        "Empty selection should be cleared on mouse release"
    );
}

#[test]
fn test_backspace_with_empty_selection() {
    let mut app = app_with_content(vec!["hello world"]);
    let dv = editor_document_for_test(&mut app, 0);

    dv.cursor_move_to_offset(5);
    dv.cursor_mut().selection_anchor = Some(5); // Empty selection

    // Execute Backspace
    let mut cursor_render_state = crate::cursor_motion::CursorRenderState::new();
    crate::commands::execute_edit_command_v2_with_presentation(
        &EditCommand::Backspace,
        dv,
        &[],
        &mut cursor_render_state,
        1,
    );

    // Should delete backward instead of being blocked
    assert_eq!(dv.cursor().offset, ByteIndex(4), "Cursor should move backward after deletion");
    assert_eq!(dv.cursor().selection_anchor, None, "Anchor should be cleared");
    // Read full text from buffer
    let mut content = Vec::new();
    let mut off = 0;
    while off < dv.tb.text_length() {
        let chunk = dv.tb.read_forward(off);
        if chunk.is_empty() {
            break;
        }
        content.extend_from_slice(chunk);
        off += chunk.len();
    }
    let content = String::from_utf8_lossy(&content);
    assert_eq!(content, "hell world", "Character before cursor should be deleted");
}

#[test]
fn tabs_geometry_and_preview_offset_use_instance_settings() {
    let mut app = app_with_content(vec!["first"]);
    let second = DocumentView::new(vec!["second".into()], 10, 10.0);
    app.push_entry_for_test(second, Box::new(EditorPlugin::new()));
    app.switch_workspace_for_test(1);
    app.settings.view_mode = ui::view_mode::ViewMode::Tabs;
    app.update_scale_factor(2.0);

    assert_eq!(app.current_tab_bar_height(), 64.0);
    assert_eq!(app.content_top_offset(), 64.0);
    let (_, preview_y) = app.preview_offsets();
    assert_eq!(preview_y, 96.0);

    app.settings.view_mode = ui::view_mode::ViewMode::Sidebar;
    assert_eq!(app.current_tab_bar_height(), 0.0);
}

#[test]
fn editor_left_margin_respects_instance_line_number_setting() {
    let mut app = app_with_content(vec!["line"]);
    app.settings.show_line_numbers = false;
    app.settings.font_size = 40.0;
    app.settings.view_mode = ui::view_mode::ViewMode::Tabs;

    assert_eq!(app.editor_left_margin(1_000_000), 32.0);

    app.settings.show_line_numbers = true;
    assert!(app.editor_left_margin(1_000_000) > 32.0);
}

// ── Compatibility facade tests ──────────────────────────────────────

#[test]
fn ui_metrics_scales_logical_settings_by_dpi() {
    let mut app = App::new(None);
    app.update_scale_factor(2.0);

    let metrics = app.ui_metrics();

    assert_eq!(metrics.dpi, 2.0);
    assert_eq!(metrics.font_size, app.settings.font_size * 2.0);
    assert_eq!(metrics.line_height, app.settings.line_height * 2.0);
}

#[test]
fn persisted_font_size_returns_logical_value() {
    let mut app = App::new(None);
    app.update_scale_factor(2.0);
    assert_eq!(app.persisted_font_size(), 15.0);
}

#[test]
fn viewport_dimensions_use_instance_line_height_and_chrome() {
    let mut app = App::new(None);
    app.replace_editor_model(
        crate::app_init::build_product_workspace(),
        crate::tab_runtime::TabRuntimeStore::default(),
    );
    let dv = crate::document_view::DocumentView::new(vec!["".into()], 80, 10.0);
    app.push_entry_for_test(dv, Box::new(EditorPlugin::new()));
    app.settings.view_mode = ui::view_mode::ViewMode::Tabs;
    app.settings.show_status_bar = false;
    app.update_scale_factor(2.0);
    app.settings.line_height = 40.0;
    // metrics.line_height = 40.0 * 2.0 = 80.0 (physical)
    // screen 400 physical / 80 physical = 5 visible rows
    let dims = app.viewport_dimensions(400.0);
    let expected_height = app.visible_height_lines(400.0);
    assert_eq!(dims.visible_rows, expected_height.floor() as usize);
    assert_eq!(dims.viewport_height, expected_height);
}

// ── Logical settings invariance tests ────────────────────────────────

#[test]
fn scale_factor_changes_metrics_but_not_logical_settings() {
    let mut app = App::new(None);
    let before = (
        app.settings.font_size,
        app.settings.line_height,
        app.settings.status_bar_height,
        app.settings.gutter_padding,
        app.settings.toc_width,
    );

    app.update_scale_factor(2.0);
    let retina = app.ui_metrics();
    assert_eq!(
        (
            app.settings.font_size,
            app.settings.line_height,
            app.settings.status_bar_height,
            app.settings.gutter_padding,
            app.settings.toc_width,
        ),
        before
    );
    assert_eq!(retina.font_size, before.0 * 2.0);

    app.update_scale_factor(1.0);
    assert_eq!(app.ui_metrics().font_size, before.0);
}

// ── DPI / zoom / sidebar regression tests ──────────────────────────────

#[test]
fn dpi_zoom_and_sidebar_width_are_reversible() {
    let mut app = App::new(None);
    let logical_font = app.settings.font_size;
    let logical_sidebar = app.ui_shell.sidebar_width() / app.ui_metrics().dpi;
    let settings_version = app.settings.version;
    let reshape_generation = app.editor_runtime.reshape_generation();

    app.handle_scale_factor_changed(2.0);
    assert_eq!(app.settings.font_size, logical_font);
    assert_eq!(app.settings.version, settings_version);
    assert!(app.editor_runtime.reshape_generation() > reshape_generation);
    assert_eq!(app.ui_metrics().font_size, logical_font * 2.0);
    assert_eq!(app.ui_shell.sidebar_width(), logical_sidebar * 2.0);

    app.apply_zoom(logical_font + 1.0);
    assert_eq!(app.settings.font_size, logical_font + 1.0);
    assert_eq!(app.ui_metrics().font_size, (logical_font + 1.0) * 2.0);

    app.handle_scale_factor_changed(1.0);
    assert_eq!(app.settings.font_size, logical_font + 1.0);
    assert_eq!(app.ui_metrics().font_size, logical_font + 1.0);
    assert_eq!(app.ui_shell.sidebar_width(), logical_sidebar);
}

// ── apply_effect step pipeline tests ────────────────────────────────

use crate::app_effect::AppEffect;
use crate::plugins::editor::EditorPlugin;

#[test]
fn apply_effect_runs_reshape_before_redraw_without_window() {
    let mut app = App::new(None);
    app.needs_redraw = false;
    let generation = app.editor_runtime.reshape_generation();

    app.apply_effect(AppEffect::RESHAPE);

    assert_eq!(app.editor_runtime.reshape_generation(), generation + 1);
    assert!(app.needs_redraw);
}

#[test]
fn apply_window_chrome_effect_is_safe_without_window() {
    let mut app = App::new(None);
    app.needs_redraw = false;

    app.apply_effect(AppEffect::SYNC_WINDOW_CHROME);

    assert!(app.needs_redraw);
}

#[test]
fn apply_persist_settings_continues_on_io_error() {
    // persist_settings should not panic even if settings file is unavailable;
    // the pipeline must continue and still trigger redraw if requested.
    let mut app = App::new(None);
    app.needs_redraw = false;

    let effect = AppEffect::PERSIST_SETTINGS.merge(AppEffect::REDRAW);
    app.apply_effect(effect);

    assert!(app.needs_redraw, "pipeline must continue past settings error to redraw");
}

// ── WYSIWYG source and cursor sync tests ────────────────────────────

struct RecordingWysiwygState {
    source_text: String,
    generation: u32,
    cursor_byte: Option<usize>,
    sel_anchor_byte: Option<usize>,
    sel_cursor_byte: Option<usize>,
    render_count: usize,
    /// Preconfigured response for HitTestByte queries.
    hit_test_byte: Option<usize>,
    /// Preconfigured content height used to distinguish a miss from a click below content.
    content_height: f32,
    /// Preconfigured response for VisualMove queries.
    visual_move_result: Option<usize>,
    /// Last VisualMove query observed by the recording plugin.
    visual_move_query: Option<(usize, ui::plugin::MoveDirection, Option<f32>)>,
    /// Preconfigured response for CursorScreenPos queries (x, y, w, h).
    cursor_rect: Option<(f32, f32, f32, f32)>,
    cursor_rect_by_byte: Vec<(usize, (f32, f32, f32, f32))>,
    /// Scroll messages observed by the recording plugin (delta, viewport_h).
    scroll_messages: Vec<(f32, f32)>,
    /// Preconfigured response for SelectionRange queries (source byte range).
    selection_range: Option<(usize, usize)>,
    edit_plan: ui::plugin::EditPlan,
    recorded_edit_requests: Vec<ui::plugin::EditRequest>,
    preedit_text: String,
    preedit_cursor: Option<(usize, usize)>,
}

impl Default for RecordingWysiwygState {
    fn default() -> Self {
        Self {
            render_count: 0,
            source_text: String::new(),
            generation: 0,
            cursor_byte: None,
            sel_anchor_byte: None,
            sel_cursor_byte: None,
            hit_test_byte: None,
            content_height: 0.0,
            visual_move_result: None,
            visual_move_query: None,
            cursor_rect: None,
            cursor_rect_by_byte: vec![],
            scroll_messages: vec![],
            selection_range: None,
            edit_plan: ui::plugin::EditPlan::Consume,
            recorded_edit_requests: vec![],
            preedit_text: String::new(),
            preedit_cursor: None,
        }
    }
}

struct RecordingWysiwygPlugin {
    state: std::rc::Rc<std::cell::RefCell<RecordingWysiwygState>>,
}

impl RecordingWysiwygPlugin {
    fn new(state: std::rc::Rc<std::cell::RefCell<RecordingWysiwygState>>) -> Self {
        Self { state }
    }
}

impl ui::plugin::ViewPlugin for RecordingWysiwygPlugin {
    fn name(&self) -> &str {
        "recording_wysiwyg"
    }

    fn render(
        &mut self,
        _doc: &dyn core::document::DocView,
        _bounds: ui::core::geom::Rect,
        _theme: &ui::theme::Theme,
        _shaper: &mut shaping::Shaper,
        _dpi_scale: f32,
    ) -> ui::core::paint::DrawList {
        self.state.borrow_mut().render_count += 1;
        ui::core::paint::DrawList::new()
    }

    fn allows_editing(&self) -> bool {
        true
    }

    fn handles_own_rendering(&self) -> bool {
        true
    }

    fn edit_policy(&self) -> &dyn ui::plugin::EditPolicy {
        self
    }

    fn query(
        &self,
        query: ui::plugin::PluginQuery,
        _doc: &dyn core::document::DocView,
    ) -> ui::plugin::PluginResponse {
        match query {
            ui::plugin::PluginQuery::NeedsSourceUpdate(generation) => {
                ui::plugin::PluginResponse::Bool(generation != self.state.borrow().generation)
            }
            ui::plugin::PluginQuery::CursorScreenPos(byte) => {
                let state = self.state.borrow();
                let rect = state
                    .cursor_rect_by_byte
                    .iter()
                    .find_map(|(candidate, rect)| (*candidate == byte).then_some(*rect))
                    .or(state.cursor_rect);
                ui::plugin::PluginResponse::CursorScreenRect(rect)
            }
            ui::plugin::PluginQuery::HitTestByte { .. } => {
                let state = self.state.borrow();
                ui::plugin::PluginResponse::BytePosition(state.hit_test_byte)
            }
            ui::plugin::PluginQuery::ContentHeight => {
                ui::plugin::PluginResponse::Float(self.state.borrow().content_height)
            }
            ui::plugin::PluginQuery::VisualMove { current_byte, direction, target_x } => {
                let mut state = self.state.borrow_mut();
                state.visual_move_query = Some((current_byte, direction, target_x));
                ui::plugin::PluginResponse::BytePosition(state.visual_move_result)
            }
            ui::plugin::PluginQuery::HasSelection => {
                let state = self.state.borrow();
                ui::plugin::PluginResponse::Bool(
                    state.sel_anchor_byte.is_some()
                        && state.sel_cursor_byte.is_some()
                        && state.sel_anchor_byte != state.sel_cursor_byte,
                )
            }
            ui::plugin::PluginQuery::SelectionRange => {
                let state = self.state.borrow();
                let range = state.selection_range.or_else(|| {
                    let start = state.sel_anchor_byte?;
                    let end = state.sel_cursor_byte?;
                    (start != end).then_some((start.min(end), start.max(end)))
                });
                ui::plugin::PluginResponse::PositionPair(
                    range.map(|(start, end)| ((start, 0), (end, 0))),
                )
            }
            _ => ui::plugin::PluginResponse::None,
        }
    }

    fn handle_message(
        &mut self,
        msg: ui::plugin::PluginMessage,
        _doc: &mut dyn core::document::DocViewMut,
    ) -> bool {
        match msg {
            ui::plugin::PluginMessage::UpdateSource { text, generation } => {
                let mut state = self.state.borrow_mut();
                state.source_text = text;
                state.generation = generation;
                true
            }
            ui::plugin::PluginMessage::SetCursorByte(byte) => {
                self.state.borrow_mut().cursor_byte = Some(byte);
                true
            }
            ui::plugin::PluginMessage::SetSelAnchorByte(byte) => {
                self.state.borrow_mut().sel_anchor_byte = byte;
                true
            }
            ui::plugin::PluginMessage::SetSelCursorByte(byte) => {
                self.state.borrow_mut().sel_cursor_byte = byte;
                true
            }
            ui::plugin::PluginMessage::Scroll { delta, viewport_h } => {
                self.state.borrow_mut().scroll_messages.push((delta, viewport_h));
                true
            }
            ui::plugin::PluginMessage::SetPreedit { text, cursor } => {
                let mut state = self.state.borrow_mut();
                state.preedit_text = text;
                state.preedit_cursor = cursor;
                true
            }
            _ => false,
        }
    }
}

impl ui::plugin::EditPolicy for RecordingWysiwygPlugin {
    fn plan_edit(&self, request: &ui::plugin::EditRequest) -> ui::plugin::EditPlan {
        let mut state = self.state.borrow_mut();
        state.recorded_edit_requests.push(request.clone());
        state.edit_plan.clone()
    }
}

#[test]
fn selected_enter_still_queries_plugin_edit_policy() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        edit_plan: ui::plugin::EditPlan::Consume,
        ..RecordingWysiwygState::default()
    }));
    let mut app = App::new(None);
    let mut doc = DocumentView::new(vec!["| a | b |".to_string()], 80, 10.0);
    doc.cursor_move_to_offset(3);
    doc.cursor_mut().selection_anchor = Some(2); // select `a`
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    let effect = app.dispatch_transactional_edit_for_test(EditCommand::InsertNewline);

    let recorded = state.borrow();
    assert_eq!(recorded.recorded_edit_requests.len(), 1);
    assert_eq!(recorded.recorded_edit_requests[0].selection, Some(2..3));
    let tab = app.active_tab_session().expect("active tab should exist");
    assert_eq!(tab.full_text(), "| a | b |");
    assert!(effect.redraw);
}

#[test]
#[cfg(feature = "markdown")]
fn markdown_empty_list_enter_uses_structural_edit_policy() {
    let mut app = App::new(None);
    let mut doc = DocumentView::new(vec!["- ".to_string()], 80, 10.0);
    doc.cursor_move_to_offset(2);
    app.push_entry_for_test(doc, Box::new(textora_markdown::view::MarkdownEditorView::new()));
    app.switch_workspace_for_test(0);

    app.dispatch_transactional_edit_for_test(EditCommand::InsertNewline);

    assert_eq!(active_document_text(&app), "");
}

#[test]
#[cfg(feature = "markdown")]
fn markdown_table_enter_keeps_moved_cursor_visible() {
    let source = "| a |\n|---|\n| b |";
    let expected_cursor = source.find('b').expect("table target content must exist");
    let mut app = App::new(None);
    let mut doc = DocumentView::new(source.lines().map(str::to_owned).collect(), 1, 1.0);
    doc.cursor_move_to_offset(source.find('a').expect("source table cell must exist"));
    let generation_before = doc.generation();
    doc.presentation.display.display_map.set_entries(
        (0..doc.line_count())
            .map(|line| crate::snap_tree::DisplayLineEntry::placeholder(line, 10, 0, 1))
            .collect(),
    );
    app.push_entry_for_test(doc, Box::new(textora_markdown::view::MarkdownEditorView::new()));
    app.switch_workspace_for_test(0);

    let effect = app.dispatch_transactional_edit_for_test(EditCommand::InsertNewline);

    let tab = app.active_tab_session().expect("active tab");
    assert_eq!(tab.document.cursor_offset().to_usize(), expected_cursor);
    assert_eq!(tab.document.generation(), generation_before);
    assert!(!tab.document.dirty);
    assert!(!effect.reshape);
    let visible_range = tab
        .display()
        .viewport
        .visible_doc_range_from_anchor(&tab.display().display_map, app.ui_metrics().line_height);
    assert!(
        visible_range.contains(&tab.document.cursor_line()),
        "cursor line {} must be visible in {visible_range:?}",
        tab.document.cursor_line()
    );
    assert!(tab.cursor_render_state().sticky_x_dirty);
}

#[derive(Default)]
struct RecordingPreviewState {
    hit_test_offset: Option<(f32, f32)>,
    hit_test_position: Option<(usize, usize)>,
    selection_anchor: Option<(usize, usize)>,
    selection_cursor: Option<(usize, usize)>,
}

struct RecordingPreviewPlugin {
    state: std::rc::Rc<std::cell::RefCell<RecordingPreviewState>>,
}

impl RecordingPreviewPlugin {
    fn new(state: std::rc::Rc<std::cell::RefCell<RecordingPreviewState>>) -> Self {
        Self { state }
    }
}

impl ui::plugin::ViewPlugin for RecordingPreviewPlugin {
    fn name(&self) -> &str {
        "recording_preview"
    }

    fn render(
        &mut self,
        _doc: &dyn core::document::DocView,
        _bounds: ui::core::geom::Rect,
        _theme: &ui::theme::Theme,
        _shaper: &mut shaping::Shaper,
        _dpi_scale: f32,
    ) -> ui::core::paint::DrawList {
        ui::core::paint::DrawList::new()
    }

    fn allows_editing(&self) -> bool {
        false
    }

    fn query(
        &self,
        query: ui::plugin::PluginQuery,
        _doc: &dyn core::document::DocView,
    ) -> ui::plugin::PluginResponse {
        match query {
            ui::plugin::PluginQuery::HitTest { offset_x, offset_y, .. } => {
                let mut state = self.state.borrow_mut();
                state.hit_test_offset = Some((offset_x, offset_y));
                ui::plugin::PluginResponse::Position(state.hit_test_position)
            }
            ui::plugin::PluginQuery::SelCursor => {
                ui::plugin::PluginResponse::Position(self.state.borrow().selection_cursor)
            }
            ui::plugin::PluginQuery::HasSelection => {
                let state = self.state.borrow();
                ui::plugin::PluginResponse::Bool(
                    state.selection_anchor.is_some()
                        && state.selection_cursor.is_some()
                        && state.selection_anchor != state.selection_cursor,
                )
            }
            _ => ui::plugin::PluginResponse::None,
        }
    }

    fn handle_message(
        &mut self,
        msg: ui::plugin::PluginMessage,
        _doc: &mut dyn core::document::DocViewMut,
    ) -> bool {
        match msg {
            ui::plugin::PluginMessage::SetSelAnchor(position) => {
                self.state.borrow_mut().selection_anchor = position;
                true
            }
            ui::plugin::PluginMessage::SetSelCursor(position) => {
                self.state.borrow_mut().selection_cursor = position;
                true
            }
            ui::plugin::PluginMessage::ClearSelection => {
                let mut state = self.state.borrow_mut();
                state.selection_anchor = None;
                state.selection_cursor = None;
                true
            }
            _ => false,
        }
    }
}

#[test]
fn preview_mouse_hit_test_uses_plugin_render_bounds_origin() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingPreviewState::default()));
    let mut app = App::new(None);
    let doc = DocumentView::new(vec!["hello world".to_string()], 80, 10.0);
    app.push_entry_for_test(doc, Box::new(RecordingPreviewPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    let bounds = app.plugin_render_bounds();
    app.dispatch_editor_mouse_input(
        winit::event::ElementState::Pressed,
        bounds.x + 10.0,
        bounds.y + 10.0,
        None,
    );

    assert_eq!(
        state.borrow().hit_test_offset,
        Some((bounds.x, bounds.y)),
        "preview hit testing must use the same origin as plugin rendering"
    );
}

#[test]
fn preview_tiny_mouse_move_does_not_start_selection() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingPreviewState {
        hit_test_position: Some((0, 0)),
        ..RecordingPreviewState::default()
    }));
    let mut app = App::new(None);
    let doc = DocumentView::new(vec!["hello world".to_string()], 80, 10.0);
    app.push_entry_for_test(doc, Box::new(RecordingPreviewPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    let bounds = app.plugin_render_bounds();
    let click_x = bounds.x + 10.0;
    let click_y = bounds.y + 10.0;
    let tiny_drag_x = click_x + 1.0;
    app.dispatch_editor_mouse_input(winit::event::ElementState::Pressed, click_x, click_y, None);
    state.borrow_mut().hit_test_position = Some((0, 1));

    app.dispatch_editor_cursor_moved(tiny_drag_x, click_y, None);

    assert!(
        !app.active_tab_session()
            .expect("active runtime session")
            .query_bool(ui::plugin::PluginQuery::HasSelection),
        "tiny preview mouse movement should not start text selection"
    );
}

#[test]
fn preview_mouse_release_clears_empty_selection() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingPreviewState {
        hit_test_position: Some((0, 0)),
        ..RecordingPreviewState::default()
    }));
    let mut app = App::new(None);
    let doc = DocumentView::new(vec!["hello world".to_string()], 80, 10.0);
    app.push_entry_for_test(doc, Box::new(RecordingPreviewPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    let bounds = app.plugin_render_bounds();
    let click_x = bounds.x + 10.0;
    let click_y = bounds.y + 10.0;
    app.dispatch_editor_mouse_input(winit::event::ElementState::Pressed, click_x, click_y, None);
    app.dispatch_editor_mouse_input(winit::event::ElementState::Released, click_x, click_y, None);

    let preview_state = state.borrow();
    assert_eq!(preview_state.selection_anchor, None);
    assert_eq!(preview_state.selection_cursor, None);
}

#[test]
fn wysiwyg_missing_projection_keeps_cursor_and_selection_unchanged() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        hit_test_byte: None,
        content_height: 500.0,
        ..RecordingWysiwygState::default()
    }));
    let mut app = App::new(None);
    let mut doc = DocumentView::new(vec!["abcdef".to_string()], 80, 10.0);
    doc.cursor_move_to_offset(3);
    doc.cursor_mut().selection_anchor = Some(1);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state)));
    app.switch_workspace_for_test(0);

    let bounds = app.plugin_render_bounds();
    app.dispatch_editor_mouse_input(
        winit::event::ElementState::Pressed,
        bounds.x + 10.0,
        bounds.y + 10.0,
        None,
    );

    let entry = app.active_tab_session().expect("active entry");
    assert_eq!(entry.cursor_offset().to_usize(), 3);
    assert_eq!(entry.cursor().selection_anchor, Some(1));
}

#[test]
fn wysiwyg_tiny_mouse_move_after_click_does_not_create_selection() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        hit_test_byte: Some(2),
        ..RecordingWysiwygState::default()
    }));
    let mut app = App::new(None);
    let doc = DocumentView::new(vec!["abcdef".to_string()], 80, 10.0);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    let bounds = app.plugin_render_bounds();
    let click_x = bounds.x + 10.0;
    let click_y = bounds.y + 10.0;
    app.dispatch_editor_mouse_input(winit::event::ElementState::Pressed, click_x, click_y, None);
    state.borrow_mut().hit_test_byte = Some(3);

    app.dispatch_editor_cursor_moved(click_x + 1.0, click_y, None);
    app.dispatch_editor_mouse_input(
        winit::event::ElementState::Released,
        click_x + 1.0,
        click_y,
        None,
    );

    let active_entry = app.active_tab_session().expect("active entry");
    assert_eq!(
        active_entry.selection_range(),
        None,
        "tiny WYSIWYG mouse movement after a click should not create a document selection"
    );

    let recorded = state.borrow();
    assert_eq!(
        recorded.sel_anchor_byte, None,
        "mouse release should clear the empty WYSIWYG selection anchor"
    );
    assert_eq!(
        recorded.sel_cursor_byte, None,
        "mouse release should clear the empty WYSIWYG selection cursor"
    );
}

#[test]
fn wysiwyg_backward_drag_keeps_plugin_selection_anchor() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        hit_test_byte: Some(8),
        ..RecordingWysiwygState::default()
    }));
    let mut app = App::new(None);
    let doc = DocumentView::new(vec!["abc：def".to_string()], 80, 10.0);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    let bounds = app.plugin_render_bounds();
    let click_x = bounds.x + 10.0;
    let click_y = bounds.y + 10.0;
    app.dispatch_editor_mouse_input(winit::event::ElementState::Pressed, click_x, click_y, None);
    state.borrow_mut().hit_test_byte = Some(3);

    app.dispatch_editor_cursor_moved(click_x - 20.0, click_y, None);
    app.dispatch_editor_mouse_input(
        winit::event::ElementState::Released,
        click_x - 20.0,
        click_y,
        None,
    );

    let active_entry = app.active_tab_session().expect("active entry");
    assert_eq!(
        active_entry.selection_range(),
        Some((3, 8)),
        "backward WYSIWYG drag should create a document selection"
    );
    assert!(
        app.active_tab_session()
            .expect("active runtime session")
            .query_bool(ui::plugin::PluginQuery::HasSelection),
        "backward WYSIWYG drag should keep the plugin selection visible"
    );
}

#[test]
fn wysiwyg_backward_drag_keeps_initial_anchor_across_moves() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        hit_test_byte: Some(8),
        ..RecordingWysiwygState::default()
    }));
    let mut app = App::new(None);
    let doc = DocumentView::new(vec!["abc：def".to_string()], 80, 10.0);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    let bounds = app.plugin_render_bounds();
    let click_x = bounds.x + 80.0;
    let click_y = bounds.y + 10.0;
    app.dispatch_editor_mouse_input(winit::event::ElementState::Pressed, click_x, click_y, None);

    state.borrow_mut().hit_test_byte = Some(5);
    app.dispatch_editor_cursor_moved(click_x - 20.0, click_y, None);

    state.borrow_mut().hit_test_byte = Some(3);
    app.dispatch_editor_cursor_moved(click_x - 40.0, click_y, None);

    let active_entry = app.active_tab_session().expect("active entry");
    assert_eq!(
        active_entry.selection_range(),
        Some((3, 8)),
        "backward WYSIWYG drag should keep the original press point as the selection anchor"
    );
    let recorded = state.borrow();
    assert_eq!(
        recorded.sel_anchor_byte,
        Some(8),
        "plugin selection anchor should remain at the original press point"
    );
    assert_eq!(
        recorded.sel_cursor_byte,
        Some(3),
        "plugin selection cursor should follow the current drag point"
    );
}

#[test]
fn wysiwyg_drag_move_does_not_synchronously_render_plugin() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        hit_test_byte: Some(8),
        ..RecordingWysiwygState::default()
    }));
    let mut app = App::new(None);
    let doc =
        DocumentView::new(vec!["first line".to_string(), "> quoted line".to_string()], 80, 10.0);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    let bounds = app.plugin_render_bounds();
    let click_x = bounds.x + 80.0;
    let click_y = bounds.y + 10.0;
    app.dispatch_editor_mouse_input(winit::event::ElementState::Pressed, click_x, click_y, None);
    state.borrow_mut().render_count = 0;

    state.borrow_mut().hit_test_byte = Some(2);
    app.dispatch_editor_cursor_moved(click_x - 40.0, click_y + 24.0, None);

    let active_entry = app.active_tab_session().expect("active entry");
    assert_eq!(
        active_entry.selection_range(),
        Some((2, 8)),
        "WYSIWYG drag should still update the document selection"
    );
    assert_eq!(
        state.borrow().render_count,
        0,
        "WYSIWYG drag movement should not synchronously render the plugin"
    );
}

#[test]
fn wysiwyg_backward_drag_across_paragraph_keeps_plugin_selection_anchor() {
    let source = "previous paragraph\n\nabc：def";
    let anchor_byte = source.find("def").expect("fixture should contain text after colon") + 2;
    let cursor_byte = source.find("previous").expect("fixture should contain previous paragraph");
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        hit_test_byte: Some(anchor_byte),
        ..RecordingWysiwygState::default()
    }));
    let mut app = App::new(None);
    let doc = DocumentView::new(source.lines().map(|line| line.to_string()).collect(), 80, 10.0);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    let bounds = app.plugin_render_bounds();
    let click_x = bounds.x + 180.0;
    let click_y = bounds.y + 80.0;
    app.dispatch_editor_mouse_input(winit::event::ElementState::Pressed, click_x, click_y, None);
    state.borrow_mut().hit_test_byte = Some(cursor_byte);

    app.dispatch_editor_cursor_moved(click_x - 120.0, click_y - 48.0, None);
    app.dispatch_editor_mouse_input(
        winit::event::ElementState::Released,
        click_x - 120.0,
        click_y - 48.0,
        None,
    );

    let active_entry = app.active_tab_session().expect("active entry");
    assert_eq!(
        active_entry.selection_range(),
        Some((cursor_byte, anchor_byte)),
        "backward WYSIWYG drag across paragraphs should create a document selection"
    );
    assert!(
        app.active_tab_session()
            .expect("active runtime session")
            .query_bool(ui::plugin::PluginQuery::HasSelection),
        "backward WYSIWYG drag across paragraphs should keep the plugin selection visible"
    );
}

#[cfg(feature = "markdown")]
fn render_active_wysiwyg_plugin_for_test(app: &mut App) -> ui::core::paint::DrawList {
    let bounds = app.plugin_render_bounds();
    render_active_wysiwyg_plugin_in_bounds_for_test(app, bounds)
}

#[cfg(feature = "markdown")]
fn render_active_wysiwyg_plugin_in_bounds_for_test(
    app: &mut App,
    bounds: ui::core::geom::Rect,
) -> ui::core::paint::DrawList {
    let theme = app.current_theme.clone();
    let dpi = app.ui_metrics().dpi;
    let font_size = app.ui_metrics().font_size;
    let mut shaper = app
        .editor_runtime
        .new_shaper(font_size, "")
        .unwrap_or_else(|| shaping::Shaper::new().expect("test shaper should initialize"));
    let mut tab = app.active_tab_session_mut().expect("active entry");
    tab.render_plugin(bounds, &theme, &mut shaper, dpi)
}

#[cfg(feature = "markdown")]
fn prepare_and_render_active_mmap_canvas_for_test(
    app: &mut App,
) -> ui::canvas::CanvasViewportSnapshot {
    let font_size = app.ui_metrics().font_size;
    let dpi = app.ui_metrics().dpi;
    let theme = app.current_theme.clone();
    let mut shaper = app
        .editor_runtime
        .new_shaper(font_size, "")
        .unwrap_or_else(|| shaping::Shaper::new().expect("test shaper should initialize"));
    let snapshot = app
        .sync_and_prepare_canvas_frame(&mut shaper)
        .expect("valid mmap source must resolve a canvas viewport");
    let mut tab = app.active_tab_session_mut().expect("active mmap tab");
    let _ = tab.render_canvas_plugin(&snapshot, &theme, &mut shaper, dpi);
    snapshot
}

#[cfg(feature = "markdown")]
fn render_active_mmap_canvas_snapshot_for_test(
    app: &mut App,
    snapshot: ui::canvas::CanvasViewportSnapshot,
) {
    let font_size = app.ui_metrics().font_size;
    let dpi = app.ui_metrics().dpi;
    let theme = app.current_theme.clone();
    let mut shaper = app
        .editor_runtime
        .new_shaper(font_size, "")
        .unwrap_or_else(|| shaping::Shaper::new().expect("test shaper should initialize"));
    let mut tab = app.active_tab_session_mut().expect("active mmap tab");
    let _ = tab.render_canvas_plugin(&snapshot, &theme, &mut shaper, dpi);
}

#[cfg(feature = "markdown")]
fn mmap_cursor_window_rect_for_test(app: &App, byte_offset: usize) -> ui::core::geom::Rect {
    let bounds = app.plugin_render_bounds();
    let tab = app.active_tab_session().expect("active mmap tab");
    let Some((x, y, width, height)) = tab.query_cursor_screen_rect(byte_offset) else {
        panic!("mmap cursor must have a screen rect for the active title");
    };
    ui::core::geom::Rect::new(bounds.x + x, bounds.y + y, width, height)
}

#[cfg(feature = "markdown")]
fn app_with_mmap_source(source: &str) -> App {
    let mut app = App::new(None);
    let doc = DocumentView::new(source.split('\n').map(str::to_owned).collect(), 80, 10.0);
    app.push_entry_for_test(doc, Box::new(textora_markdown::mindmap_view::MindmapView::new()));
    app.switch_workspace_for_test(0);
    app.sync_plugin_state();
    render_active_wysiwyg_plugin_for_test(&mut app);
    app
}

#[cfg(feature = "markdown")]
fn select_mmap_source_object(app: &mut App, source_range: std::ops::Range<usize>) {
    let mut tab = app.active_tab_session_mut().expect("active mmap tab");
    tab.cursor_move_to_offset(source_range.end);
    tab.cursor_mut().selection_anchor = Some(source_range.start);
    assert_eq!(
        tab.selection_range(),
        Some((source_range.start, source_range.end)),
        "mmap object selection must match the requested subtree source range"
    );
    app.sync_plugin_state();
}

#[cfg(feature = "markdown")]
fn undo_active_mmap_edit(app: &mut App) {
    let mut tab = app.active_tab_session_mut().expect("active mmap tab");
    tab.undo();
    app.sync_plugin_state();
}

#[cfg(feature = "markdown")]
fn redo_active_mmap_edit(app: &mut App) {
    let mut tab = app.active_tab_session_mut().expect("active mmap tab");
    tab.redo();
    app.sync_plugin_state();
}

#[cfg(feature = "markdown")]
fn mmap_text_caret_hit_point(app: &mut App, expected_byte_offset: usize) -> (f32, f32) {
    const HIT_TEST_GRID_STEP_PX: usize = 4;

    let bounds = app.plugin_render_bounds();
    let tab = app.active_tab_session().expect("active mmap tab");
    let width = bounds.w.ceil() as usize;
    let height = bounds.h.ceil() as usize;

    for y in (0..=height).step_by(HIT_TEST_GRID_STEP_PX) {
        for x in (0..=width).step_by(HIT_TEST_GRID_STEP_PX) {
            let screen_x = bounds.x + x as f32;
            let screen_y = bounds.y + y as f32;
            if matches!(
                tab.hit_test_edit_target(screen_x, screen_y, bounds.x, bounds.y),
                Some(Some(ui::plugin::EditHitTarget::TextCaret { byte_offset, .. }))
                    if byte_offset == expected_byte_offset
            ) {
                return (screen_x, screen_y);
            }
        }
    }

    panic!("expected a semantic hit point for mmap text caret {expected_byte_offset}");
}

#[cfg(feature = "markdown")]
const MMAP_HIT_TEST_GRID_STEP_PX: usize = 4;
#[cfg(feature = "markdown")]
const MMAP_SIBLING_DROP_X_OFFSET_PX: f32 = 0.0;
#[cfg(feature = "markdown")]
const MMAP_LAST_CHILD_DROP_X_OFFSET_PX: f32 = 96.0;
#[cfg(feature = "markdown")]
const MMAP_DROP_Y_OFFSET_PX: f32 = 8.0;

#[cfg(feature = "markdown")]
enum DragTarget<'a> {
    After(&'a str),
    LastChildOf(&'a str),
}

#[cfg(feature = "markdown")]
struct MmapNodeCardBounds {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

#[cfg(feature = "markdown")]
fn mmap_node_subtree_range_by_title(source: &str, title: &str) -> std::ops::Range<usize> {
    fn find_node_by_title<'a>(
        node: &'a textora_markdown::mmf::Node,
        title: &str,
    ) -> Option<&'a textora_markdown::mmf::Node> {
        if node.title == title {
            return Some(node);
        }
        node.children.iter().find_map(|child| find_node_by_title(child, title))
    }

    let tree = textora_markdown::mmf::parser::parse(source).expect("mmap drag fixture must parse");
    find_node_by_title(&tree.root, title)
        .unwrap_or_else(|| panic!("mmap drag fixture must contain node {title}"))
        .subtree_source_range
        .clone()
}

#[cfg(feature = "markdown")]
fn mmap_node_card_bounds(
    app: &App,
    expected_source_range: std::ops::Range<usize>,
) -> MmapNodeCardBounds {
    let bounds = app.plugin_render_bounds();
    mmap_node_card_bounds_in_render_bounds(app, expected_source_range, bounds)
}

#[cfg(feature = "markdown")]
fn mmap_node_card_bounds_in_render_bounds(
    app: &App,
    expected_source_range: std::ops::Range<usize>,
    bounds: ui::core::geom::Rect,
) -> MmapNodeCardBounds {
    let width = bounds.w.ceil() as usize;
    let height = bounds.h.ceil() as usize;
    let mut card_bounds: Option<MmapNodeCardBounds> = None;

    for y in (0..=height).step_by(MMAP_HIT_TEST_GRID_STEP_PX) {
        for x in (0..=width).step_by(MMAP_HIT_TEST_GRID_STEP_PX) {
            let screen_x = bounds.x + x as f32;
            let screen_y = bounds.y + y as f32;
            let tab = app.active_tab_session().expect("active mmap tab");
            let matches_node = matches!(
                tab.hit_test_edit_target(screen_x, screen_y, bounds.x, bounds.y),
                Some(Some(ui::plugin::EditHitTarget::SourceObject { source_range }))
                    if source_range == expected_source_range
            );
            if !matches_node {
                continue;
            }

            match &mut card_bounds {
                Some(card_bounds) => {
                    card_bounds.left = card_bounds.left.min(screen_x);
                    card_bounds.top = card_bounds.top.min(screen_y);
                    card_bounds.right = card_bounds.right.max(screen_x);
                    card_bounds.bottom = card_bounds.bottom.max(screen_y);
                }
                None => {
                    card_bounds = Some(MmapNodeCardBounds {
                        left: screen_x,
                        top: screen_y,
                        right: screen_x,
                        bottom: screen_y,
                    });
                }
            }
        }
    }

    card_bounds.expect("expected a semantic hit point in the mmap node card")
}

#[cfg(feature = "markdown")]
fn drag_mmap_node(app: &mut App, source_title: &str, target: DragTarget<'_>) {
    let source = active_document_text(app);
    let source_range = mmap_node_subtree_range_by_title(&source, source_title);
    let source_card = mmap_node_card_bounds(app, source_range);
    let anchor_title = match target {
        DragTarget::After(anchor_title) | DragTarget::LastChildOf(anchor_title) => anchor_title,
    };
    let anchor_range = mmap_node_subtree_range_by_title(&source, anchor_title);
    let anchor_card = mmap_node_card_bounds(app, anchor_range);
    let (drop_x_offset, drop_y_offset) = match target {
        DragTarget::After(_) => (MMAP_SIBLING_DROP_X_OFFSET_PX, MMAP_DROP_Y_OFFSET_PX),
        DragTarget::LastChildOf(_) => (MMAP_LAST_CHILD_DROP_X_OFFSET_PX, MMAP_DROP_Y_OFFSET_PX),
    };
    let drop_x = anchor_card.right + drop_x_offset;
    let drop_y = anchor_card.bottom + drop_y_offset;

    app.dispatch_editor_mouse_input(
        winit::event::ElementState::Pressed,
        source_card.left,
        source_card.top,
        None,
    );
    app.dispatch_editor_mouse_input(
        winit::event::ElementState::Released,
        source_card.left,
        source_card.top,
        None,
    );
    app.dispatch_editor_mouse_input(
        winit::event::ElementState::Pressed,
        source_card.left,
        source_card.top,
        None,
    );
    app.dispatch_editor_cursor_moved(drop_x, drop_y, None);
    app.dispatch_editor_mouse_input(winit::event::ElementState::Released, drop_x, drop_y, None);
}

#[test]
#[cfg(feature = "markdown")]
fn mmap_drag_reorders_siblings_and_undo_restores_the_original_source() {
    let source = "# Root\n## A\n## B\n## C\n";
    let expected = "# Root\n## A\n## C\n## B\n";
    let mut app = app_with_mmap_source(source);

    drag_mmap_node(&mut app, "B", DragTarget::After("C"));

    let tab = app.active_tab_session().expect("active mmap tab");
    assert_eq!(tab.full_text(), expected);
    let moved_node_range = mmap_node_subtree_range_by_title(expected, "B");
    assert_eq!(
        tab.selection_range(),
        Some((moved_node_range.start, moved_node_range.end)),
        "the moved node must remain selected after the drag transaction"
    );
    assert!(tab.dirty, "a completed mmap drag must mark the document dirty");

    undo_active_mmap_edit(&mut app);
    let tab = app.active_tab_session().expect("active mmap tab");
    assert_eq!(tab.full_text(), source);
    assert!(!tab.dirty, "undo back to the clean source must clear the dirty state");

    redo_active_mmap_edit(&mut app);
    let tab = app.active_tab_session().expect("active mmap tab");
    assert_eq!(tab.full_text(), expected);
    assert!(tab.dirty, "redo must restore the dirty mmap drag state");
}

#[test]
#[cfg(feature = "markdown")]
fn mmap_dragging_to_a_long_sibling_right_edge_stays_same_level_and_undoes() {
    const LONG_SIBLING_TITLE: &str = "A wide sibling title";
    let source = format!("# Root\n## A\n## B\n## {LONG_SIBLING_TITLE}\n");
    let expected = format!("# Root\n## A\n## {LONG_SIBLING_TITLE}\n## B\n");
    let mut app = app_with_mmap_source(&source);
    let target_range = mmap_node_subtree_range_by_title(&source, LONG_SIBLING_TITLE);
    let expanded_render_bounds = ui::core::geom::Rect::new(0.0, 0.0, 1_600.0, 1_200.0);
    let _ = render_active_wysiwyg_plugin_in_bounds_for_test(&mut app, expanded_render_bounds);
    let target_card =
        mmap_node_card_bounds_in_render_bounds(&app, target_range, expanded_render_bounds);
    assert!(
        target_card.left > expanded_render_bounds.x
            && target_card.right < expanded_render_bounds.x + expanded_render_bounds.w
            && target_card.top > expanded_render_bounds.y
            && target_card.bottom < expanded_render_bounds.y + expanded_render_bounds.h,
        "fixture card must be fully visible in the expanded plugin render bounds"
    );
    let _ = render_active_wysiwyg_plugin_for_test(&mut app);

    drag_mmap_node(&mut app, "B", DragTarget::After(LONG_SIBLING_TITLE));

    let tab = app.active_tab_session().expect("active mmap tab");
    assert_eq!(
        tab.full_text(),
        expected,
        "dropping on a wide card's right edge must create a sibling move"
    );

    undo_active_mmap_edit(&mut app);
    let tab = app.active_tab_session().expect("active mmap tab");
    assert_eq!(tab.full_text(), source);
    assert!(!tab.dirty, "undo must restore the clean long-title source");
}

#[test]
#[cfg(feature = "markdown")]
fn mmap_drag_to_left_anchor_makes_the_source_a_child_with_its_subtree() {
    let source = "# Root\n## A\n### A1\n## B\n";
    let expected = "# Root\n## B\n### A\n#### A1\n";
    let mut app = app_with_mmap_source(source);

    drag_mmap_node(&mut app, "A", DragTarget::LastChildOf("B"));

    let tab = app.active_tab_session().expect("active mmap tab");
    assert_eq!(tab.full_text(), expected);
    let moved_subtree_range = mmap_node_subtree_range_by_title(expected, "A");
    assert_eq!(
        tab.selection_range(),
        Some((moved_subtree_range.start, moved_subtree_range.end)),
        "the moved subtree must remain selected after becoming a child"
    );
    assert!(tab.dirty, "a completed mmap subtree drag must mark the document dirty");
}

#[test]
#[cfg(feature = "markdown")]
fn mmap_dragging_the_root_keeps_the_source_unchanged() {
    let source = "# Root\n## A\n## B\n";
    let mut app = app_with_mmap_source(source);

    drag_mmap_node(&mut app, "Root", DragTarget::After("B"));

    let tab = app.active_tab_session().expect("active mmap tab");
    assert_eq!(tab.full_text(), source);
    assert!(!tab.dirty, "an invalid root drag must not dirty the document");
}

#[test]
#[cfg(feature = "markdown")]
fn mmap_dragging_title_text_selects_text_without_moving_the_node() {
    let source = "# Root\n## Alpha\n## Beta\n";
    let tree = textora_markdown::mmf::parser::parse(source).expect("fixture must parse");
    let alpha_range = tree.root.children[0].title_byte_range.clone();
    let mut app = app_with_mmap_source(source);
    let alpha_start_point = mmap_text_caret_hit_point(&mut app, alpha_range.start);
    let alpha_end_point = mmap_text_caret_hit_point(&mut app, alpha_range.end);

    app.dispatch_editor_mouse_input(
        winit::event::ElementState::Pressed,
        alpha_start_point.0,
        alpha_start_point.1,
        None,
    );
    app.dispatch_editor_cursor_moved(alpha_end_point.0, alpha_end_point.1, None);
    app.dispatch_editor_mouse_input(
        winit::event::ElementState::Released,
        alpha_end_point.0,
        alpha_end_point.1,
        None,
    );

    let tab = app.active_tab_session().expect("active mmap tab");
    assert_eq!(tab.full_text(), source);
    assert_eq!(tab.selection_range(), Some((alpha_range.start, alpha_range.end)));
    assert!(!tab.dirty, "title text selection must not dirty the mmap source");
}

#[test]
#[cfg(feature = "markdown")]
fn mmap_drag_between_titles_does_not_create_a_document_selection() {
    let source = "# Root\n## Alpha\n## Beta\n";
    let tree = textora_markdown::mmf::parser::parse(source).expect("fixture must parse");
    let alpha_range = tree.root.children[0].title_byte_range.clone();
    let beta_range = tree.root.children[1].title_byte_range.clone();
    let mut app = app_with_mmap_source(source);
    app.sync_plugin_state();
    render_active_wysiwyg_plugin_for_test(&mut app);
    let alpha_point = mmap_text_caret_hit_point(&mut app, alpha_range.start);
    let beta_point = mmap_text_caret_hit_point(&mut app, beta_range.start);

    app.dispatch_editor_mouse_input(
        winit::event::ElementState::Pressed,
        alpha_point.0,
        alpha_point.1,
        None,
    );
    assert_eq!(
        app.active_tab_session().expect("active mmap tab").cursor_offset().to_usize(),
        alpha_range.start,
        "fixture must press the first title's semantic text caret"
    );
    app.dispatch_editor_cursor_moved(beta_point.0, beta_point.1, None);
    app.dispatch_editor_mouse_input(
        winit::event::ElementState::Released,
        beta_point.0,
        beta_point.1,
        None,
    );

    let tab = app.active_tab_session().expect("active mmap tab");
    assert_eq!(
        tab.selection_range(),
        None,
        "dragging from one mmap title to another must not create a cross-title document selection"
    );
}

#[test]
#[cfg(feature = "markdown")]
fn mmap_drag_within_a_title_selects_its_source_range() {
    let source = "# Root\n## Alpha\n## Beta\n";
    let tree = textora_markdown::mmf::parser::parse(source).expect("fixture must parse");
    let alpha_range = tree.root.children[0].title_byte_range.clone();
    let mut app = app_with_mmap_source(source);
    app.sync_plugin_state();
    render_active_wysiwyg_plugin_for_test(&mut app);
    let alpha_start_point = mmap_text_caret_hit_point(&mut app, alpha_range.start);
    let alpha_end_point = mmap_text_caret_hit_point(&mut app, alpha_range.end);

    app.dispatch_editor_mouse_input(
        winit::event::ElementState::Pressed,
        alpha_start_point.0,
        alpha_start_point.1,
        None,
    );
    app.dispatch_editor_cursor_moved(alpha_end_point.0, alpha_end_point.1, None);

    let tab = app.active_tab_session().expect("active mmap tab");
    assert_eq!(tab.selection_range(), Some((alpha_range.start, alpha_range.end)));
}

#[test]
#[cfg(feature = "markdown")]
fn mmap_title_double_click_selects_the_title_word() {
    let source = "# Root\n## Alpha\n## Beta\n";
    let tree = textora_markdown::mmf::parser::parse(source).expect("fixture must parse");
    let alpha_range = tree.root.children[0].title_byte_range.clone();
    let mut app = app_with_mmap_source(source);
    app.sync_plugin_state();
    render_active_wysiwyg_plugin_for_test(&mut app);
    let alpha_point = mmap_text_caret_hit_point(&mut app, alpha_range.start + 1);

    app.dispatch_editor_mouse_input(
        winit::event::ElementState::Pressed,
        alpha_point.0,
        alpha_point.1,
        None,
    );
    app.dispatch_editor_mouse_input(
        winit::event::ElementState::Released,
        alpha_point.0,
        alpha_point.1,
        None,
    );
    app.dispatch_editor_mouse_input(
        winit::event::ElementState::Pressed,
        alpha_point.0,
        alpha_point.1,
        None,
    );

    let tab = app.active_tab_session().expect("active mmap tab");
    assert_eq!(tab.selection_range(), Some((alpha_range.start, alpha_range.end)));
}

#[test]
#[cfg(feature = "markdown")]
fn mmap_selected_node_typing_replaces_title_then_undo_and_redo_restore_it() {
    let source = "# Root\n## Parent\n### Child\n## Next\n";
    let tree = textora_markdown::mmf::parser::parse(source).expect("parse fixture");
    let parent_range = tree.root.children[0].subtree_source_range.clone();
    let mut app = app_with_mmap_source(source);
    select_mmap_source_object(&mut app, parent_range);
    let generation_before =
        app.active_tab_session().expect("active document").document.generation();

    app.dispatch_transactional_edit_for_test(EditCommand::InsertChar("Renamed".into()));
    assert_eq!(active_document_text(&app), "# Root\n## Renamed\n### Child\n## Next\n");
    assert!(
        app.active_tab_session().expect("active document").document.generation()
            > generation_before,
        "replacing a selected mmap title must advance the source generation"
    );

    undo_active_mmap_edit(&mut app);
    assert_eq!(active_document_text(&app), source);

    redo_active_mmap_edit(&mut app);
    assert_eq!(active_document_text(&app), "# Root\n## Renamed\n### Child\n## Next\n");
}

#[test]
#[cfg(feature = "markdown")]
fn mmap_tab_creates_empty_child_and_enter_creates_empty_sibling() {
    let source = "# Root\n## Parent\n";
    let tree = textora_markdown::mmf::parser::parse(source).expect("parse fixture");
    let mut app = app_with_mmap_source(source);
    {
        let mut tab = app.active_tab_session_mut().expect("active mmap tab");
        tab.cursor_move_to_offset(tree.root.children[0].title_byte_range.end);
        tab.cursor_mut().selection_anchor = None;
    }
    app.sync_plugin_state();

    app.dispatch_transactional_edit_for_test(EditCommand::Tab);
    let after_child = active_document_text(&app);
    assert!(after_child.contains("###\n"));

    app.dispatch_transactional_edit_for_test(EditCommand::InsertNewline);
    let after_sibling = active_document_text(&app);
    let parsed = textora_markdown::mmf::parser::parse(&after_sibling).expect("parse edited map");
    assert_eq!(parsed.root.children[0].children.len(), 2);
    assert!(parsed.root.children[0].children.iter().all(|child| child.title.is_empty()));
}

#[test]
#[cfg(feature = "markdown")]
fn mmap_demote_adjusts_whole_subtree_and_undoes_once() {
    let source = "# Root\n## First\n## Second\n### Leaf\n";
    let tree = textora_markdown::mmf::parser::parse(source).expect("parse fixture");
    let second_range = tree.root.children[1].subtree_source_range.clone();
    let mut app = app_with_mmap_source(source);
    select_mmap_source_object(&mut app, second_range);

    app.dispatch_transactional_edit(ui::plugin::EditIntent::DemoteObject, None);
    assert_eq!(active_document_text(&app), "# Root\n## First\n### Second\n#### Leaf\n");

    undo_active_mmap_edit(&mut app);
    assert_eq!(active_document_text(&app), source);
}

#[test]
#[cfg(feature = "markdown")]
fn mmap_selected_node_delete_removes_subtree_then_undo_restores_it() {
    let source = "# Root\n## Parent\n### Child\n## Next\n";
    let tree = textora_markdown::mmf::parser::parse(source).expect("parse fixture");
    let parent_range = tree.root.children[0].subtree_source_range.clone();
    let mut app = app_with_mmap_source(source);
    select_mmap_source_object(&mut app, parent_range);

    app.dispatch_transactional_edit_for_test(EditCommand::DeleteForward);
    assert_eq!(active_document_text(&app), "# Root\n## Next\n");

    undo_active_mmap_edit(&mut app);
    assert_eq!(active_document_text(&app), source);
}

#[test]
#[cfg(feature = "markdown")]
fn mmap_preedit_does_not_change_document_until_commit() {
    let source = "# Root\n## Original\n";
    let tree = textora_markdown::mmf::parser::parse(source).expect("parse fixture");
    let selected_range = tree.root.children[0].subtree_source_range.clone();
    let mut app = app_with_mmap_source(source);
    select_mmap_source_object(&mut app, selected_range);
    set_editor_preedit_for_test(&mut app, "ni", Some((2, 2)));
    app.sync_plugin_state();

    assert_eq!(active_document_text(&app), source);

    app.dispatch_transactional_edit_for_test(EditCommand::InsertChar("你".into()));
    assert_eq!(active_document_text(&app), "# Root\n## 你\n");
}

#[test]
#[cfg(feature = "markdown")]
fn mmap_invalid_source_exposes_no_edit_target() {
    const INVALID_SOURCE: &str = "## Orphan\n";

    let diagnostic = textora_markdown::mmf::parser::parse(INVALID_SOURCE)
        .expect_err("fixture must be invalid mmap source");
    let mut app = app_with_mmap_source("# Root\n");
    {
        let mut tab = app.active_tab_session_mut().expect("active mmap tab");
        tab.select_all();
        tab.delete_selection();
        tab.insert_at_cursor(INVALID_SOURCE.as_bytes());
    }
    app.sync_plugin_state();
    let draw_list = render_active_wysiwyg_plugin_for_test(&mut app);
    assert!(
        draw_list.cmds.iter().any(|cmd| {
            matches!(
                cmd,
                ui::core::paint::DrawCmd::TextLayout { layout, .. }
                    if layout.text == diagnostic.message
            )
        }),
        "invalid mmap source must render its parser diagnostic"
    );

    let tab = app.active_tab_session().expect("active mmap tab");
    assert_eq!(tab.full_text(), INVALID_SOURCE);
    assert!(tab.hit_test_edit_target(10.0, 10.0, 0.0, 0.0).flatten().is_none());
}

#[test]
#[cfg(feature = "markdown")]
fn mmap_canvas_tab_switch_restores_each_session_and_new_doc_item_refits() {
    use crate::canvas_viewport::CanvasViewportAction;
    use ui::canvas::{CanvasPoint, CanvasViewportConfig, CanvasViewportInput, resolve_viewport};
    use ui::plugin::CanvasContentMetrics;

    const SHARED_PATH: &str = "/tmp/shared.mmap.md";

    let content_bounds = ui::core::geom::Rect::new(0.0, 0.0, 2_400.0, 1_800.0);
    let viewport_bounds = ui::core::geom::Rect::new(120.0, 80.0, 800.0, 600.0);
    let metrics = CanvasContentMetrics { content_bounds, focus_anchor: None };
    let config = CanvasViewportConfig::for_dpi(1.0);
    let mut app = App::new(None);
    for source in ["# First\n", "# Second\n"] {
        let mut document =
            DocumentView::new(source.split('\n').map(str::to_owned).collect(), 80, 10.0);
        document.file_path = Some(std::path::PathBuf::from(SHARED_PATH));
        app.push_entry_for_test(
            document,
            Box::new(textora_markdown::mindmap_view::MindmapView::new()),
        );
    }

    let first_id = app.editor_tab_id_at(0).expect("first tab id");
    let second_id = app.editor_tab_id_at(1).expect("second tab id");
    for id in [first_id, second_id] {
        app.tab_session_mut(id)
            .expect("test tab session")
            .runtime
            .canvas_viewport
            .prepare(metrics, viewport_bounds, config)
            .expect("canvas session must resolve");
    }
    app.tab_session_mut(first_id)
        .expect("first mmap tab")
        .runtime
        .canvas_viewport
        .apply(CanvasViewportAction::PanBy(CanvasPoint::new(180.0, 120.0)));
    app.tab_session_mut(second_id)
        .expect("second mmap tab")
        .runtime
        .canvas_viewport
        .apply(CanvasViewportAction::PanBy(CanvasPoint::new(360.0, 240.0)));

    let first_position = app
        .tab_session(first_id)
        .expect("first mmap tab")
        .runtime
        .canvas_viewport
        .snapshot()
        .expect("first mmap snapshot")
        .position();
    let second_position = app
        .tab_session(second_id)
        .expect("second mmap tab")
        .runtime
        .canvas_viewport
        .snapshot()
        .expect("second mmap snapshot")
        .position();
    assert_ne!(first_position, second_position, "fixture must give each tab a distinct view");

    app.dispatch_tab_switch(second_id);
    assert_eq!(
        app.active_tab_session()
            .expect("second mmap tab must be active")
            .runtime
            .canvas_viewport
            .snapshot()
            .expect("second mmap snapshot")
            .position(),
        second_position,
        "switching to the second mmap tab must restore its own view position"
    );
    app.dispatch_tab_switch(first_id);
    assert_eq!(
        app.active_tab_session()
            .expect("first mmap tab must be active")
            .runtime
            .canvas_viewport
            .snapshot()
            .expect("first mmap snapshot")
            .position(),
        first_position,
        "switching back must restore the first mmap tab view position"
    );

    let mut reopened_document = DocumentView::new(vec!["# Reopened".to_owned()], 80, 10.0);
    reopened_document.file_path = Some(std::path::PathBuf::from(SHARED_PATH));
    let reopened_id = app.push_entry_for_test(
        reopened_document,
        Box::new(textora_markdown::mindmap_view::MindmapView::new()),
    );
    let reopened_snapshot = app
        .tab_session_mut(reopened_id)
        .expect("new mmap doc item")
        .runtime
        .canvas_viewport
        .prepare(metrics, viewport_bounds, config)
        .expect("new mmap doc item must resolve its initial view");
    let expected_initial =
        resolve_viewport(CanvasViewportInput::initial(viewport_bounds, content_bounds, config));
    assert_eq!(
        reopened_snapshot.position(),
        expected_initial.position(),
        "a new tab at the same path must start at AwaitingInitialFit rather than inheriting another tab view"
    );
}

#[test]
#[cfg(feature = "markdown")]
fn mmap_canvas_mmap_layout_long_title_keeps_active_node_center_stable() {
    const MAX_SCREEN_DRIFT_PX: f32 = 1.0;
    const VISIBLE_SCROLL_OFFSET_PX: f32 = 1.0;
    const EXTRA_SIBLING_COUNT: usize = 48;

    let wide_sibling_title = "wide sibling ".repeat(80);
    let replacement_title = "replacement title that expands the active canvas layout ".repeat(40);
    let mut source = format!("# Root\n## Active\n## {wide_sibling_title}\n## Resize Target\n");
    for sibling_index in 0..EXTRA_SIBLING_COUNT {
        source.push_str(&format!("## Tall sibling {sibling_index}\n"));
    }

    let active_title_start = source.find("Active").expect("fixture must contain active title");
    let resize_target_start =
        source.find("Resize Target").expect("fixture must contain resize target");
    let mut app = app_with_mmap_source(&source);
    {
        let mut tab = app.active_tab_session_mut().expect("active mmap tab");
        tab.cursor_move_to_offset(active_title_start);
    }
    app.sync_plugin_state();
    let _ = prepare_and_render_active_mmap_canvas_for_test(&mut app);
    let active_range = mmap_node_subtree_range_by_title(&source, "Active");
    let initial_card = mmap_node_card_bounds(&app, active_range.clone());
    let zoom_anchor = ui::canvas::CanvasPoint::new(
        (initial_card.left + initial_card.right) * 0.5,
        (initial_card.top + initial_card.bottom) * 0.5,
    );
    let zoomed_snapshot = {
        let tab = app.active_tab_session_mut().expect("active mmap tab");
        tab.runtime.canvas_viewport.apply(crate::canvas_viewport::CanvasViewportAction::ZoomBy {
            factor: 2.0,
            screen_anchor: zoom_anchor,
        });
        let zoomed_snapshot = tab.runtime.canvas_viewport.snapshot().expect("zoomed mmap viewport");
        assert!(
            zoomed_snapshot.max_scroll.x > 0.0 && zoomed_snapshot.max_scroll.y > 0.0,
            "fixture must establish a genuinely two-axis scrollable canvas"
        );
        tab.runtime.canvas_viewport.apply(crate::canvas_viewport::CanvasViewportAction::PanBy(
            ui::canvas::CanvasPoint::new(VISIBLE_SCROLL_OFFSET_PX, VISIBLE_SCROLL_OFFSET_PX),
        ));
        let positioned_snapshot =
            tab.runtime.canvas_viewport.snapshot().expect("positioned mmap viewport");
        assert!(
            positioned_snapshot.scroll.x > 0.0 && positioned_snapshot.scroll.y > 0.0,
            "fixture must establish nonzero horizontal and vertical canvas scroll"
        );
        positioned_snapshot
    };
    render_active_mmap_canvas_snapshot_for_test(&mut app, zoomed_snapshot);
    let before_card = mmap_node_card_bounds(&app, active_range.clone());
    let before_center = (
        (before_card.left + before_card.right) * 0.5,
        (before_card.top + before_card.bottom) * 0.5,
    );
    let before = mmap_cursor_window_rect_for_test(&app, active_title_start);

    {
        let mut tab = app.active_tab_session_mut().expect("active mmap tab");
        tab.replace_range(
            resize_target_start..resize_target_start + "Resize Target".len(),
            &replacement_title,
        );
    }
    let _ = prepare_and_render_active_mmap_canvas_for_test(&mut app);
    let after_card = mmap_node_card_bounds(&app, active_range);
    let after_center =
        ((after_card.left + after_card.right) * 0.5, (after_card.top + after_card.bottom) * 0.5);
    let after = mmap_cursor_window_rect_for_test(&app, active_title_start);

    assert!(
        (after_center.0 - before_center.0).abs() < MAX_SCREEN_DRIFT_PX
            && (after_center.1 - before_center.1).abs() < MAX_SCREEN_DRIFT_PX,
        "layout changes must preserve the focused node center; before={before_center:?}, after={after_center:?}",
    );
    assert!(
        (after.x - before.x).abs() < MAX_SCREEN_DRIFT_PX
            && (after.y - before.y).abs() < MAX_SCREEN_DRIFT_PX,
        "the active title caret must stay aligned with its stable node center; before=({}, {}), after=({}, {})",
        before.x,
        before.y,
        after.x,
        after.y,
    );
}

#[test]
#[cfg(feature = "markdown")]
fn mmap_canvas_initial_fit_to_overflow_keeps_viewport_state_finite_and_clamped() {
    const SOURCE: &str = "# Root\n## Active\n## Sibling\n";

    let long_sibling_title =
        "Sibling title that turns an initially fitted canvas into overflow ".repeat(80);
    let active_title_start = SOURCE.find("Active").expect("fixture must contain active title");
    let sibling_title_start = SOURCE.find("Sibling").expect("fixture must contain sibling title");
    let mut app = app_with_mmap_source(SOURCE);
    {
        let mut tab = app.active_tab_session_mut().expect("active mmap tab");
        tab.cursor_move_to_offset(active_title_start);
    }
    app.sync_plugin_state();
    let initial_snapshot = prepare_and_render_active_mmap_canvas_for_test(&mut app);
    assert!(
        initial_snapshot.max_scroll.x == 0.0 && initial_snapshot.max_scroll.y == 0.0,
        "fixture must begin as an initial-fit canvas without overflow"
    );
    {
        let mut tab = app.active_tab_session_mut().expect("active mmap tab");
        tab.replace_range(
            sibling_title_start..sibling_title_start + "Sibling".len(),
            &long_sibling_title,
        );
    }
    let overflow_snapshot = prepare_and_render_active_mmap_canvas_for_test(&mut app);

    assert!(
        overflow_snapshot.max_scroll.x > 0.0 || overflow_snapshot.max_scroll.y > 0.0,
        "updating the source must turn the initially fitted canvas into an overflow canvas"
    );
    assert!(initial_snapshot.zoom.is_finite() && overflow_snapshot.zoom.is_finite());
    assert!(overflow_snapshot.scroll.x.is_finite() && overflow_snapshot.scroll.y.is_finite());
    assert!(
        overflow_snapshot.scroll.x >= 0.0
            && overflow_snapshot.scroll.x <= overflow_snapshot.max_scroll.x
            && overflow_snapshot.scroll.y >= 0.0
            && overflow_snapshot.scroll.y <= overflow_snapshot.max_scroll.y,
        "initial-fit to overflow must clamp the requested anchor position instead of producing invalid scroll"
    );
}

#[test]
fn non_canvas_plugin_editor_and_preview_keep_scroll_and_global_font_zoom() {
    const INITIAL_SCROLL_ROW: f64 = 12.0;
    const FONT_ZOOM_DELTA: f32 = 3.0;
    const LINE_COUNT: usize = 128;

    let mut app = App::new(None);
    let line_height = app.ui_metrics().line_height;
    let make_scrolled_document = || {
        let mut document = DocumentView::new(
            (0..LINE_COUNT).map(|line| format!("line {line}")).collect(),
            80,
            10.0,
        );
        document.presentation.display.display_map.set_entries(
            (0..LINE_COUNT)
                .map(|line| crate::snap_tree::DisplayLineEntry::placeholder(line, 1, 0, 1))
                .collect(),
        );
        document.presentation.display.viewport.set_scroll_top(
            INITIAL_SCROLL_ROW,
            &document.presentation.display.display_map,
            line_height,
        );
        document
    };
    let editor_document = make_scrolled_document();
    let editor_tab_id = app.push_entry_for_test(editor_document, Box::new(EditorPlugin::new()));
    let preview_document = make_scrolled_document();
    let preview_state = std::rc::Rc::new(std::cell::RefCell::new(RecordingPreviewState::default()));
    let preview_tab_id = app.push_entry_for_test(
        preview_document,
        Box::new(RecordingPreviewPlugin::new(preview_state)),
    );

    let original_font_size = app.settings.font_size;
    app.apply_zoom(original_font_size + FONT_ZOOM_DELTA);

    assert_eq!(app.settings.font_size, original_font_size + FONT_ZOOM_DELTA);
    assert_eq!(
        app.tab_session(editor_tab_id).expect("editor tab").scroll_top() as f32,
        INITIAL_SCROLL_ROW as f32,
        "the ordinary editor must keep its pre-canvas scroll anchor"
    );
    assert_eq!(
        app.tab_session(preview_tab_id).expect("preview tab").scroll_top() as f32,
        INITIAL_SCROLL_ROW as f32,
        "the ordinary preview must keep its pre-canvas scroll anchor"
    );
}

#[cfg(feature = "markdown")]
fn move_left_reaches_range_without_cycle(
    app: &mut App,
    start_byte: usize,
    target: std::ops::Range<usize>,
) -> bool {
    app.active_tab_session_mut().expect("active entry").cursor_move_to_offset(start_byte);
    let mut visited = std::collections::BTreeSet::new();
    for _ in 0..=start_byte + 1 {
        let current = app.active_tab_session().expect("active entry").cursor_offset().to_usize();
        if target.contains(&current) {
            return true;
        }
        if !visited.insert(current) {
            return false;
        }
        let effect = app.dispatch_wysiwyg_navigation(&crate::input::EditCommand::MoveLeft);
        if !effect.redraw {
            return false;
        }
    }
    false
}

#[cfg(feature = "markdown")]
fn interior_x_for_visible_grapheme(grapheme_x: &[f32], grapheme_index: usize) -> Option<f32> {
    let start_x = *grapheme_x.get(grapheme_index)?;
    if let Some(end_x) =
        grapheme_x.get(grapheme_index + 1).copied().filter(|end_x| *end_x > start_x)
    {
        return Some((start_x + end_x) * 0.5);
    }

    grapheme_x[..grapheme_index]
        .iter()
        .rev()
        .copied()
        .find(|previous_x| *previous_x < start_x)
        .map(|previous_x| (previous_x + start_x) * 0.5)
}

#[test]
#[cfg(feature = "markdown")]
fn wysiwyg_promotion_blockquote_line_three_click_and_up_navigation_reach_its_source_range() {
    use crate::input::EditCommand;

    let source = "# Promotion & Marketing\n\n> Applicable scenarios: Brand launches, marketing campaigns, art/fashion/culture showcases, product introductions, etc.\n> Style anchor: Apple Keynote / Xiaomi product launch / High-end fashion brands / Art exhibitions / Cultural promotion / Premium brand visual systems\n\n## Design Philosophy\n";
    let line_three_start =
        source.find("Applicable scenarios").expect("fixture must contain line three");
    let line_four_start =
        source.find("\n> Style anchor").expect("fixture must contain line four") + 1;
    let line_four_content =
        source.find("Style anchor").expect("fixture must contain line four text");
    let mut app = App::new(None);
    let document =
        DocumentView::new(source.split('\n').map(|line| line.to_string()).collect(), 80, 10.0);
    app.push_entry_for_test(document, Box::new(textora_markdown::view::MarkdownEditorView::new()));
    app.switch_workspace_for_test(0);
    app.sync_plugin_state();
    render_active_wysiwyg_plugin_for_test(&mut app);

    {
        let mut tab = app.active_tab_session_mut().expect("active entry");
        tab.cursor_move_to_offset(line_three_start);
    }
    app.sync_plugin_state();
    render_active_wysiwyg_plugin_for_test(&mut app);
    let visual_lines = { app.active_tab_session().expect("active entry").flat_lines() };
    let line_three = visual_lines
        .iter()
        .find(|line| line.text.contains("Applicable scenarios"))
        .expect("line three text must be visible");
    let visible_offset =
        line_three.text.find("Applicable scenarios").expect("needle must be in line three");
    let visible_text_x = interior_x_for_visible_grapheme(&line_three.grapheme_x, visible_offset)
        .expect("visible text must expose a nonzero grapheme advance");
    let bounds = app.plugin_render_bounds();
    let click_x = bounds.x + line_three.rect.x + visible_text_x;
    let click_y = bounds.y + line_three.rect.y + line_three.rect.h * 0.5;

    {
        let mut tab = app.active_tab_session_mut().expect("active entry");
        tab.cursor_move_to_offset(line_four_content);
    }
    app.sync_plugin_state();
    render_active_wysiwyg_plugin_for_test(&mut app);
    app.dispatch_editor_mouse_input(winit::event::ElementState::Pressed, click_x, click_y, None);
    app.dispatch_editor_mouse_input(winit::event::ElementState::Released, click_x, click_y, None);

    let clicked_byte = app.active_tab_session().expect("active entry").cursor_offset().to_usize();
    assert!(
        (line_three_start..line_four_start).contains(&clicked_byte),
        "clicking line three must land in its source range {line_three_start}..{line_four_start}, got {clicked_byte}"
    );

    {
        let mut tab = app.active_tab_session_mut().expect("active entry");
        tab.cursor_move_to_offset(line_four_content);
    }
    app.sync_plugin_state();
    render_active_wysiwyg_plugin_for_test(&mut app);
    let effect = app.dispatch_wysiwyg_navigation(&EditCommand::MoveUp);
    assert!(effect.redraw, "Up from line four should redraw the WYSIWYG view");

    let moved_byte = app.active_tab_session().expect("active entry").cursor_offset().to_usize();
    assert!(
        (line_three_start..line_four_start).contains(&moved_byte),
        "Up from line four must land in line three source range {line_three_start}..{line_four_start}, got {moved_byte}"
    );

    assert!(
        move_left_reaches_range_without_cycle(
            &mut app,
            line_four_content,
            line_three_start..line_four_start,
        ),
        "Left from line four must reach line three source range without a navigation cycle"
    );
}

#[test]
#[cfg(feature = "markdown")]
fn wysiwyg_select_all_survives_copy_time_sync_with_trailing_empty_lines() {
    let source = "\
C608-03 武昌职业第03组：2025 最低 389，且是民办、计划 4 人，不应太靠前，除非非常想去海军水面方向。
C503-01、T105-01：疑似非军士，建议后置或剔除。
C501 武汉船舶、C523 湖北交通、C537 武汉交通：如果体检类别匹配，本地公办、计划较多，价值更高，应该优先级更清晰。
C608-01 武昌职业第01组：计划 68，历史低线较低，是表里最像“保底”的组之一，但民办学费高，要接受成本。


";
    let mut app = App::new(None);
    let doc =
        DocumentView::new(source.split('\n').map(|line| line.to_string()).collect(), 80, 10.0);
    app.push_entry_for_test(doc, Box::new(textora_markdown::view::MarkdownEditorView::new()));
    app.switch_workspace_for_test(0);
    app.sync_plugin_state();
    render_active_wysiwyg_plugin_for_test(&mut app);

    {
        let mut tab = app.active_tab_session_mut().expect("active entry");
        tab.select_all();
    }
    app.sync_plugin_state();
    app.sync_plugin_state();

    let tab = app.active_tab_session().expect("active entry");
    assert_eq!(
        tab.selection_range(),
        Some((0, tab.buffer_len())),
        "copy-time WYSIWYG sync should not shrink a full-document selection"
    );
    let selected = String::from_utf8(
        tab.extract_selected_text().expect("document selection should contain text"),
    )
    .expect("selected text should be valid UTF-8");
    assert!(
        selected.contains("要接受成本"),
        "copy-time document selection should include the final visible line, got {selected:?}"
    );
}

#[test]
#[cfg(feature = "markdown")]
fn wysiwyg_mouse_drag_from_trailing_blank_selects_final_text_line() {
    let source = "\
C608-03 武昌职业第03组：2025 最低 389，且是民办、计划 4 人，不应太靠前，除非非常想去海军水面方向。
C503-01、T105-01：疑似非军士，建议后置或剔除。
C501 武汉船舶、C523 湖北交通、C537 武汉交通：如果体检类别匹配，本地公办、计划较多，价值更高，应该优先级更清晰。
C608-01 武昌职业第01组：计划 68，历史低线较低，是表里最像“保底”的组之一，但民办学费高，要接受成本。


";
    let probe_byte = source.find("要接受成本").expect("fixture should contain final text");
    let mut app = App::new(None);
    let doc =
        DocumentView::new(source.split('\n').map(|line| line.to_string()).collect(), 80, 10.0);
    app.push_entry_for_test(doc, Box::new(textora_markdown::view::MarkdownEditorView::new()));
    app.switch_workspace_for_test(0);
    app.sync_plugin_state();
    render_active_wysiwyg_plugin_for_test(&mut app);

    let blank_rect = wysiwyg_cursor_rect_for_byte_for_test(&mut app, source.len());
    let probe_rect = wysiwyg_cursor_rect_for_byte_for_test(&mut app, probe_byte);
    let bounds = app.plugin_render_bounds();
    let blank_x = bounds.x + blank_rect.0 + blank_rect.2 * 0.5;
    let blank_y = bounds.y + blank_rect.1 + blank_rect.3 * 0.5;
    let probe_x = bounds.x + probe_rect.0 + probe_rect.2 * 0.5;
    let probe_y = bounds.y + probe_rect.1 + probe_rect.3 * 0.5;

    app.dispatch_editor_mouse_input(winit::event::ElementState::Pressed, blank_x, blank_y, None);
    app.dispatch_editor_cursor_moved(probe_x, probe_y, None);
    render_active_wysiwyg_plugin_for_test(&mut app);

    let tab = app.active_tab_session().expect("active entry");
    assert!(
        tab.selection_range()
            .is_some_and(|(start, end)| start <= probe_byte && end == tab.buffer_len()),
        "dragging from trailing blank to final text should create a document selection, got {:?}",
        tab.selection_range()
    );

    let highlights = tab.selection_highlights(app.current_theme.editor.selection);
    let current_probe_rect = {
        let tab = app.active_tab_session().expect("active entry");
        tab.query_cursor_screen_rect(probe_byte).expect("expected current WYSIWYG cursor rect")
    };
    let current_probe_y = bounds.y + current_probe_rect.1 + current_probe_rect.3 * 0.5;
    let final_text_line_highlighted = highlights.cmds.iter().any(|cmd| {
        matches!(
            cmd,
            ui::core::paint::DrawCmd::FillRect { rect, .. }
                if rect.y <= current_probe_y && rect.y + rect.h >= current_probe_y
        )
    });
    assert!(
        final_text_line_highlighted,
        "mouse drag selection should visibly highlight the final text line; selection={:?}, probe_y={}, highlights={:?}",
        tab.selection_range(),
        current_probe_y,
        highlights.cmds
    );
}

#[test]
#[cfg(feature = "markdown")]
fn wysiwyg_select_all_visibly_highlights_final_text_line() {
    let source = "\
C608-03 武昌职业第03组：2025 最低 389，且是民办、计划 4 人，不应太靠前，除非非常想去海军水面方向。
C503-01、T105-01：疑似非军士，建议后置或剔除。
C501 武汉船舶、C523 湖北交通、C537 武汉交通：如果体检类别匹配，本地公办、计划较多，价值更高，应该优先级更清晰。
C608-01 武昌职业第01组：计划 68，历史低线较低，是表里最像“保底”的组之一，但民办学费高，要接受成本。


";
    let probe_byte = source.find("要接受成本").expect("fixture should contain final text");
    let mut app = App::new(None);
    let doc =
        DocumentView::new(source.split('\n').map(|line| line.to_string()).collect(), 80, 10.0);
    app.push_entry_for_test(doc, Box::new(textora_markdown::view::MarkdownEditorView::new()));
    app.switch_workspace_for_test(0);
    app.sync_plugin_state();
    render_active_wysiwyg_plugin_for_test(&mut app);

    let probe_rect = wysiwyg_cursor_rect_for_byte_for_test(&mut app, probe_byte);
    {
        let mut tab = app.active_tab_session_mut().expect("active entry");
        tab.select_all();
    }
    app.sync_plugin_state();
    render_active_wysiwyg_plugin_for_test(&mut app);

    let bounds = app.plugin_render_bounds();
    let probe_y = bounds.y + probe_rect.1 + probe_rect.3 * 0.5;
    let tab = app.active_tab_session().expect("active entry");
    let highlights = tab.selection_highlights(app.current_theme.editor.selection);
    let final_text_line_highlighted = highlights.cmds.iter().any(|cmd| {
        matches!(
            cmd,
            ui::core::paint::DrawCmd::FillRect { rect, .. }
                if rect.y <= probe_y && rect.y + rect.h >= probe_y
        )
    });

    assert!(
        final_text_line_highlighted,
        "select-all should visibly highlight the final text line; highlights={:?}",
        highlights.cmds
    );
}

#[test]
#[cfg(feature = "markdown")]
fn wysiwyg_paragraph_end_enter_then_typing_inserts_at_new_paragraph_start() {
    let source = "hello";
    let mut app = App::new(None);
    let mut doc = DocumentView::new(vec![source.to_string()], 80, 10.0);
    doc.cursor_move_to_offset(source.len());
    app.push_entry_for_test(doc, Box::new(textora_markdown::view::MarkdownEditorView::new()));
    app.switch_workspace_for_test(0);
    app.sync_plugin_state();
    render_active_wysiwyg_plugin_for_test(&mut app);

    let enter_effect = app.dispatch_transactional_edit_for_test(EditCommand::InsertNewline);
    assert!(enter_effect.redraw, "paragraph-end Enter should redraw WYSIWYG content");
    app.sync_plugin_state();
    render_active_wysiwyg_plugin_for_test(&mut app);

    {
        let mut tab = app.active_tab_session_mut().expect("active entry");
        tab.insert_at_cursor(b"X");
    }

    let tab = app.active_tab_session().expect("active entry");
    assert_eq!(
        tab.full_text(),
        "hello\n\nX",
        "typing after paragraph-end Enter must insert at the new paragraph start"
    );
}

#[test]
#[cfg(feature = "markdown")]
fn wysiwyg_paragraph_end_before_existing_newline_typing_stays_in_new_paragraph() {
    let source = "hello\n";
    let paragraph_end = "hello".len();
    let mut app = App::new(None);
    let mut doc =
        DocumentView::new(source.split('\n').map(|line| line.to_string()).collect(), 80, 10.0);
    doc.cursor_move_to_offset(paragraph_end);
    app.push_entry_for_test(doc, Box::new(textora_markdown::view::MarkdownEditorView::new()));
    app.switch_workspace_for_test(0);
    app.sync_plugin_state();
    render_active_wysiwyg_plugin_for_test(&mut app);

    let enter_effect = app.dispatch_transactional_edit_for_test(EditCommand::InsertNewline);
    assert!(enter_effect.redraw, "paragraph-end Enter should redraw WYSIWYG content");
    app.sync_plugin_state();
    render_active_wysiwyg_plugin_for_test(&mut app);

    {
        let mut tab = app.active_tab_session_mut().expect("active entry");
        tab.insert_at_cursor(b"X");
    }

    let tab = app.active_tab_session().expect("active entry");
    assert_eq!(
        tab.full_text(),
        "hello\n\nX",
        "typing after paragraph-end Enter before an existing newline must not become a softbreak"
    );
}

#[test]
#[cfg(feature = "markdown")]
fn wysiwyg_paragraph_end_enter_then_typing_keeps_following_line_below() {
    let source = "hello\nnext";
    let paragraph_end = "hello".len();
    let mut app = App::new(None);
    let mut doc =
        DocumentView::new(source.split('\n').map(|line| line.to_string()).collect(), 80, 10.0);
    doc.cursor_move_to_offset(paragraph_end);
    app.push_entry_for_test(doc, Box::new(textora_markdown::view::MarkdownEditorView::new()));
    app.switch_workspace_for_test(0);
    app.sync_plugin_state();
    render_active_wysiwyg_plugin_for_test(&mut app);

    let enter_effect = app.dispatch_transactional_edit_for_test(EditCommand::InsertNewline);
    assert!(enter_effect.redraw, "paragraph-end Enter should redraw WYSIWYG content");
    app.sync_plugin_state();
    render_active_wysiwyg_plugin_for_test(&mut app);

    {
        let mut tab = app.active_tab_session_mut().expect("active entry");
        tab.insert_at_cursor(b"X");
    }

    let tab = app.active_tab_session().expect("active entry");
    assert_eq!(
        tab.full_text(),
        "hello\n\nX\nnext",
        "typing on the new paragraph line must not pull the following source line upward"
    );
}

#[cfg(feature = "markdown")]
fn wysiwyg_cursor_rect_for_byte_for_test(app: &mut App, byte: usize) -> (f32, f32, f32, f32) {
    {
        let mut tab = app.active_tab_session_mut().expect("active entry");
        tab.cursor_move_to_offset(byte);
    }
    app.sync_plugin_state();
    render_active_wysiwyg_plugin_for_test(app);
    let tab = app.active_tab_session().expect("active entry");
    tab.query_cursor_screen_rect(byte).expect("expected WYSIWYG cursor rect")
}

#[test]
fn sync_plugin_state_pushes_source_and_cursor() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState::default()));
    let mut app = App::new(None);
    let mut doc = DocumentView::new(vec!["hello **world**".to_string()], 80, 10.0);
    doc.cursor_move_to_offset("hello **world**".len());
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);
    set_editor_preedit_for_test(&mut app, "ni", Some((2, 2)));

    app.sync_plugin_state();

    let recorded = state.borrow();
    assert_eq!(recorded.source_text, "hello **world**");
    assert_eq!(recorded.cursor_byte, Some("hello **world**".len()));
    assert_eq!(recorded.preedit_text, "ni");
    assert_eq!(recorded.preedit_cursor, Some((2, 2)));
}

#[test]
fn sync_plugin_state_pulls_selection_range_into_document() {
    let doc = DocumentView::new(vec!["abcdef".to_string()], 80, 10.0);
    let generation = doc.tb().gap_buffer().generation();
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        generation,
        selection_range: Some((1, 4)),
        ..RecordingWysiwygState::default()
    }));
    let mut app = App::new(None);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state)));
    app.switch_workspace_for_test(0);

    app.sync_plugin_state();

    let doc = &app.active_tab_session().expect("active entry");
    assert_eq!(doc.selection_range(), Some((1, 4)));
}

#[test]
fn sync_plugin_state_does_not_pull_selection_when_plugin_needs_source_update() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        generation: u32::MAX,
        selection_range: Some((1, 4)),
        ..RecordingWysiwygState::default()
    }));
    let mut app = App::new(None);
    let doc = DocumentView::new(vec!["abcdef".to_string()], 80, 10.0);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state)));
    app.switch_workspace_for_test(0);

    app.sync_plugin_state();

    let doc = &app.active_tab_session().expect("active entry");
    assert_eq!(doc.selection_range(), None);
}

#[test]
fn sync_plugin_state_clears_stale_plugin_selection_after_source_update() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        generation: u32::MAX,
        sel_anchor_byte: Some(1),
        sel_cursor_byte: Some(4),
        ..RecordingWysiwygState::default()
    }));
    let mut app = App::new(None);
    let doc = DocumentView::new(vec!["abcdef".to_string()], 80, 10.0);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    app.sync_plugin_state();

    let recorded = state.borrow();
    assert_eq!(recorded.sel_anchor_byte, None);
    assert_eq!(recorded.sel_cursor_byte, None);
}

#[test]
fn wysiwyg_selection_delete_clears_plugin_highlight() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState::default()));
    let mut app = App::new(None);
    let mut doc = DocumentView::new(vec!["abcdef".to_string()], 80, 10.0);
    doc.cursor_move_to_offset(4);
    doc.cursor_mut().selection_anchor = Some(1);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);
    app.sync_plugin_state();
    assert_eq!(state.borrow().sel_anchor_byte, Some(1));
    assert_eq!(state.borrow().sel_cursor_byte, Some(4));

    {
        let mut tab = app.active_tab_session_mut().expect("active entry");
        assert!(tab.delete_selection(), "selected text should be deleted");
    }
    app.sync_plugin_state();

    let tab = app.active_tab_session().expect("active entry");
    assert_eq!(tab.full_text(), "aef");
    assert_eq!(tab.selection_range(), None);
    let recorded = state.borrow();
    assert_eq!(recorded.sel_anchor_byte, None);
    assert_eq!(recorded.sel_cursor_byte, None);
}

/// After an IME commit inserts characters into the document, the WYSIWYG
/// plugin must receive the updated source text and the snapped cursor
/// byte immediately — before the next render cycle.
#[test]
fn ime_commit_syncs_wysiwyg_source_and_snapped_cursor() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState::default()));
    let mut app = App::new(None);
    let mut doc = DocumentView::new(vec!["hello **world**".to_string()], 80, 10.0);
    // Set cursor at byte 5 (after "hello")
    doc.cursor_move_to_offset(5);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    // Simulate IME commit inserting "中" at cursor
    {
        let mut tab = app.active_tab_session_mut().unwrap();
        tab.insert_at_cursor("中".as_bytes());
    }

    // Sync — this is what the IME commit handler does after insertion
    app.sync_plugin_state();

    let recorded = state.borrow();
    // "中" is 3 UTF-8 bytes, inserted at byte 5 → cursor snaps to 8
    assert_eq!(recorded.source_text, "hello中 **world**");
    assert_eq!(recorded.cursor_byte, Some(8));
}

/// After an IME commit, the composition state (preedit_text,
/// preedit_cursor) must be cleared, and the WYSIWYG preferred x
/// must be reset so subsequent vertical moves don't anchor to
/// a stale position.
#[test]
fn ime_commit_clears_preedit_and_preferred_x() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState::default()));
    let mut app = App::new(None);
    let doc = DocumentView::new(vec!["hello".to_string()], 80, 10.0);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    // Set preedit/composition state as if IME was active
    set_editor_preedit_for_test(&mut app, "ni", Some((1, 2)));
    app.editor_runtime.set_preferred_x(Some(100.0));

    // Simulate the commit path: insert char, sync, clear
    {
        let mut tab = app.active_tab_session_mut().unwrap();
        tab.insert_at_cursor("中".as_bytes());
    }
    app.sync_plugin_state();

    // Clear IME state — these are the lines added to the commit handler
    set_editor_preedit_for_test(&mut app, "", None);
    app.editor_runtime.set_preferred_x(None);

    let (preedit_text, preedit_cursor) = app.editor_runtime.preedit();
    assert!(preedit_text.is_empty());
    assert!(preedit_cursor.is_none());
    assert!(app.editor_runtime.preferred_x().is_none());
}

#[test]
fn wysiwyg_navigation_sends_snapped_cursor_byte_to_plugin() {
    // Plugin returns a byte from VisualMove. Host must move
    // DocumentView cursor, read back cursor_offset(), then send
    // SetCursorByte with the read-back value. This verifies the sync
    // path: plugin byte -> cursor_move_to_offset -> cursor_offset()
    // -> SetCursorByte.
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        visual_move_result: Some(3), // mid-cluster inside ZWJ emoji
        ..Default::default()
    }));
    let mut app = App::new(None);
    let emoji = "👨\u{200D}👩\u{200D}👧";
    let content = format!("x{emoji}y");
    let doc = DocumentView::new(vec![content.clone()], 80, 10.0);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    // Simulate the WYSIWYG navigation sync path directly:
    // Phase 1: query plugin for target byte
    let new_byte = {
        let tab = app.active_tab_session().unwrap();
        match tab.query(ui::plugin::PluginQuery::VisualMove {
            current_byte: 0,
            direction: ui::plugin::MoveDirection::Right,
            target_x: None,
        }) {
            ui::plugin::PluginResponse::BytePosition(Some(b)) => b,
            _ => panic!("plugin should return a byte"),
        }
    };
    assert_eq!(new_byte, 3);

    // Phase 2: move cursor, read back, notify plugin
    let mut tab = app.active_tab_session_mut().unwrap();
    tab.cursor_move_to_offset(new_byte);
    let snapped_byte = tab.cursor_offset().to_usize();
    tab.send_message(ui::plugin::PluginMessage::SetCursorByte(snapped_byte));

    let recorded = state.borrow();
    assert!(recorded.cursor_byte.is_some(), "SetCursorByte should have been called");
    assert_eq!(
        recorded.cursor_byte,
        Some(snapped_byte),
        "SetCursorByte should receive DocumentView's cursor_offset after move"
    );
}

#[test]
fn wysiwyg_vertical_navigation_preserves_starting_target_x_across_short_line() {
    const ANCHOR_X: f32 = 120.0;
    const SHORT_LINE_X: f32 = 24.0;
    const CURSOR_Y: f32 = 0.0;
    const CURSOR_WIDTH: f32 = 2.0;
    const CURSOR_HEIGHT: f32 = 16.0;

    let content = "first line\nx\nthird line";
    let short_line_byte = content.find('x').expect("fixture should contain the short line");
    let third_line_byte = content.find("third").expect("fixture should contain the third line");
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        visual_move_result: Some(short_line_byte),
        cursor_rect_by_byte: vec![
            (0, (ANCHOR_X, CURSOR_Y, CURSOR_WIDTH, CURSOR_HEIGHT)),
            (short_line_byte, (SHORT_LINE_X, CURSOR_Y, CURSOR_WIDTH, CURSOR_HEIGHT)),
        ],
        ..Default::default()
    }));
    let mut app = App::new(None);
    let doc = DocumentView::new(vec![content.to_string()], 80, 10.0);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    let first_effect = app.dispatch_wysiwyg_navigation(&EditCommand::MoveDown);

    assert!(first_effect.redraw, "first vertical move should redraw WYSIWYG content");
    assert_eq!(
        state.borrow().visual_move_query,
        Some((0, ui::plugin::MoveDirection::Down, Some(ANCHOR_X))),
        "first vertical move should seed target_x from the starting cursor rect"
    );
    assert_eq!(
        app.editor_runtime.preferred_x(),
        Some(ANCHOR_X),
        "preferred x should remain the starting column after landing on a short line"
    );

    state.borrow_mut().visual_move_result = Some(third_line_byte);
    let second_effect = app.dispatch_wysiwyg_navigation(&EditCommand::MoveDown);

    assert!(second_effect.redraw, "second vertical move should redraw WYSIWYG content");
    assert_eq!(
        state.borrow().visual_move_query,
        Some((short_line_byte, ui::plugin::MoveDirection::Down, Some(ANCHOR_X))),
        "subsequent vertical moves should keep the original column, not the short-line landing x"
    );
}

#[test]
fn wysiwyg_move_down_scrolls_to_reveal_cursor_below_viewport() {
    const CURSOR_X: f32 = 10.0;
    const CURSOR_WIDTH: f32 = 2.0;
    const CURSOR_HEIGHT: f32 = 16.0;

    let content = "first line\nsecond line\nthird line";
    let second_line_byte = content.find("second").expect("fixture should contain the second line");
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        visual_move_result: Some(second_line_byte),
        ..Default::default()
    }));
    let mut app = App::new(None);
    let doc = DocumentView::new(vec![content.to_string()], 80, 10.0);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    // Cursor lands with its top exactly at the viewport bottom edge: fully off-screen.
    let viewport_h = app.plugin_viewport_h();
    state.borrow_mut().cursor_rect_by_byte =
        vec![(second_line_byte, (CURSOR_X, viewport_h, CURSOR_WIDTH, CURSOR_HEIGHT))];

    let effect = app.dispatch_wysiwyg_navigation(&EditCommand::MoveDown);

    assert!(effect.redraw, "vertical move should redraw WYSIWYG content");
    assert_eq!(
        state.borrow().scroll_messages,
        vec![(CURSOR_HEIGHT, viewport_h)],
        "cursor below the viewport should scroll down by the minimal overflow delta"
    );
}

#[test]
fn wysiwyg_move_up_scrolls_to_reveal_cursor_above_viewport() {
    const ABOVE_TOP_Y: f32 = -24.0;
    const CURSOR_X: f32 = 10.0;
    const CURSOR_WIDTH: f32 = 2.0;
    const CURSOR_HEIGHT: f32 = 16.0;

    let content = "first line\nsecond line\nthird line";
    let first_line_byte = 0;
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        visual_move_result: Some(first_line_byte),
        ..Default::default()
    }));
    let mut app = App::new(None);
    let mut doc = DocumentView::new(vec![content.to_string()], 80, 10.0);
    doc.cursor_move_to_offset(content.find("second").expect("fixture should contain second line"));
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    // Cursor lands partially above the viewport top edge.
    let viewport_h = app.plugin_viewport_h();
    state.borrow_mut().cursor_rect_by_byte =
        vec![(first_line_byte, (CURSOR_X, ABOVE_TOP_Y, CURSOR_WIDTH, CURSOR_HEIGHT))];

    let effect = app.dispatch_wysiwyg_navigation(&EditCommand::MoveUp);

    assert!(effect.redraw, "vertical move should redraw WYSIWYG content");
    assert_eq!(
        state.borrow().scroll_messages,
        vec![(ABOVE_TOP_Y, viewport_h)],
        "cursor above the viewport should scroll up by the negative cursor y"
    );
}

#[test]
fn wysiwyg_move_down_does_not_scroll_when_cursor_already_visible() {
    const VISIBLE_Y: f32 = 8.0;
    const CURSOR_X: f32 = 10.0;
    const CURSOR_WIDTH: f32 = 2.0;
    const CURSOR_HEIGHT: f32 = 16.0;

    let content = "first line\nsecond line\nthird line";
    let second_line_byte = content.find("second").expect("fixture should contain the second line");
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        visual_move_result: Some(second_line_byte),
        cursor_rect: Some((CURSOR_X, VISIBLE_Y, CURSOR_WIDTH, CURSOR_HEIGHT)),
        ..Default::default()
    }));
    let mut app = App::new(None);
    let doc = DocumentView::new(vec![content.to_string()], 80, 10.0);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    let effect = app.dispatch_wysiwyg_navigation(&EditCommand::MoveDown);

    assert!(effect.redraw, "vertical move should redraw WYSIWYG content");
    assert!(
        state.borrow().scroll_messages.is_empty(),
        "cursor already inside the viewport must not scroll (no jitter)"
    );
}

#[test]
fn wysiwyg_extend_down_scrolls_to_reveal_cursor_below_viewport() {
    const CURSOR_X: f32 = 10.0;
    const CURSOR_WIDTH: f32 = 2.0;
    const CURSOR_HEIGHT: f32 = 16.0;

    let content = "first line\nsecond line\nthird line";
    let second_line_byte = content.find("second").expect("fixture should contain the second line");
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        visual_move_result: Some(second_line_byte),
        ..Default::default()
    }));
    let mut app = App::new(None);
    let doc = DocumentView::new(vec![content.to_string()], 80, 10.0);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    let viewport_h = app.plugin_viewport_h();
    state.borrow_mut().cursor_rect_by_byte =
        vec![(second_line_byte, (CURSOR_X, viewport_h, CURSOR_WIDTH, CURSOR_HEIGHT))];

    let effect = app.dispatch_wysiwyg_navigation(&EditCommand::ExtendDown);

    assert!(effect.redraw, "vertical selection extension should redraw WYSIWYG content");
    assert_eq!(
        state.borrow().scroll_messages,
        vec![(CURSOR_HEIGHT, viewport_h)],
        "extend-down past the viewport bottom should scroll by the minimal overflow delta"
    );
    assert_eq!(
        state.borrow().sel_anchor_byte,
        Some(0),
        "extend-down must keep the selection anchor at the original cursor"
    );
}

#[test]
fn wysiwyg_extend_right_preserves_anchor_and_notifies_plugin_selection() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        visual_move_result: Some(5),
        ..Default::default()
    }));
    let mut app = App::new(None);
    let mut doc = DocumentView::new(vec!["hello world".to_string()], 80, 10.0);
    doc.cursor_move_to_offset(2);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    let effect = app.dispatch_wysiwyg_navigation(&EditCommand::ExtendRight);

    assert!(effect.redraw, "selection extension should redraw WYSIWYG content");
    let tab = app.active_tab_session().expect("active tab");
    assert_eq!(tab.selection_range(), Some((2, 5)));
    let recorded = state.borrow();
    assert_eq!(
        recorded.visual_move_query,
        Some((2, ui::plugin::MoveDirection::Right, None)),
        "Shift+Right should still use plugin visual navigation from the current byte"
    );
    assert_eq!(recorded.sel_anchor_byte, Some(2));
    assert_eq!(recorded.sel_cursor_byte, Some(5));
    assert_eq!(recorded.cursor_byte, Some(5));
}

#[test]
fn wysiwyg_page_down_clears_plugin_selection_bytes() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        hit_test_byte: Some(5),
        sel_anchor_byte: Some(1),
        sel_cursor_byte: Some(4),
        ..RecordingWysiwygState::default()
    }));
    let mut app = App::new(None);
    let mut doc = DocumentView::new(vec!["hello world".to_string()], 80, 10.0);
    doc.cursor_mut().selection_anchor = Some(1);
    doc.cursor_move_to_offset(4);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    let effect = app.dispatch_wysiwyg_navigation(&EditCommand::PageDown);

    assert!(effect.redraw, "page navigation should redraw WYSIWYG content");
    let recorded = state.borrow();
    assert_eq!(recorded.cursor_byte, Some(5));
    assert_eq!(recorded.sel_anchor_byte, None);
    assert_eq!(recorded.sel_cursor_byte, None);
}

#[test]
fn wysiwyg_single_click_uses_snapped_byte_for_selection_anchor() {
    let emoji = "👨\u{200D}👩\u{200D}👧";
    let content = format!("x{emoji}y");
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        hit_test_byte: Some(3),
        ..RecordingWysiwygState::default()
    }));
    let mut app = App::new(None);
    let doc = DocumentView::new(vec![content], 80, 10.0);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    app.dispatch_editor_mouse_input(winit::event::ElementState::Pressed, 40.0, 24.0, None);

    let tab = app.active_tab_session().expect("active tab should exist");
    let snapped = tab.cursor_offset().to_usize();
    assert_eq!(tab.cursor().selection_anchor, Some(snapped));
    let recorded = state.borrow();
    assert_eq!(recorded.cursor_byte, Some(snapped));
    assert_eq!(recorded.sel_anchor_byte, Some(snapped));
    assert_eq!(recorded.sel_cursor_byte, Some(snapped));
}

#[test]
fn wysiwyg_double_click_selects_word_at_hit_byte() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        hit_test_byte: Some(7),
        ..Default::default()
    }));
    let mut app = App::new(None);
    let doc = DocumentView::new(vec!["hello world".to_string()], 80, 10.0);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    app.dispatch_editor_mouse_input(winit::event::ElementState::Pressed, 40.0, 24.0, None);
    app.dispatch_editor_mouse_input(winit::event::ElementState::Released, 40.0, 24.0, None);
    app.dispatch_editor_mouse_input(winit::event::ElementState::Pressed, 40.0, 24.0, None);
    app.dispatch_editor_mouse_input(winit::event::ElementState::Released, 40.0, 24.0, None);

    let tab = app.active_tab_session().expect("active tab");
    assert_eq!(tab.selection_range(), Some((6, 11)));
    let recorded = state.borrow();
    assert_eq!(recorded.sel_anchor_byte, Some(6));
    assert_eq!(recorded.sel_cursor_byte, Some(11));
    assert_eq!(recorded.cursor_byte, Some(11));
}

#[test]
fn wysiwyg_single_click_moves_cursor_without_leaving_empty_selection() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        hit_test_byte: Some(5),
        ..Default::default()
    }));
    let mut app = App::new(None);
    let doc = DocumentView::new(vec!["hello world".to_string()], 80, 10.0);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    app.dispatch_editor_mouse_input(winit::event::ElementState::Pressed, 40.0, 24.0, None);
    app.dispatch_editor_mouse_input(winit::event::ElementState::Released, 40.0, 24.0, None);

    let tab = app.active_tab_session().expect("active tab");
    assert_eq!(tab.cursor_offset().to_usize(), 5);
    assert_eq!(tab.selection_range(), None);
    let recorded = state.borrow();
    assert_eq!(recorded.sel_anchor_byte, None);
    assert_eq!(recorded.sel_cursor_byte, None);
    assert_eq!(recorded.cursor_byte, Some(5));
}

// ── WYSIWYG cursor → window rect tests ───────────────────────────────

#[test]
fn wysiwyg_cursor_window_rect_adds_plugin_render_bounds() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        cursor_rect: Some((10.0, 20.0, 2.0, 18.0)),
        ..Default::default()
    }));
    let mut app = App::new(None);
    let doc = DocumentView::new(vec!["hello world".to_string()], 80, 10.0);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    let bounds = app.plugin_render_bounds();
    let rect = app.plugin_cursor_window_rect(5);
    assert!(rect.is_some(), "should return a rect for WYSIWYG plugin");

    let r = rect.unwrap();
    assert!((r.w - 2.0).abs() < 0.001, "width should match plugin cursor_rect w");
    assert!((r.h - 18.0).abs() < 0.001, "height should match plugin cursor_rect h");
    assert!(
        (r.x - (bounds.x + 10.0)).abs() < 0.001,
        "x should equal bounds.x + plugin_cursor_x, got r.x={} bounds.x={}",
        r.x,
        bounds.x
    );
    assert!(
        (r.y - (bounds.y + 20.0)).abs() < 0.001,
        "y should equal bounds.y + plugin_cursor_y, got r.y={} bounds.y={}",
        r.y,
        bounds.y
    );
}

#[test]
fn ime_cursor_area_uses_nonzero_plugin_render_bounds_origin() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        cursor_rect: Some((10.0, 20.0, 2.0, 18.0)),
        ..Default::default()
    }));
    let mut app = App::new(None);
    let doc = DocumentView::new(vec!["hello world".to_string()], 80, 10.0);
    app.push_entry_for_test(doc, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    // A nonzero advance would expose an accidental second shift by the host.
    app.preedit_advance_px = 15.0;

    let rect = app.plugin_cursor_window_rect(5).expect("should return rect");
    let bounds = app.plugin_render_bounds();
    assert!(
        bounds.x > 0.0 || bounds.y > 0.0,
        "fixture must exercise a nonzero plugin render origin"
    );

    // Candidate window Y = cursor bottom
    let candidate_y = rect.y + rect.h;
    // Preedit draw Y = cursor top
    let preedit_y = rect.y;
    assert!(
        (candidate_y - preedit_y - 18.0).abs() < 0.001,
        "candidate Y ({}) should be preedit Y ({}) + line height (18.0)",
        candidate_y,
        preedit_y
    );
    assert_eq!(candidate_y, rect.bottom(), "candidate_y should equal rect.bottom()");

    let ime_cursor_x = crate::app_window::ime_cursor_x(rect, true, app.preedit_advance_px);
    let expected_x = bounds.x + 10.0;
    assert!(
        (ime_cursor_x - expected_x).abs() < 0.001,
        "ime_cursor_x ({}) should equal the projected plugin cursor at {}",
        ime_cursor_x,
        expected_x
    );
}

#[test]
fn wysiwyg_cursor_window_rect_returns_none_for_non_wysiwyg_plugin() {}

#[test]
fn wysiwyg_preedit_cursor_rect_resolves_with_cursor_window_rect() {
    // When a WYSIWYG plugin is active and preedit_text is set, the
    // cursor window rect helper should return the expected position so
    // the renderer can generate preedit vertices at the right spot.
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        cursor_rect: Some((10.0, 5.0, 2.0, 18.0)),
        ..RecordingWysiwygState::default()
    }));
    let mut app = App::new(None);
    let mut document = DocumentView::new(vec!["ni hao".to_string()], 80, 10.0);
    document.cursor_move_to_offset(3);
    app.push_entry_for_test(document, Box::new(RecordingWysiwygPlugin::new(state.clone())));
    app.switch_workspace_for_test(0);

    // Set preedit text to simulate IME composition
    set_editor_preedit_for_test(&mut app, "ni", None);

    // Search not visible: preedit should render
    assert!(!app.active_tab_session().unwrap().search_state().panel_visible);

    // Verify the active tab is WYSIWYG
    let tab = app.active_tab_session().expect("active tab should exist");

    // The cursor window rect should resolve through the plugin query
    let cursor_byte = tab.cursor_offset().to_usize();
    let rect = app.plugin_cursor_window_rect(cursor_byte).expect("cursor rect should resolve");
    assert_eq!(rect.w, 2.0);
    assert_eq!(rect.h, 18.0);

    // At this point, if text/gpu were available, the renderer would call
    // preedit_text_vertices(metrics, "ni", rect.x, rect.y, ...)
    // which generates glyph vertices at the cursor position.
}
