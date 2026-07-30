# WYSIWYG 样式化 Advance 修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让含粗体等内联样式的 Markdown WYSIWYG 行，在绘制、点击命中与光标位置上使用相同的 grapheme advance。

**Architecture:** 渲染器已经按每个样式段的真实字体宽度绘制文本，但 `FlatLine` 仍保存整行常规字重的 `ShapedRun`。布局精度阶段将在计算样式段后，再为相同文本拼装一个按样式 shaping 的 `ShapedRun`，作为 `FlatLine` 的命中和光标几何来源；源码 byte 映射逻辑保持不变。

**Tech Stack:** Rust、textora-markdown、Shaper、Unicode grapheme、cargo test。

## Global Constraints

- 产品名为 textora，Markdown crate 包名为 `textora-markdown`。
- 光标只停在 Unicode grapheme cluster 边界；不得按 UTF-8 byte 中点分割字符。
- 不改变 Markdown 语义、源码 byte 映射或 UI/app 依赖关系。
- 使用 `cargo fmt`；不使用无说明的 `.unwrap()`。

---

### Task 1: 统一样式化行的视觉 advance

**Files:**

- Modify: `crates/markdown/src/view.rs`
- Modify: `crates/markdown/src/layout/shaping.rs`

**Interfaces:**

- Consumes: `LaidOutLine::text`、`LaidOutLine::styles`、`StyleSegment` 与 `Shaper`。
- Produces: `LaidOutLine::shaped` 在有内联样式时包含与渲染器相同的按段 glyph advance；`FlatLine` 继续复制该 run，既有 `grapheme_x` / `grapheme_at_x` 自动用于光标和命中。

- [ ] **Step 1: 写失败回归测试**

在 `view.rs` 用实际第 8 行列表项构造 `- **Top-tier visual impact**: … — …`。把光标设在粗体闭合标记之后的 `:`，从精确布局读取粗体 `StyleSegment.width`，断言 `cursor_screen_pos().x` 与该视觉边界一致；用同一 x 做 `hit_test_byte`，断言返回 `:` 的源码 byte。修复前，这两项会因整行常规字宽小于粗体段字宽而失败。

- [ ] **Step 2: 运行并记录 RED**

Run: `cargo test -p textora-markdown --lib view::wysiwyg_tests::styled_prefix_cursor_and_hit_test_use_rendered_advance -- --exact`

Expected: FAIL，光标 x 与 `StyleSegment.width` 存在明显差值，或命中返回粗体段内 byte。

- [ ] **Step 3: 构造带样式的 ShapedRun**

在 `layout/shaping.rs` 新增内部 helper：顺序遍历普通间隔与 `StyleSpan`，用渲染器相同的 `Weight` / `Style` 分别 shape 每个片段，将每个 glyph cluster 的 byte range 平移回整行坐标后拼接。对于斜体，将现有渲染位置使用的额外 advance 计入该样式段最后一个 cluster。`populate_style_segments` 必须先计算 `StyleSegment`，随后将该 helper 的结果写回 `line.shaped`。

- [ ] **Step 4: 运行 GREEN 与回归测试**

Run: `cargo test -p textora-markdown --lib view::wysiwyg_tests::styled_prefix_cursor_and_hit_test_use_rendered_advance -- --exact && cargo test -p textora-markdown --lib view::wysiwyg_tests::promotion_blockquote_click_roundtrip_and_vertical_navigation_reach_line_three -- --exact && cargo test -p textora-markdown --lib view::wysiwyg_tests::promotion_em_dash_click_roundtrip_never_lands_inside_utf8_sequence -- --exact`

Expected: PASS；line 3 命中仍到达真实源码行，`—` 左右移动仍只返回合法 grapheme 边界。

- [ ] **Step 5: 格式化、完整验证与提交**

Run: `cargo fmt --check && cargo test -p textora-markdown --lib && cargo check -p textora-markdown`

Expected: 全部成功。

```bash
git add crates/markdown/src/view.rs crates/markdown/src/layout/shaping.rs
git commit -m "fix(markdown): align WYSIWYG cursor with styled text advances"
```
