# 常见修改指南

本指南按任务类型列出需要触碰的模块和检查要点。

## 1. 新增或调整块级语法 / 渲染效果
- **解析与构树**：`src/parser.rs`（事件封装）→ `src/builder.rs`（`BlockKind` 与 `BlockNode` 构建）。
- **布局与间距**：
  - `src/layout/block.rs`（块级入口与容器/代码/嵌入布局调度）、`src/layout/block_text.rs`（文本与行内数学行高排版）、`src/layout/block_table.rs`（表格列宽与单元格排版）
  - `src/layout/types.rs`（若新增 `LaidOutBlockKind` 变体）
  - `src/layout/spacing.rs`（块间间距规则）
  - `src/layout/mod.rs`（如果新块像 `Container` 一样会展开为多个布局块，须同步更新 `count_laid_out_blocks`、`flatten_blocks` 和 `expands_into_multiple_laid_out_blocks`）。
- **绘制与样式**：`src/style.rs`（主题配色/字号常量）→ `src/render.rs`（块级/ASCII 图绘制与入口）与 `src/render_line.rs`（行级与行内样式/公式绘制）。
- **WYSIWYG 兼容**：若新块在编辑态有语法标记展开或特殊空行归属，检查 `src/edit.rs`、`src/editable_paragraphs.rs` 与 `src/augmenter/`。

## 2. 修改 WYSIWYG 按键行为（Enter / Backspace / Tab / 输入）
- **定位上下文**：在 `src/augmenter/classify.rs` 中查看 `EnterContext` 与 `classify_enter_context`（先确认是否命中已有上下文分支）。
- **生成变更**：使用 `src/augmenter/primitives.rs` 内既有的产物原语（`emit_*`）构造 `EditAugmentation`，保证满足字节边界不变量。
- **空段落相关**：如果涉及空行增删或跨块合并，修改需同步 `src/editable_paragraphs.rs` / `editable_paragraph_edit.rs` / `editable_paragraph_navigation.rs`，不要在 `augmenter/` 里另外手写换行扫描。
- **验证**：先在 `tests/system_boundary_audit.rs` 或对应单元测试中补充矩阵/回归用例，再修改实现。

## 3. 新增或修改语义编辑命令（工具栏 / 快捷键命令）
- **命令定义**：命令枚举 `SemanticEditCommand` 与返回类型 `SemanticEditPlan` 定义在 `ui::plugin`（如需新增命令变体，先改 `crates/ui`）。
- **命令规划**：在 `src/commands.rs`（或子模块 `commands/inline_formatting.rs`、`commands/table_structure.rs`）实现规划逻辑，返回源码替换事务，不直接持有编辑器状态。
- **宿主接线**：在 `crates/app/src/events.rs` 中接入（如需）。

## 4. 调整光标、选区、IME 或视图缓存
- **坐标与投影**：修改映射规则时优先改 `src/projection.rs` 与 `src/grapheme_map.rs`，确保 `SourceProjectionIndex` 成为唯一权威。
- **Preedit / 搜索 / 拆分历史 / 代码裁剪**：分别位于独立子文件 `src/view_preedit.rs`、`src/view_source_search.rs`、`src/view_split_history.rs`、`src/view_code_viewport.rs`，与 `src/view/` 解耦；修改缓存失效逻辑时检查 `PreviewEngine` 的 `EngineDirty` 状态机及增量复用（`LazyLayout::reuse_unchanged_blocks_from`）。

## 5. 新增嵌入内容或调整数学/图表渲染
- **双实现同步**：任何对 `src/embedded.rs` 公开接口（`EmbeddedKind`、`EmbeddedRequest`、`EmbeddedImage`、`EmbeddedError`、`render_embedded`）的修改，**必须**同步更新 `src/embedded_lite.rs`，保证 `--no-default-features` 编译通过。
- **布局与渲染接线**：`src/layout/embedded.rs` 与 `src/render.rs`。

## 6. 扩展智能粘贴（`paste/`）
- **新格式解析**：参考 `src/paste/html.rs` 或 `src/paste/rtf.rs`，将新格式解析为中间 AST `paste::model::RichDocument`。
- **Markdown 输出**：若 `RichDocument` 新增节点类型，同步修改 `src/paste/writer.rs`。
- **策略与校验**：在 `src/paste/selection.rs` 中接入入口与可见文字等价性校验；若依赖富文本库，注意用 `#[cfg(feature = "rich-markdown")]` 隔离。

## 7. 修改思维导图（`mmf/` + `mindmap_view.rs`）
- **语法/属性**：`src/mmf/model.rs` → `src/mmf/parser.rs`（注意维护 `source_range` / `subtree_source_range` / `property_source` 的精确字节范围）。
- **布局/命中/绘制**：`src/mmf/layout.rs`（`LayoutTree` / `HitMap`）→ `src/mmf/canvas.rs`（`render_cards` / 连线缓存）。
- **交互/编辑**：`src/mmf/edit.rs`（规划 `EditPlan`）→ `src/mindmap_view.rs` / `src/mindmap_view_interaction.rs`（`MindmapFocus` / `MindmapDragState` 状态机与拖拽/导航）。
