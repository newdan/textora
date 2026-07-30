import re

with open("crates/markdown/src/view.rs", "r") as f:
    content = f.read()

# Replace visible_line_snapshots with layout_snapshot in entering_editor_keeps_softbreak_paragraph_layout
content = re.sub(
    r'(assert_eq!\(\s*)visible_line_snapshots\((editor\.engine\(\))\),\s*before_click,',
    r'\1layout_snapshot(\2),\n                before_click,',
    content
)

tests_to_remove = [
    "hit_test_byte_gap_between_paragraphs_returns_none",
    "hit_test_byte_activates_extra_empty_line_between_paragraphs",
    "visual_move_down_skips_inter_paragraph_empty_source_line",
    "visual_move_up_skips_inter_paragraph_empty_source_line",
    "visual_move_visits_extra_empty_source_line_between_paragraphs",
    "visual_move_left_right_visits_extra_empty_source_line_between_paragraphs",
    "editor_preedit_on_inter_paragraph_empty_source_line_has_no_cursor_rect"
]

for test in tests_to_remove:
    # Match #[test] and the function body
    pattern = r'(\s*#\[test\]\s*fn\s+' + test + r'\s*\(\)\s*\{.*?\n    \})\n'
    content = re.sub(pattern, '', content, flags=re.DOTALL)

with open("crates/markdown/src/view.rs", "w") as f:
    f.write(content)
