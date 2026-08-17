//! Mouse input handling and hit-testing.
//!
//! Extracts mouse-related state and logic from App.

use std::time::Instant;
use winit::event::ElementState;
use winit::keyboard::ModifiersState;

use crate::cursor_motion::CursorRenderState;
use crate::document_view::DocumentView;
use crate::edit_transaction::DocumentModelMut;
use crate::line_index::LineIndex;
use core::types::UniCharOffset;
use ui::render_geom::AdvanceCacheEntry;
use ui::settings::UiMetrics;

pub(crate) use appkit_shell::mouse_state::{CanvasDragEligibility, CanvasDragSession, MouseState};

/// Convert mouse click position (physical pixels) to (document byte offset, doc_line, vis_line).
/// vis_line is the advance_cache index — preserved so cursor rendering
/// can avoid ambiguous VL boundary disambiguation.
/// Uses the advance cache populated during `shape_visible_lines()`.
/// Accounts for sub-line pixel offset from scroll_y fractional part.
pub(crate) fn hit_test(
    px: f32,
    py: f32,
    dv: &DocumentView,
    advance_cache: &[AdvanceCacheEntry],
    metrics: &UiMetrics,
    left_margin: f32,
    tab_bar_height: f32,
    line_index: &LineIndex,
) -> Option<(UniCharOffset, usize, usize)> {
    let sub_line_offset = dv.sub_line_pixel_offset(metrics.line_height);
    hit_test_with_sub_line_offset(
        px,
        py,
        advance_cache,
        metrics,
        left_margin,
        tab_bar_height,
        sub_line_offset,
        line_index,
    )
}

pub(crate) fn hit_test_with_sub_line_offset(
    px: f32,
    py: f32,
    advance_cache: &[AdvanceCacheEntry],
    metrics: &UiMetrics,
    left_margin: f32,
    tab_bar_height: f32,
    sub_line_offset: f32,
    line_index: &LineIndex,
) -> Option<(UniCharOffset, usize, usize)> {
    let adjusted_py = py - tab_bar_height - sub_line_offset;
    if adjusted_py < 0.0 {
        return None;
    }
    let vis_line = (adjusted_py / metrics.line_height) as usize;
    if vis_line >= advance_cache.len() {
        return None;
    }
    let entry = &advance_cache[vis_line];
    let doc_line = entry.doc_line;
    let vl_grapheme_start = entry.vl_grapheme_start;
    let clusters = &entry.clusters;

    // 簇是字符边界单位（可能含 multi-byte 字符）。按像素位置 snap 到簇起点
    // 或终点，**不在簇内插字节偏移**——否则 multi-byte 字符簇内会得到非字符
    // 边界 offset，set_cursor_offset_synced 调用 tb.cursor_move_to_offset 后位置
    // 被规整成合法字符边界，与请求 offset 不一致，触发 debug_assert panic
    // (cursor desync: tb.cursor_offset() != requested offset)。
    let mut unichar_in_vl = vl_grapheme_start;
    let mut prev_x = left_margin;
    let mut prev_unichar = vl_grapheme_start;
    for &(cluster_end, cluster_x, grapheme_idx) in clusters {
        let _ = cluster_end; // still needed for pixel lookup
        if px <= cluster_x {
            let pixel_width = cluster_x - prev_x;
            let fraction =
                if pixel_width > 0.0 { ((px - prev_x) / pixel_width).clamp(0.0, 1.0) } else { 0.0 };
            unichar_in_vl = if fraction >= 0.5 {
                vl_grapheme_start + grapheme_idx as usize + 1
            } else {
                prev_unichar
            };
            break;
        }
        prev_x = cluster_x;
        prev_unichar = vl_grapheme_start + grapheme_idx as usize + 1;
        unichar_in_vl = vl_grapheme_start + grapheme_idx as usize + 1;
    }

    // Convert line-local unichar offset to document-level UniCharOffset
    let doc_unichar = line_index.unichar_of_line(doc_line) + unichar_in_vl;
    Some((doc_unichar, doc_line, vis_line))
}

/// Handle CursorMoved while mouse button is held.
/// Returns true if redraw is needed.
///
/// For double-click (click_count == 2) and triple-click (click_count == 3),
/// uses word-level and line-level granularity respectively, matching standard
/// editor behavior (VS Code, Sublime Text, etc.).
pub(crate) fn handle_cursor_moved(
    _px: f32,
    _py: f32,
    mouse: &mut MouseState,
    dv: &mut impl DocumentModelMut,
    hit: Option<(UniCharOffset, usize, usize)>,
) -> bool {
    let dv = dv.document_model_mut();
    if !mouse.captures_pointer() {
        return false;
    }
    if let Some((offset, doc_line, _vis_line)) = hit {
        if mouse.click_count >= 2 {
            // Double/triple-click drag: word or line granularity.
            let is_first_drag = mouse.down_byte_offset.is_some();

            if mouse.click_count >= 3 {
                // ── Triple-click: line granularity ──
                let line_start = dv.line_byte_offset(doc_line).unwrap_or(0);
                let line_end = if doc_line + 1 < dv.line_count() {
                    dv.line_byte_offset(doc_line + 1).unwrap_or(dv.buffer_len())
                } else {
                    dv.buffer_len()
                };
                let byte_offset = dv.unichar_to_byte_offset(offset);
                // TODO(Phase 5): migrate to unichar when word_select_at/line APIs migrate

                if is_first_drag {
                    let initial_anchor_byte = mouse.down_byte_offset.take().unwrap();
                    let initial_cursor = dv.cursor().offset;
                    if byte_offset >= initial_anchor_byte {
                        dv.cursor_mut().selection_anchor = Some(initial_anchor_byte);
                    } else {
                        dv.cursor_mut().selection_anchor = Some(initial_cursor.to_usize());
                    }
                }

                let anchor = dv.cursor().selection_anchor.unwrap_or(line_start);
                if byte_offset >= anchor {
                    dv.set_cursor_offset_synced(line_end);
                } else {
                    dv.set_cursor_offset_synced(line_start);
                }
            } else {
                // ── Double-click: word granularity ──
                let byte_offset = dv.unichar_to_byte_offset(offset);
                // TODO(Phase 5): migrate to unichar when word_select_at/line APIs migrate
                let (ws, we) = dv.word_select_at(byte_offset);

                if is_first_drag {
                    let initial_anchor_byte = mouse.down_byte_offset.take().unwrap();
                    let initial_cursor = dv.cursor().offset;
                    if byte_offset >= initial_anchor_byte {
                        dv.cursor_mut().selection_anchor = Some(initial_anchor_byte);
                    } else {
                        dv.cursor_mut().selection_anchor = Some(initial_cursor.to_usize());
                    }
                }

                let anchor = dv.cursor().selection_anchor.unwrap_or(ws);
                if byte_offset >= anchor {
                    dv.set_cursor_offset_synced(we);
                } else {
                    dv.set_cursor_offset_synced(ws);
                }
            }
        } else {
            // ── Single-click drag: character granularity ──
            if let Some(anchor_byte) = mouse.down_byte_offset.take() {
                dv.cursor_mut().selection_anchor = Some(anchor_byte);
            }
            dv.set_cursor_unichar_synced_on_line(offset, doc_line);
        }
        return true;
    }
    false
}

/// Handle MouseInput (left button press/release).
/// Returns true if redraw is needed.
pub(crate) fn handle_mouse_input(
    state: ElementState,
    px: f32,
    py: f32,
    mouse: &mut MouseState,
    dv: &mut DocumentView,
    modifiers: ModifiersState,
    hit: Option<(UniCharOffset, usize, usize)>,
) -> bool {
    let mut presentation = dv.take_presentation();
    let handled = handle_mouse_input_with_cursor_state(
        state,
        px,
        py,
        mouse,
        dv,
        &mut presentation.cursor_render_state,
        modifiers,
        hit,
    );
    dv.restore_presentation(presentation);
    handled
}

pub(crate) fn handle_mouse_input_with_cursor_state(
    state: ElementState,
    px: f32,
    py: f32,
    mouse: &mut MouseState,
    dv: &mut impl DocumentModelMut,
    cursor_render_state: &mut CursorRenderState,
    modifiers: ModifiersState,
    hit: Option<(UniCharOffset, usize, usize)>,
) -> bool {
    let dv = dv.document_model_mut();
    if state.is_pressed() {
        mouse.is_down = true;

        // A pointer press is user-driven caret movement: split any ongoing
        // undo coalescing run even if the click lands back on the byte where
        // the last edit ended.
        dv.break_edit_merge();

        // Double/triple click detection (within 500ms and 5 pixels distance)
        let now = Instant::now();
        let elapsed = now.duration_since(mouse.last_click_time);
        let dx = px - mouse.last_click_pos.0;
        let dy = py - mouse.last_click_pos.1;
        let dist_sq = dx * dx + dy * dy;

        if elapsed.as_millis() > 500 || dist_sq > 25.0 {
            mouse.click_count = 0;
        }
        mouse.click_count = (mouse.click_count + 1).min(3);
        mouse.last_click_time = now;
        mouse.last_click_pos = (px, py);

        if let Some((offset, doc_line, vis_line)) = hit {
            let shift = modifiers.shift_key();

            if mouse.click_count == 3 {
                // Triple-click: select line
                let line_start = dv.line_byte_offset(doc_line).unwrap_or(0);
                let line_end = if doc_line + 1 < dv.line_count() {
                    dv.line_byte_offset(doc_line + 1).unwrap_or(dv.buffer_len())
                } else {
                    dv.buffer_len()
                };
                dv.cursor_mut().selection_anchor = Some(line_start);
                dv.set_cursor_offset_synced(line_end);
                mouse.down_byte_offset = Some(line_start);
            } else if mouse.click_count == 2 {
                // Double-click: word select
                let byte_offset = dv.unichar_to_byte_offset(offset);
                let (start, end) = dv.word_select_at(byte_offset);
                dv.cursor_mut().selection_anchor = Some(start);
                dv.set_cursor_offset_synced(end);
                mouse.down_byte_offset = Some(start);
            } else if shift {
                // Shift+click: extend selection
                if dv.cursor().selection_anchor.is_none() {
                    dv.cursor_mut().selection_anchor = Some(dv.cursor().offset.to_usize());
                }
                dv.set_cursor_unichar_synced_on_line(offset, doc_line);
                cursor_render_state.click_hint = Some((offset, vis_line));
            } else {
                // Single click: set cursor, clear any existing selection
                // Record down_byte_offset for potential drag-to-select
                dv.cursor_mut().selection_anchor = None;
                dv.cursor_move_to_unichar_on_line(offset, doc_line);
                mouse.down_byte_offset = Some(dv.cursor().offset.to_usize());
                cursor_render_state.click_hint = Some((offset, vis_line));
            }
            cursor_render_state.sticky_x = px;
            cursor_render_state.cursor_blink_instant = Instant::now();
            return true;
        } else {
            // 点击空白区域（hit 为 None，即 advance_cache 之外的空白）：
            // 取消选区并将光标移至文档末尾，同时隐藏选区高亮。
            dv.cursor_mut().selection_anchor = None;
            dv.cursor_move_to_offset(dv.buffer_len()); // byte offset OK for EOF
            mouse.down_byte_offset = None;
            cursor_render_state.sticky_x = px;
            cursor_render_state.cursor_blink_instant = Instant::now();
            return true;
        }
    } else {
        mouse.is_down = false;
        mouse.down_byte_offset = None;
        if dv.cursor().selection_anchor == Some(dv.cursor().offset.to_usize()) {
            dv.cursor_mut().selection_anchor = None;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document_view::DocumentView;
    use core::types::ByteIndex;
    use ui::settings::Settings;

    fn make_dv(content: &str) -> DocumentView {
        DocumentView::new(content.lines().map(|s| s.to_string()).collect(), 10, 10.0)
    }

    fn type_default_text(dv: &mut DocumentView, text: &str) {
        let intent = ui::plugin::EditIntent::InsertText(text.to_owned());
        let request = crate::edit_transaction::build_edit_request(&dv.model, intent);
        let plan = crate::edit_transaction::default_edit_plan(&request, &dv.model);
        crate::edit_transaction::execute_edit_plan(plan, &mut dv.model)
            .expect("typing must execute in the mouse coalescing test");
    }

    fn click(dv: &mut DocumentView, mouse: &mut MouseState, px: f32, offset: UniCharOffset) {
        let modifiers = winit::keyboard::ModifiersState::empty();
        for state in [winit::event::ElementState::Pressed, winit::event::ElementState::Released] {
            handle_mouse_input(state, px, 10.0, mouse, dv, modifiers, Some((offset, 0, 0)));
        }
    }

    #[test]
    fn mouse_click_away_and_back_splits_typing_undo_run() {
        let mut dv = make_dv("");
        let mut mouse = MouseState::new();

        type_default_text(&mut dv, "a");
        assert_eq!(dv.full_text(), "a");

        // The caret leaves (click at line start) and returns to the exact byte
        // where typing ended (click at line end). A user click must split the
        // undo run even though the caret is back on the last edit's byte.
        click(&mut dv, &mut mouse, 200.0, UniCharOffset(0));
        click(&mut dv, &mut mouse, 10.0, UniCharOffset(1));
        type_default_text(&mut dv, "b");

        assert_eq!(dv.full_text(), "ab");
        dv.undo();
        assert_eq!(
            dv.full_text(),
            "a",
            "first undo must only remove the text typed after clicking"
        );
        dv.undo();
        assert_eq!(dv.full_text(), "");
    }

    #[test]
    fn typing_without_mouse_clicks_coalesces_into_one_undo_entry() {
        let mut dv = make_dv("");

        type_default_text(&mut dv, "a");
        type_default_text(&mut dv, "b");

        assert_eq!(dv.full_text(), "ab");
        dv.undo();
        assert_eq!(dv.full_text(), "", "one undo must revert the whole typing run");
    }

    #[test]
    fn hit_test_snaps_to_char_boundary_inside_multibyte_cluster() {
        // 用户报告的 panic 复现：multi-byte 字符簇内点击产生非字符边界 offset。
        // hit_test 必须 snap 到簇起点或终点（cluster_end / prev_end），不能
        // 返回 prev_end+1 或 prev_end+2 这种字符内字节偏移。
        // 模拟一个汉字簇：3 字节宽，从 left_margin=100 到 cluster_x=130（30px）。
        let entry = ui::render_geom::AdvanceCacheEntry {
            doc_line: 0,
            vl_byte_start: 0,
            vl_grapheme_start: 0,
            clusters: vec![(3usize, 130.0f32, 0)], // 簇 byte_range=[0,3], 像素 [100,130]
        };
        let cache = vec![entry];

        let mut dv = make_dv("汉字"); // 汉=3 字节
        let settings = Settings::new();
        let metrics = UiMetrics::from_settings(&settings, 1.0);
        let lh = metrics.line_height;

        // py=0 → vis_line=0；这里 dv 的 line_byte_offset(0) = 0
        // px=100 (簇起点): fraction=0 → snap to prev_end=0
        let r =
            hit_test(100.0, lh * 0.1, &dv, &cache, &metrics, 100.0, 0.0, &dv.line_index).unwrap();
        assert_eq!(r.0, UniCharOffset(0), "簇起点应 snap 到 prev_end=0");
        // px=110 (1/3 位置): fraction≈0.33 < 0.5 → snap to prev_end=0
        let r =
            hit_test(110.0, lh * 0.1, &dv, &cache, &metrics, 100.0, 0.0, &dv.line_index).unwrap();
        assert_eq!(r.0, UniCharOffset(0), "簇内左半段应 snap 到 prev_end");
        // px=120 (2/3 位置): fraction≈0.67 >= 0.5 → snap 到第二字符起点
        let r =
            hit_test(120.0, lh * 0.1, &dv, &cache, &metrics, 100.0, 0.0, &dv.line_index).unwrap();
        assert_eq!(r.0, UniCharOffset(1), "簇内右半段应 snap 到第二字符起点");
        // 关键：UniCharOffset 是字符级偏移，不会产生 mid-character 值
        dv.set_cursor_offset_synced(dv.unichar_to_byte_offset(r.0)); // 不应 panic
    }

    #[test]
    fn hit_test_second_vl_returns_line_local_offset() {
        // clusters 改为 vl-local 后，非首 VL 的 hit_test 应正确返回
        // line-local 偏移（vl_byte_start + cluster_end），而非裸 vl-local 值。
        // 场景：一行 10 字节软折行为两 VL：
        //   VL0: vl_byte_start=0, bytes [0, 5)
        //   VL1: vl_byte_start=5, bytes [5, 10)
        // VL1 的 clusters: [(5, 130.0, 0)] —— cluster_end=5 是 vl-local (10-5)。
        let entry_vl1 = ui::render_geom::AdvanceCacheEntry {
            doc_line: 0,
            vl_byte_start: 5,
            vl_grapheme_start: 5,
            clusters: vec![(5usize, 130.0f32, 4)],
        };
        let cache = vec![
            ui::render_geom::AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 0,
                vl_grapheme_start: 0,
                clusters: vec![(5usize, 130.0f32, 4)],
            },
            entry_vl1,
        ];

        let dv = make_dv("abcdefghij");
        let settings = Settings::new();
        let metrics = UiMetrics::from_settings(&settings, 1.0);
        let lh = metrics.line_height;

        // vis_line=1（第二 VL），点击簇右半段 → fraction≈0.67 >= 0.5
        let r =
            hit_test(120.0, lh * 1.5, &dv, &cache, &metrics, 100.0, 0.0, &dv.line_index).unwrap();
        assert_eq!(
            r.0,
            UniCharOffset(10),
            "第二 VL 右半段应返回 line-local offset=10 (0+5+5)，非 vl-local 的 5"
        );
        assert_eq!(r.1, 0, "doc_line 应为 0");
        assert_eq!(r.2, 1, "vis_line 应为 1（第二 VL）");

        // vis_line=1，点击簇左半段 → fraction≈0.33 < 0.5 → snap to prev_end
        let r =
            hit_test(110.0, lh * 1.5, &dv, &cache, &metrics, 100.0, 0.0, &dv.line_index).unwrap();
        assert_eq!(r.0, UniCharOffset(5), "第二 VL 左半段应返回 line-local offset=5");

        // vis_line=0（第一 VL），右半段点击
        let r =
            hit_test(120.0, lh * 0.5, &dv, &cache, &metrics, 100.0, 0.0, &dv.line_index).unwrap();
        assert_eq!(r.0, UniCharOffset(5), "第一 VL 右半段应返回 line-local offset=5");
    }

    #[test]
    fn hit_test_scrolled_second_vl_preserves_line_local_offset() {
        let line_bytes = b"abcdefghij";
        let clusters: Vec<_> = (0..10)
            .map(|i| shaping::GlyphCluster {
                byte_range: i..i + 1,
                glyph_id: 0,
                font_id: shaping::FontId::default(),
                advance: 10.0,
                x_offset: 0.0,
                y_offset: 0.0,
            })
            .collect();
        let shaped = shaping::ShapedRun { clusters, width: 100.0 };
        let visual_lines = vec![(0, 5, 50.0), (5, 10, 50.0)];
        let mut cluster_pool = Vec::new();
        let cache = ui::layout::build_advance_cache_entries(
            &visual_lines,
            1,
            &shaped,
            line_bytes,
            10.0,
            0,
            &mut cluster_pool,
            100.0,
        );

        let dv = make_dv("abcdefghij");
        let settings = Settings::new();
        let metrics = UiMetrics::from_settings(&settings, 1.0);
        let result = hit_test(
            120.0,
            metrics.line_height * 0.5,
            &dv,
            &cache,
            &metrics,
            100.0,
            0.0,
            &dv.line_index,
        )
        .expect("second visual line should be hit after scroll");

        assert_eq!(result.0, UniCharOffset(7));
        assert_eq!(result.1, 0);
        assert_eq!(result.2, 0);
    }

    #[test]
    fn test_double_click_spatial_proximity() {
        let mut dv = make_dv("hello world this is a test");
        let mut mouse = MouseState::new();
        let modifiers = winit::keyboard::ModifiersState::empty();

        // First click at position (10, 10)
        handle_mouse_input(
            winit::event::ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(0), 0, 0)),
        );

        assert_eq!(mouse.click_count, 1);

        // Release
        handle_mouse_input(
            winit::event::ElementState::Released,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(0), 0, 0)),
        );

        // Second click within 500ms but FAR AWAY (100, 100) -> distance squared is large
        handle_mouse_input(
            winit::event::ElementState::Pressed,
            100.0,
            100.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(15), 0, 0)),
        );

        // Click count should be reset to 1 due to spatial proximity check
        assert_eq!(
            mouse.click_count, 1,
            "Click count should reset if spatial distance is too large"
        );
    }

    #[test]
    fn test_mouse_release_clears_empty_selection() {
        let mut dv = make_dv("hello world");
        let mut mouse = MouseState::new();
        let modifiers = winit::keyboard::ModifiersState::empty();

        // Single click
        handle_mouse_input(
            winit::event::ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(5), 0, 0)),
        );

        // Simulate drag but cursor hasn't moved (so selection anchor == cursor)
        dv.cursor_mut().selection_anchor = Some(5);
        dv.cursor_move_to_offset(5);

        // Release
        handle_mouse_input(
            winit::event::ElementState::Released,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(5), 0, 0)),
        );

        // Empty selection should be cleared on release
        assert_eq!(
            dv.cursor().selection_anchor,
            None,
            "Empty selection should be cleared on mouse release"
        );
    }

    #[test]
    fn single_click_on_wrapped_vl_sets_click_hint() {
        // Click on VL1 start of wrapped line. click_hint should store
        // the vis_line so cursor rendering avoids VL boundary ambiguity.
        let mut dv = make_dv("abcdefghij");
        let mut mouse = MouseState::new();
        let modifiers = winit::keyboard::ModifiersState::empty();

        // Simulate hit_test result for click on VL1 (second visual line)
        // at byte 5 (start of VL1). vis_line=1.
        handle_mouse_input(
            winit::event::ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(5), 0, 1)),
        );

        // Cursor should be at byte 5 (start of VL1)
        assert_eq!(dv.cursor().offset, ByteIndex(5), "cursor should be at byte 5");

        // click_hint should be set with vis_line=1
        let hint = dv.presentation.cursor_render_state.click_hint;
        assert!(hint.is_some(), "click_hint should be set");
        assert_eq!(hint.unwrap(), (UniCharOffset(5), 1), "click_hint should be (5, 1, 0)");
    }

    #[test]
    fn shift_click_sets_click_hint() {
        // Shift+click extends selection AND should set click_hint
        // so cursor renders at the correct VL.
        let mut dv = make_dv("abcdefghij");
        let mut mouse = MouseState::new();
        let mut modifiers = winit::keyboard::ModifiersState::empty();
        modifiers.set(ModifiersState::SHIFT, true);
        dv.cursor_move_to_offset(0); // anchor at start

        handle_mouse_input(
            winit::event::ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(5), 0, 1)),
        );

        assert_eq!(dv.cursor().offset, ByteIndex(5), "shift+click cursor at byte 5");
        let hint = dv.presentation.cursor_render_state.click_hint;
        assert!(hint.is_some(), "shift+click should set click_hint");
        assert_eq!(hint.unwrap(), (UniCharOffset(5), 1), "click_hint should be (5, 1, 0)");
    }

    #[test]
    fn keyboard_clears_click_hint() {
        // After keyboard input, click_hint should be cleared
        // (tested indirectly: execute_edit_command clears it).
        // Here we verify the field exists and is default None.
        let dv = make_dv("hello");
        assert!(
            dv.presentation.cursor_render_state.click_hint.is_none(),
            "new DocumentView should have click_hint=None"
        );
    }

    #[test]
    fn single_click_clears_existing_selection() {
        // Single click should clear any existing selection (e.g., from double-click)
        let mut dv = make_dv("hello world test");
        let mut mouse = MouseState::new();
        let modifiers = winit::keyboard::ModifiersState::empty();

        // First, double-click to select a word (positions 0-5 for "hello")
        handle_mouse_input(
            winit::event::ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(0), 0, 0)),
        );
        handle_mouse_input(
            winit::event::ElementState::Released,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(0), 0, 0)),
        );

        // Second click (double-click) to trigger word selection
        handle_mouse_input(
            winit::event::ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(0), 0, 0)),
        );

        // Verify selection exists (double-click should have selected word)
        assert!(dv.cursor().selection_anchor.is_some(), "Double-click should create selection");

        // Release mouse
        handle_mouse_input(
            winit::event::ElementState::Released,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(0), 0, 0)),
        );

        // Now single click at a different position (offset 10)
        handle_mouse_input(
            winit::event::ElementState::Pressed,
            20.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(10), 0, 0)),
        );

        // Selection should be cleared
        assert_eq!(
            dv.cursor().selection_anchor,
            None,
            "Single click should clear existing selection"
        );
        assert_eq!(dv.cursor().offset, ByteIndex(10), "Cursor should move to clicked position");
    }

    #[test]
    fn single_click_drag_selects() {
        // Single click and drag should create selection from click point
        let mut dv = make_dv("hello world test");
        let mut mouse = MouseState::new();
        let modifiers = winit::keyboard::ModifiersState::empty();

        // Single click at offset 0
        handle_mouse_input(
            winit::event::ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(0), 0, 0)),
        );

        // Verify down_byte_offset is set for drag selection
        assert_eq!(mouse.down_byte_offset, Some(0), "Single click should set down_byte_offset");

        // Simulate drag to offset 5
        let hit = Some((UniCharOffset(5), 0, 0));
        let needs_redraw = handle_cursor_moved(20.0, 10.0, &mut mouse, &mut dv, hit);

        // Should create selection from 0 to 5
        assert!(needs_redraw, "Drag should trigger redraw");
        assert_eq!(
            dv.cursor().selection_anchor,
            Some(0),
            "Single click drag should create selection anchor at click point"
        );
        assert_eq!(dv.cursor().offset, ByteIndex(5), "Cursor should move to drag position");
    }

    #[test]
    fn double_click_then_tiny_move_preserves_word_selection() {
        // Regression: double-click selects word, but tiny CursorMoved (mouse jitter)
        // should NOT shrink the selection to just the clicked character.
        let mut dv = make_dv("hello world test");
        let mut mouse = MouseState::new();
        let modifiers = winit::keyboard::ModifiersState::empty();

        // First click (establishes click position)
        handle_mouse_input(
            winit::event::ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(2), 0, 0)), // click on 'l' in "hello"
        );
        handle_mouse_input(
            winit::event::ElementState::Released,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(2), 0, 0)),
        );

        // Double-click on same position
        handle_mouse_input(
            winit::event::ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(2), 0, 0)), // click on 'l' in "hello"
        );

        // Verify double-click selected the whole word
        assert_eq!(dv.cursor().selection_anchor, Some(0), "anchor should be word start");
        assert_eq!(dv.cursor().offset, ByteIndex(5), "cursor should be at word end");

        // Simulate tiny mouse movement (jitter) — hit_test returns same position
        let hit = Some((UniCharOffset(2), 0, 0));
        handle_cursor_moved(11.0, 10.0, &mut mouse, &mut dv, hit);

        // Selection should STILL be the full word, not [0, 2)
        assert_eq!(
            dv.cursor().selection_anchor,
            Some(0),
            "anchor should remain at word start after jitter"
        );
        assert_eq!(
            dv.cursor().offset,
            ByteIndex(5),
            "cursor should remain at word end after jitter, not shrink to click pos"
        );
    }

    #[test]
    fn double_click_drag_right_extends_by_words() {
        // Double-click on "hello", drag right to "world" → selection covers both words.
        let mut dv = make_dv("hello world test");
        let mut mouse = MouseState::new();
        let modifiers = winit::keyboard::ModifiersState::empty();

        // First click + release
        handle_mouse_input(
            winit::event::ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(2), 0, 0)),
        );
        handle_mouse_input(
            winit::event::ElementState::Released,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(2), 0, 0)),
        );

        // Double-click
        handle_mouse_input(
            winit::event::ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(2), 0, 0)),
        );
        assert_eq!(dv.selection_range(), Some((0, 5)), "should select 'hello'");

        // Drag right to "world" (offset 8 = inside "world")
        handle_cursor_moved(30.0, 10.0, &mut mouse, &mut dv, Some((UniCharOffset(8), 0, 0)));

        // Should extend to cover both words
        assert_eq!(dv.cursor().selection_anchor, Some(0), "anchor at word_start (hello)");
        assert_eq!(dv.cursor().offset, ByteIndex(11), "cursor at word_end of 'world'");
    }

    #[test]
    fn double_click_drag_left_extends_by_words() {
        // Double-click on "world", drag left to "hello" → selection covers both words.
        let mut dv = make_dv("hello world test");
        let mut mouse = MouseState::new();
        let modifiers = winit::keyboard::ModifiersState::empty();

        // First click + release on "world" (offset 7)
        handle_mouse_input(
            winit::event::ElementState::Pressed,
            20.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(7), 0, 0)),
        );
        handle_mouse_input(
            winit::event::ElementState::Released,
            20.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(7), 0, 0)),
        );

        // Double-click on "world"
        handle_mouse_input(
            winit::event::ElementState::Pressed,
            20.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(7), 0, 0)),
        );
        assert_eq!(dv.selection_range(), Some((6, 11)), "should select 'world'");

        // Drag left to "hello" (offset 2 = inside "hello")
        handle_cursor_moved(5.0, 10.0, &mut mouse, &mut dv, Some((UniCharOffset(2), 0, 0)));

        // Anchor should switch to word_end (11), cursor to word_start of "hello" (0)
        assert_eq!(
            dv.cursor().selection_anchor,
            Some(11),
            "anchor should be at original word_end (world end)"
        );
        assert_eq!(dv.cursor().offset, ByteIndex(0), "cursor at word_start of 'hello'");
    }

    #[test]
    fn triple_click_then_tiny_move_preserves_line_selection() {
        // Regression: triple-click selects line, but tiny CursorMoved should NOT
        // shrink the selection.
        let mut dv = make_dv("first line\nsecond line\nthird line");
        let mut mouse = MouseState::new();
        let modifiers = winit::keyboard::ModifiersState::empty();

        // First click + release
        handle_mouse_input(
            winit::event::ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(2), 0, 0)),
        );
        handle_mouse_input(
            winit::event::ElementState::Released,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(2), 0, 0)),
        );

        // Second click + release (builds to double)
        handle_mouse_input(
            winit::event::ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(2), 0, 0)),
        );
        handle_mouse_input(
            winit::event::ElementState::Released,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(2), 0, 0)),
        );

        // Triple-click on first line
        handle_mouse_input(
            winit::event::ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(2), 0, 0)),
        );

        let line_start = dv.line_byte_offset(0).unwrap_or(0);
        let line_end = dv.line_byte_offset(1).unwrap_or(dv.buffer_len());
        assert_eq!(dv.cursor().selection_anchor, Some(line_start));
        assert_eq!(dv.cursor().offset, ByteIndex(line_end), "cursor at line end");

        // Simulate tiny mouse movement (jitter)
        handle_cursor_moved(11.0, 10.0, &mut mouse, &mut dv, Some((UniCharOffset(2), 0, 0)));

        // Selection should still be the full line
        assert_eq!(
            dv.cursor().selection_anchor,
            Some(line_start),
            "anchor should remain at line start after jitter"
        );
        assert_eq!(
            dv.cursor().offset,
            ByteIndex(line_end),
            "cursor should remain at line end after jitter"
        );
    }

    #[test]
    fn triple_click_drag_extends_by_lines() {
        // Triple-click on first line, drag to second line → selection covers both lines.
        let mut dv = make_dv("first\nsecond\nthird");
        let mut mouse = MouseState::new();
        let modifiers = winit::keyboard::ModifiersState::empty();

        // Build up to triple-click (3 press/release cycles)
        for _ in 0..2 {
            handle_mouse_input(
                winit::event::ElementState::Pressed,
                10.0,
                10.0,
                &mut mouse,
                &mut dv,
                modifiers,
                Some((UniCharOffset(2), 0, 0)),
            );
            handle_mouse_input(
                winit::event::ElementState::Released,
                10.0,
                10.0,
                &mut mouse,
                &mut dv,
                modifiers,
                Some((UniCharOffset(2), 0, 0)),
            );
        }
        // Triple-click press
        handle_mouse_input(
            winit::event::ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(2), 0, 0)),
        );

        let line0_start = dv.line_byte_offset(0).unwrap();
        let line0_end = dv.line_byte_offset(1).unwrap();
        let line1_end = dv.line_byte_offset(2).unwrap_or(dv.buffer_len());
        assert_eq!(dv.selection_range(), Some((line0_start, line0_end)));

        // Drag to second line (doc_line=1)
        handle_cursor_moved(10.0, 30.0, &mut mouse, &mut dv, Some((UniCharOffset(8), 1, 1)));

        // Should extend to cover both lines
        assert_eq!(dv.cursor().selection_anchor, Some(line0_start));
        assert_eq!(dv.cursor().offset, ByteIndex(line1_end));
    }

    #[test]
    fn click_on_empty_area_clears_selection() {
        // 选中文本后，在空白区域点击应该取消选区
        let mut dv = make_dv("hello world");
        let mut mouse = MouseState::new();
        let modifiers = winit::keyboard::ModifiersState::empty();

        // 先用鼠标选中 "world" (offset 6..11)
        handle_mouse_input(
            ElementState::Pressed,
            20.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(6), 0, 0)),
        );
        // 拖拽到 offset 11
        handle_cursor_moved(60.0, 10.0, &mut mouse, &mut dv, Some((UniCharOffset(11), 0, 0)));
        handle_mouse_input(
            ElementState::Released,
            60.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(11), 0, 0)),
        );

        // 确认有选区
        assert!(dv.selection_range().is_some(), "应该有选区");

        // 在空白区域点击（hit = None）
        handle_mouse_input(
            ElementState::Pressed,
            10.0,
            200.0,
            &mut mouse,
            &mut dv,
            modifiers,
            None,
        );

        // 选区应该被清除
        assert!(dv.selection_range().is_none(), "点击空白区域后选区应该被清除");
    }

    #[test]
    fn hit_test_uses_physical_line_height() {
        let dv = make_dv("abcdefghij");
        let cache = vec![
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 0,
                vl_grapheme_start: 0,
                clusters: vec![(5, 130.0, 0)],
            },
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 5,
                vl_grapheme_start: 0,
                clusters: vec![(5, 130.0, 0)],
            },
        ];
        let settings = Settings::new();
        let metrics = UiMetrics::from_settings(&settings, 2.0);

        let hit = hit_test(
            40.0,
            metrics.line_height + 1.0,
            &dv,
            &cache,
            &metrics,
            32.0,
            0.0,
            &dv.line_index,
        )
        .unwrap();

        assert_eq!(hit.2, 1);
    }

    // ── cursor_move_to_unichar: direct unit tests ─────────────────────

    #[test]
    fn cursor_move_to_unichar_ascii() {
        // ASCII text: unichar offset == byte offset
        let mut dv = make_dv("hello world");
        dv.cursor_move_to_unichar(UniCharOffset(3));
        assert_eq!(dv.cursor().offset, ByteIndex(3), "ASCII unichar 3 → byte 3");
    }

    #[test]
    fn cursor_move_to_unichar_cjk() {
        // CJK text: each character is 1 unichar but 3 bytes
        // "汉字测试" → 汉=byte0, 字=byte3, 测=byte6, 试=byte9
        let mut dv = make_dv("汉字测试");
        dv.cursor_move_to_unichar(UniCharOffset(2));
        assert_eq!(dv.cursor().offset, ByteIndex(6), "CJK unichar 2 → byte 6 (after 汉字)");
    }

    #[test]
    fn cursor_move_to_unichar_mixed() {
        // Mixed text: "hello汉字" → hello=5 ASCII chars, then 2 CJK chars
        // UniCharOffset(5) should land at byte 5 (between 'o' and '汉')
        let mut dv = make_dv("hello汉字");
        dv.cursor_move_to_unichar(UniCharOffset(5));
        assert_eq!(
            dv.cursor().offset,
            ByteIndex(5),
            "mixed unichar 5 → byte 5 (after hello, before 汉)"
        );
    }

    #[test]
    fn cursor_move_to_unichar_boundary_zero() {
        // UniCharOffset(0) should land at the start of the line
        let mut dv = make_dv("hello");
        dv.cursor_move_to_unichar(UniCharOffset(0));
        assert_eq!(dv.cursor().offset, ByteIndex(0), "unichar 0 → byte 0 (line start)");
    }

    #[test]
    fn cursor_move_to_unichar_boundary_exceeds() {
        // UniCharOffset beyond line length should clamp to line end
        let mut dv = make_dv("hello");
        dv.cursor_move_to_unichar(UniCharOffset(100));
        assert_eq!(dv.cursor().offset, ByteIndex(5), "unichar beyond end → byte at line end");
    }

    // ── double-click / triple-click: multibyte text ───────────────────

    #[test]
    fn double_click_cjk_selects_word() {
        // CJK ideographs are classified as Word (same as ASCII letters).
        // Double-click on "世" in "hello 世界 test" (with spaces) should
        // select "世界" as a contiguous CJK word.
        let mut dv = make_dv("hello 世界 test");
        let mut mouse = MouseState::new();
        let modifiers = winit::keyboard::ModifiersState::empty();

        // "hello " = 6 bytes, "世" starts at byte 6 (unichar 6)
        handle_mouse_input(
            ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(6), 0, 0)),
        );
        handle_mouse_input(
            ElementState::Released,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(6), 0, 0)),
        );
        // Second click (double-click)
        handle_mouse_input(
            ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(6), 0, 0)),
        );

        let sel = dv.selection_range();
        assert!(sel.is_some(), "double-click on CJK word should create selection");
        let (start, end) = sel.unwrap();
        // "世界" at bytes 6..12 (each CJK char is 3 bytes)
        assert_eq!(start, 6, "selection should start at byte 6 (世 start)");
        assert_eq!(end, 12, "selection should end at byte 12 (界 end)");
    }

    #[test]
    fn triple_click_cjk_selects_line() {
        // Triple-click on a line containing CJK text → entire line selected
        let mut dv = make_dv(
            "汉字测试
def",
        );
        let mut mouse = MouseState::new();
        let modifiers = winit::keyboard::ModifiersState::empty();

        // Build up to triple click
        for _ in 0..2 {
            handle_mouse_input(
                ElementState::Pressed,
                10.0,
                10.0,
                &mut mouse,
                &mut dv,
                modifiers,
                Some((UniCharOffset(1), 0, 0)),
            );
            handle_mouse_input(
                ElementState::Released,
                10.0,
                10.0,
                &mut mouse,
                &mut dv,
                modifiers,
                Some((UniCharOffset(1), 0, 0)),
            );
        }
        // Third click (triple-click)
        handle_mouse_input(
            ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(1), 0, 0)),
        );

        let line0_start = dv.line_byte_offset(0).unwrap_or(0);
        let line0_end = dv.line_byte_offset(1).unwrap_or(dv.buffer_len());
        let sel = dv.selection_range();
        assert!(sel.is_some(), "triple-click on CJK line should select entire line");
        assert_eq!(sel.unwrap(), (line0_start, line0_end), "should select entire first line");
    }

    #[test]
    fn double_click_emoji_selects_cluster() {
        // Double-click on emoji "🌍" → should select the emoji cluster
        let mut dv = make_dv("hello🌍world");
        let mut mouse = MouseState::new();
        let modifiers = winit::keyboard::ModifiersState::empty();

        // First click on "🌍" (unichar 5)
        handle_mouse_input(
            ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(5), 0, 0)),
        );
        handle_mouse_input(
            ElementState::Released,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(5), 0, 0)),
        );
        // Second click (double-click)
        handle_mouse_input(
            ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(5), 0, 0)),
        );

        let sel = dv.selection_range();
        assert!(sel.is_some(), "double-click on emoji should create selection");
        let (start, end) = sel.unwrap();
        // "🌍" is at byte 5..9 (4 bytes)
        assert_eq!(start, 5, "selection should start at byte 5 (🌍 start)");
        assert_eq!(end, 9, "selection should end at byte 9 (🌍 end)");
    }

    // ── byte_to_unichar_offset: combining chars + wrapped line ────────

    #[test]
    fn byte_to_unichar_combining_chars() {
        // "aé汉" where é = e + combining acute (NFD): a(1) + e+combining(2+1) + 汉(3) = 7 bytes
        // Unichar offsets: a=0, é=1, 汉=2
        let content = "ae\u{0301}汉";
        let dv = make_dv(content);

        // byte_to_unichar at byte 0 → unichar 0 ('a')
        assert_eq!(dv.byte_to_unichar_offset(0), UniCharOffset(0), "byte 0 → unichar 0");
        // byte_to_unichar at byte 1 → unichar 1 ('é' start)
        assert_eq!(dv.byte_to_unichar_offset(1), UniCharOffset(1), "byte 1 → unichar 1 (é)");
        // byte_to_unichar at byte 3 (after é, before 汉) → unichar 2
        assert_eq!(dv.byte_to_unichar_offset(3), UniCharOffset(2), "byte 3 → unichar 2 (汉)");
    }

    #[test]
    fn byte_to_unichar_wrapped_line() {
        // For a single long line that wraps, byte_to_unichar should still
        // produce correct document-level unichar offsets regardless of
        // visual line splits (byte_to_unichar works at doc level).
        let content = "abcdefghij"; // 10 ASCII chars
        let dv = make_dv(content);

        // All chars are ASCII, so byte == unichar offset
        for b in 0..10 {
            assert_eq!(
                dv.byte_to_unichar_offset(b),
                UniCharOffset(b),
                "ASCII byte {b} should equal unichar {b}"
            );
        }
    }

    #[test]
    fn single_click_on_empty_line() {
        // "aaa\n\nbbb\n" → lines: ["aaa", "", "bbb"]
        // Line 1 (empty) shares unichar 3 with line 2 ("bbb").
        // Clicking on the empty line must put the cursor on the empty line,
        // NOT on "bbb".
        let mut dv = make_dv("aaa\n\nbbb\n");
        let mut mouse = MouseState::new();
        let modifiers = winit::keyboard::ModifiersState::empty();
        let empty_line_byte = dv.line_byte_offset(1).unwrap(); // byte 4

        handle_mouse_input(
            winit::event::ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(3), 1, 1)), // offset=3, doc_line=1 (empty), vis_line=1
        );

        assert_eq!(
            dv.cursor().offset.to_usize(),
            empty_line_byte,
            "cursor should be on the empty line, not the text line below"
        );
        assert_eq!(dv.cursor().selection_anchor, None, "single click clears selection");
    }

    #[test]
    fn single_click_on_text_line_after_empty_line() {
        // "aaa\n\nbbb\n" — click at start of "bbb" (line 2)
        // offsets = [0, 4, 5], unichar_offsets = [0, 3, 3]
        // Click at unichar 3 on doc_line 2: local = 3-3 = 0 → start of "bbb"
        let mut dv = make_dv("aaa\n\nbbb\n");
        let mut mouse = MouseState::new();
        let modifiers = winit::keyboard::ModifiersState::empty();
        let bbb_byte = dv.line_byte_offset(2).unwrap(); // byte 5

        handle_mouse_input(
            winit::event::ElementState::Pressed,
            30.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(3), 2, 2)), // start of line 2
        );

        assert_eq!(dv.cursor().offset.to_usize(), bbb_byte, "cursor should be at start of bbb");
    }

    #[test]
    fn click_on_consecutive_empty_lines() {
        // "a\n\n\nc\n" → two consecutive empty lines (line 1 and line 2)
        // offsets = [0, 2, 3, 4], unichar_offsets = [0, 1, 1, 1]
        let modifiers = winit::keyboard::ModifiersState::empty();

        // Click first empty line (doc_line=1) with fresh mouse
        let mut dv = make_dv("a\n\n\nc\n");
        let mut mouse = MouseState::new();
        let first_empty = dv.line_byte_offset(1).unwrap(); // byte 2
        handle_mouse_input(
            winit::event::ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(1), 1, 1)),
        );
        assert_eq!(
            dv.cursor().offset.to_usize(),
            first_empty,
            "click on first empty line should set cursor there"
        );

        // Click second empty line (doc_line=2) with fresh mouse
        let mut dv2 = make_dv("a\n\n\nc\n");
        let mut mouse2 = MouseState::new();
        let second_empty = dv2.line_byte_offset(2).unwrap(); // byte 3
        handle_mouse_input(
            winit::event::ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse2,
            &mut dv2,
            modifiers,
            Some((UniCharOffset(1), 2, 2)),
        );
        assert_eq!(
            dv2.cursor().offset.to_usize(),
            second_empty,
            "click on second empty line should set cursor there"
        );
    }

    #[test]
    fn drag_on_empty_lines_uses_hint_line() {
        // "a\n\nc\n" — drag across empty lines
        let mut dv = make_dv("a\n\nc\n");
        let mut mouse = MouseState::new();
        let modifiers = winit::keyboard::ModifiersState::empty();

        // Click first (anchor) on "a"
        handle_mouse_input(
            winit::event::ElementState::Pressed,
            10.0,
            10.0,
            &mut mouse,
            &mut dv,
            modifiers,
            Some((UniCharOffset(0), 0, 0)),
        );

        // Drag to empty line — must land on empty line, not "c"
        handle_cursor_moved(10.0, 20.0, &mut mouse, &mut dv, Some((UniCharOffset(1), 1, 1)));

        let empty_line_byte = dv.line_byte_offset(1).unwrap();
        assert_eq!(
            dv.cursor().offset.to_usize(),
            empty_line_byte,
            "drag to empty line should set cursor on the empty line"
        );
    }
}
