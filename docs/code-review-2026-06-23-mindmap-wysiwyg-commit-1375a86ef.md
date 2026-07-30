# Code Review: `1375a86ef` — Mindmap WYSIWYG 渲染与键盘事件

## Overview

commit 涵盖 MindmapView 的完整实现 + app 层 InterceptKey 管线 + DPI 适配 + 大量 cargo fmt 格式化 + 两份无关设计文档。25 文件，+1022/-282 行。

---

## 1. 🔴 Critical: `allows_editing()` 从 `true` 改为 `false` — 文本编辑路径断裂

**位置**: `crates/markdown/src/mindmap_view.rs:302-308`

```rust
fn allows_editing(&self) -> bool {
    false   // ← 原设计是 true
}
```

**问题**: `events.rs` 的键盘路由逻辑是 `!tab.plugin.allows_editing()` 时转发到 `PluginInterceptKey`。`false` 意味着**所有按键事件**都走 InterceptKey 拦截路径。但 `mindmap_view.rs:handle_message` 对 `InterceptKey` 的处理只认 Tab/Shift+Tab/Enter/Ctrl+Enter 四种结构编辑键——其他所有键（包括中文输入、Backspace、Delete）都返回 `false`，然后执行 fallback。

关键在于 `events.rs:135-152` 的 fallback 逻辑：
```rust
if let Some(tab) = app.workspace.active_entry()
    && !tab.plugin.allows_editing()  // ← MindmapView 返回 false → 命中
{
    actions.push(AppAction::PluginInterceptKey {
        key: kc,
        modifiers: mods,
        fallback: fallback_cmd,  // ← 未被 InterceptKey 消费时执行
    });
    return actions;
}
```

fallback 会执行原来的 `EditCommand`（如 InsertText、Backspace），这些命令会**直接修改 DocumentView 的源码**。所以**文本编辑路径并未断裂**——只是走了 fallback 而非直接编辑管道。

但有一个严重的**语义偏差**：`allows_editing() = false` 使 app 层的编辑模式判断（`dispatch/editor.rs` 中 `is_editor`、`switch_plugin` 等）认为 MindmapView **不是编辑器**。这会影响：
- 切换插件时的缓存/恢复逻辑
- 编辑相关的 UI 状态（如 TitleBar 工具按钮显隐）

**建议**: 恢复为 `true`。如果路由需要区分，应通过新的 trait 方法表达（如 `fn intercepts_keys() -> bool`），而非复用 `allows_editing()` 的语义。

---

## 2. 🔴 Important: `visible_range()` 裁剪被禁用

**位置**: `crates/markdown/src/mmf/canvas.rs:617-629`

```rust
pub fn visible_range(layout: &LayoutTree, _viewport: Rect, _buffer: f32) -> (usize, usize) {
    // nodes is in DFS pre-order, NOT sorted by y!
    (0, layout.nodes.len())
}
```

**问题**: 注释说"DFS 先序遍历，不按 y 排序"——这是正确的。DFS 确实不保证 y 单调。但**完全禁用裁剪意味着每次 render 都绘制全部节点**。

DFS 序不等于 y 序的原因是：布局算法在分配子节点坐标时递归深入子树，而不是按 y 坐标线性展开。但 `LayoutTree.nodes` 是 DFS 序推入的，对于右分支树而言，同一深度层级内的节点确实不按 y 排序（因为子节点的子树会插入中间）。

**建议**: 在 `build_hit_map` 或 `compute_layout` 中额外维护一个**按 y 排序的节点索引数组** `y_sorted: Vec<usize>`，然后 `visible_range` 对 `y_sorted` 做二分查找，再映射回实际渲染索引。对于几百个节点的思维导图，当前全量渲染可能仍然可接受（DrawList GPU 裁剪），但这是需要标记为 TODO 的技术债。

---

## 3. 🟡 Important: 颜色获取从 `scope_color()` 改为直接 `scopes.get()` + 硬编码 alpha

**位置**: `crates/markdown/src/mmf/canvas.rs:645-665`

```rust
let node_border = theme.scopes.get("mindmap.node_border").copied().unwrap_or_else(|| {
    let mut c = theme.editor.foreground;
    c[3] = 0.2;  // ← 魔法值
    c
});
```

**问题**:
1. `theme.scope_color(name)` 本身就有 fallback 到 `editor.foreground` 的逻辑，直接改用 `scopes.get()` 等于**跳过了 Theme 的 fallback 链**（如 `scope_color` 可能还会检查 `markdown.*` 等命名空间）
2. 硬编码 alpha 值 `0.2`/`0.1`/`0.4`/`0.3` 应该提取为命名常量，符合 CLAUDE.md 中"消灭魔法值"的原则

**建议**: 恢复 `theme.scope_color()` 调用。如果当前 Theme 未定义这些 scope，应在 theme 文件中添加默认值（`theme_file.rs` 已有相关结构）。

---

## 4. 🟡 Important: DPI 变化检测逻辑脆弱

**位置**: `crates/markdown/src/mindmap_view.rs:206-212`

```rust
if self.constants.card_height != 32.0 * dpi_scale {
    self.constants.card_padding_x = 16.0 * dpi_scale;
    // ...
}
```

**问题**: 用 `32.0 * dpi_scale` 检测 DPI 变化。当 `LayoutConstants::default()` 中的 `card_height` 改变时（比如改为从 Theme 读入），这里的阈值也要手动同步，容易遗漏。

**建议**: 增加 `self.cached_dpi: f32` 字段，直接比较 `self.cached_dpi != dpi_scale`，然后统一调用 `self.constants = LayoutConstants::scaled(dpi_scale)`。

---

## 5. 🟢 Minor: 两份无关设计文档随 commit 混入

**位置**: 
- `docs/plans/2026-06-23-cursor-strong-types-refactor.md` (379 行)
- `docs/plans/2026-06-23-wysiwyg-crash-fix-and-cursor-render-review.md` (220 行)

这两个文件与 Mindmap WYSIWYG 实现无关（一个是 cursor 类型系统重构计划，另一个是 WYSIWYG markdown 编辑器的 code review）。应通过独立 commit 提交或拆分到各自的 feature branch。

---

## 6. 🟢 Minor: 格式化噪音占比高

约 60% 的 diff 是 `cargo fmt` 格式化变更（单行拆多行、import 重排、早期 return 展开等）。`builder.rs`、`layout.rs`、`workspace.rs`、`theme_file.rs` 的改动本质上是格式化，与功能无关。建议格式化变更单独一个 commit，功能变更集中在后续 commit，便于 review 和 `git bisect`。

---

## 7. 🟢 Minor: `toml::Table` → `toml::Value::Table` 语义变更

**位置**: `crates/markdown/src/mmf/parser.rs:1054`

```rust
// Before: toml_str.parse::<toml::Table>()
// After:  toml_str.parse::<toml::Value>()
if let Ok(toml::Value::Table(t)) = toml_str.parse::<toml::Value>() {
```

`toml::Value::Table` 接受更宽泛的 TOML——如果 `toml node` 代码块中写了非表格内容（如裸字符串），原代码直接拒绝，新代码会静默忽略。功能上无实际差异（MMF 规范要求 `toml node` 块必须是表），但错误检测能力略微降低。

---

## 8. 🟢 Bug Fix: parser 移除外层 `loop` 避免无限循环

**位置**: `crates/markdown/src/mmf/parser.rs:1027-1031`

栈式解析器的外层 `loop { while c.idx < c.lines.len() { ... } }` 被移除——原代码在 `while` 结束后 `loop` 重新进入但 `c.idx` 不递增 → 无限循环。这是正确的 bug 修复。

---

## Summary

| 级别 | 问题 | 建议 |
|---|---|---|
| 🔴 Critical | `allows_editing() = false` 语义偏差 | 恢复为 `true` |
| 🔴 Important | `visible_range()` 全量渲染 | 维护 `y_sorted` 索引数组 |
| 🟡 Important | `scopes.get()` 绕过 Theme fallback + 魔法 alpha | 恢复 `scope_color()` |
| 🟡 Important | DPI 检测用常量阈值 | 用 `cached_dpi` 字段 |
| 🟢 Minor | 无关文档混入 | 拆分为独立 commit |
| 🟢 Minor | ~60% fmt 噪音 | 格式化单独 commit |

**结论**: 整体架构正确——`PluginInterceptKey` 管线（events.rs → AppAction → app_dispatch → handle_message + fallback）设计合理。`visible_range` 禁用和 `allows_editing` 语义变更需要在合并前修正，其余问题可逐步跟进。
