# mmap WYSIWYG Editing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 mmap 迁入统一 WYSIWYG 编辑协议，完成标题直接编辑、IME、节点选择、同级/子级创建、层级调整、子树删除与 Undo/Redo 闭环。

**Architecture:** `DocumentView` 中的 MMF 源码继续作为唯一真实状态；`ui::plugin` 只提供通用语义命中与编辑事务类型，`app` 负责原子执行事务，`textora-markdown` 负责解析 MMF、生成局部源码编辑计划并渲染画布。实现顺序先稳定通用事务和语义命中协议，再替换 mmap 的旧 `KeyInterceptor`/画布查询，最后补齐渲染、错误态与回归验证。

**Tech Stack:** Rust、winit、wgpu、textora-ui 插件协议、DocumentView/TextBuffer、textora-markdown MMF parser/layout/canvas、项目现有 shaping 与 Unicode grapheme 工具。

## Global Constraints

- 全程遵守 `crates/ui` 不依赖 `DocumentView`、Workspace、Commands 或 app 状态结构的跨层红线。
- `DocumentView` 中的 MMF 源码是唯一可写状态；禁止从 Tree 全量序列化覆盖源码。
- 所有 mmap 修改必须生成 `EditPlan`，禁止插件直接调用 `DocViewMut::replace_range()`。
- 每个结构操作必须是单个 Undo 单元；事务校验失败时不得产生部分写入。
- Node、布局、命中几何、焦点和预编辑状态使用 enum/struct 表达，禁止组合多个 bool 表示互斥状态。
- 新增命名必须精准自解释，禁止使用 `data`、`info`、`temp`、`res`、`flag` 等宽泛名称。
- 单个任务最多修改 3 个文件；每次提交前必须先通过该任务列出的编译与测试命令。
- 每个 bugfix 或行为变更必须先提交可复现的失败测试，再写实现。
- 提交前执行 `cargo fmt`；最终执行 `./scripts/verify.sh`。
- 设计依据：`docs/specs/2026-07-12-mmap-wysiwyg-editing-design.md`。

---

## File Structure

| 文件 | 单一职责 |
|---|---|
| `crates/ui/src/plugin.rs` | 定义通用多 replacement 事务、事务后选择、语义命中目标和插件按键意图映射协议。 |
| `crates/app/src/edit_transaction.rs` | 校验并原子执行多 replacement，统一更新光标/选择和 Undo 分组。 |
| `crates/app/src/dispatch/mouse.rs` | 将插件语义命中结果转换为 DocumentView 光标或源码对象选择。 |
| `crates/app/src/dispatch/wysiwyg.rs` | 对语义导航目标应用光标/对象选择，并保留旧字节导航 fallback。 |
| `crates/app/src/events.rs` | 在全局快捷键映射前询问插件的结构编辑按键意图。 |
| `crates/markdown/src/mmf/model.rs` | 保存节点标题、标记、子树和子节点插入锚点等源码范围，以及结构化解析诊断。 |
| `crates/markdown/src/mmf/parser.rs` | 解析 MMF 并精确计算源码范围、空标题和错误行列。 |
| `crates/markdown/src/mmf/edit.rs` | 将 mmap 编辑意图纯函数化为 `EditPlan`，不写 DocumentView。 |
| `crates/markdown/src/mmf/layout.rs` | 生成卡片布局、grapheme 命中几何和按 y 排序的可见索引。 |
| `crates/markdown/src/mmf/canvas.rs` | 渲染卡片、整节点选择、标题选择、空标题占位符、光标和预编辑投影。 |
| `crates/markdown/src/mindmap_view.rs` | 协调源码状态、布局、编辑策略、语义查询、导航、错误态和插件能力。 |
| `crates/app/src/app_tests.rs` | 验证 app 与 mmap 插件之间的输入、IME、Undo/Redo 和保存集成。 |

---

### Task 1: 原子多 replacement 编辑事务

**Files:**
- Modify: `crates/ui/src/plugin.rs:283-328`
- Modify: `crates/app/src/edit_transaction.rs:1-470`
- Modify: `crates/markdown/src/view.rs:2460-2505`（以及同文件内 `EditTransaction` 构造测试）

**Interfaces:**
- Produces: `EditSelection`、`EditTransaction { source_generation, replacements, selection_after }` 和无文本修改的 `EditPlan::SetSelection`。
- Preserves: `execute_edit_plan(plan, doc, advance_cache)` 签名；事务自身携带并校验 source generation、范围、重叠和事务后选择。
- Preserves: `execute_text_replacement()` 作为旧调用点的单 replacement 适配器。

- [ ] **Step 1: 在 app 事务测试中写出多 replacement、重叠拒绝和单次 Undo 的失败测试**

在 `crates/app/src/edit_transaction.rs` 的测试模块加入：

```rust
#[test]
fn execute_multiple_replacements_is_atomic_and_undoes_once() {
    let mut doc = document_from_text("# Root\n## Child\n### Leaf\n");
    let generation = doc.generation();
    let plan = EditPlan::Apply(EditTransaction {
        source_generation: generation,
        replacements: vec![
            TextReplacement { range: 7..7, text: "#".into() },
            TextReplacement { range: 16..16, text: "#".into() },
        ],
        selection_after: EditSelection::Caret(20),
    });

    execute_edit_plan(plan, &mut doc, &[]).expect("valid grouped transaction");
    assert_eq!(doc.full_text(), "# Root\n### Child\n#### Leaf\n");

    crate::commands::execute_edit_command_v2(&EditCommand::Undo, &mut doc, &[]);
    assert_eq!(doc.full_text(), "# Root\n## Child\n### Leaf\n");
}

#[test]
fn overlapping_replacements_are_rejected_without_writing() {
    let mut doc = document_from_text("abcdef");
    let generation = doc.generation();
    let plan = EditPlan::Apply(EditTransaction {
        source_generation: generation,
        replacements: vec![
            TextReplacement { range: 1..4, text: "X".into() },
            TextReplacement { range: 3..5, text: "Y".into() },
        ],
        selection_after: EditSelection::Caret(2),
    });

    assert_eq!(
        execute_edit_plan(plan, &mut doc, &[]),
        Err(EditTransactionError::OverlappingRanges { first_end: 4, second_start: 3 })
    );
    assert_eq!(doc.full_text(), "abcdef");
}

#[test]
fn stale_generation_is_rejected_without_writing() {
    let mut doc = document_from_text("abc");
    let stale_generation = doc.generation().wrapping_sub(1);
    let plan = EditPlan::Apply(EditTransaction::replace(
        stale_generation,
        1..2,
        "Z".into(),
        2,
    ));

    assert!(matches!(
        execute_edit_plan(plan, &mut doc, &[]),
        Err(EditTransactionError::StaleGeneration { .. })
    ));
    assert_eq!(doc.full_text(), "abc");
}

#[test]
fn set_selection_changes_focus_without_creating_text_edit() {
    let mut doc = document_from_text("abcdef");
    let plan = EditPlan::SetSelection(EditSelection::Range { anchor: 1, cursor: 5 });
    let outcome = execute_edit_plan(plan, &mut doc, &[]).expect("valid selection update");
    assert_eq!(doc.selection_range(), Some((1, 5)));
    assert!(!outcome.edit_outcome.executed);
}
```

- [ ] **Step 2: 运行测试并确认协议尚未支持这些字段**

Run:

```bash
cargo test -p textora-app --lib edit_transaction::tests::execute_multiple_replacements_is_atomic_and_undoes_once
```

Expected: FAIL，编译错误指出 `EditTransaction` 没有 `replacements` / `selection_after`，或缺少 `EditSelection`。

- [ ] **Step 3: 在 UI 协议中定义事务后选择和多 replacement**

在 `crates/ui/src/plugin.rs` 替换事务定义：

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditSelection {
    Caret(usize),
    Range { anchor: usize, cursor: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditTransaction {
    pub source_generation: u32,
    pub replacements: Vec<TextReplacement>,
    pub selection_after: EditSelection,
}

impl EditTransaction {
    pub fn replace(
        source_generation: u32,
        range: std::ops::Range<usize>,
        text: String,
        cursor_after: usize,
    ) -> Self {
        Self {
            source_generation,
            replacements: vec![TextReplacement { range, text }],
            selection_after: EditSelection::Caret(cursor_after),
        }
    }
}
```

`EditPlan` 增加：

```rust
SetSelection(EditSelection),
```

保留 `MoveCursor(CursorUpdate)` 供现有 Markdown 调用点兼容；新结构化视图使用 `SetSelection`。

同步更新 `plugin.rs` 自身测试，使用 `EditTransaction::replace(7, 4..4, "\n\n".into(), 6)`。

- [ ] **Step 4: 实现完整事务预校验和原子执行**

在 `crates/app/src/edit_transaction.rs` 增加错误类型并改造执行器：

```rust
#[derive(Debug, PartialEq, Eq)]
pub enum EditTransactionError {
    StaleGeneration { expected: u32, actual: u32 },
    OverlappingRanges { first_end: usize, second_start: usize },
    CursorOutOfBounds { cursor_after: usize, final_len: usize },
    InvalidRange { start: usize, end: usize, len: usize },
    InvalidCharBoundary { byte: usize },
    InvalidGraphemeBoundary { byte: usize },
    UnresolvedDefaultPlan,
}

fn sorted_replacements(
    transaction: &EditTransaction,
) -> Result<Vec<&TextReplacement>, EditTransactionError> {
    let mut replacements: Vec<_> = transaction.replacements.iter().collect();
    replacements.sort_by_key(|replacement| replacement.range.start);
    for pair in replacements.windows(2) {
        if pair[0].range.end > pair[1].range.start {
            return Err(EditTransactionError::OverlappingRanges {
                first_end: pair[0].range.end,
                second_start: pair[1].range.start,
            });
        }
    }
    Ok(replacements)
}

fn validate_source_generation(
    transaction: &EditTransaction,
    doc: &DocumentView,
) -> Result<(), EditTransactionError> {
    if transaction.source_generation == doc.generation() {
        return Ok(());
    }
    Err(EditTransactionError::StaleGeneration {
        expected: transaction.source_generation,
        actual: doc.generation(),
    })
}
```

`execute_edit_plan()` 保持现有签名。`SetSelection` 分支复用 char/grapheme 边界验证并更新 anchor/cursor，不调用 `begin_edit()`。`Apply` 分支先调用 `validate_source_generation()`，再在克隆字符串上从后向前应用全部 replacement 并验证 `selection_after`；验证全部成功后才执行：

```rust
use core::document::DocViewMut as _;

doc.begin_edit();
for replacement in replacements.iter().rev() {
    doc.replace_range(replacement.range.clone(), &replacement.text);
}
doc.end_edit();

match transaction.selection_after {
    EditSelection::Caret(byte) => {
        doc.cursor_move_to_offset(byte);
        doc.cursor_mut().selection_anchor = None;
    }
    EditSelection::Range { anchor, cursor } => {
        doc.cursor_move_to_offset(cursor);
        doc.cursor_mut().selection_anchor = Some(anchor);
    }
}
```

同时把 `replace_selection_or_cursor()`、删除 fallback 改用 `EditTransaction::replace(request.source_generation, ...)`；`execute_text_replacement()` 使用 `doc.generation()` 构造单 replacement 事务。dirty line 范围使用全部 replacement 的最小 start 与最大 end 计算。

- [ ] **Step 5: 迁移 Markdown WYSIWYG 的事务构造并运行协议回归**

在 `crates/markdown/src/view.rs` 将：

```rust
EditTransaction {
    replacement: TextReplacement { range, text },
    cursor_after,
}
```

机械迁移为：

```rust
EditTransaction::replace(request.source_generation, range, text, cursor_after)
```

Run:

```bash
cargo test -p textora-ui --lib plugin::tests
cargo test -p textora-app --lib edit_transaction::tests
cargo test -p textora-markdown --lib view::tests
cargo check -p textora-app
```

Expected: 全部 PASS，`cargo check` 无错误。

- [ ] **Step 6: 格式化并提交**

```bash
cargo fmt
git add crates/ui/src/plugin.rs crates/app/src/edit_transaction.rs crates/markdown/src/view.rs
git commit -m "refactor(editor): support atomic multi-range transactions"
```

---

### Task 2: 通用语义命中与语义导航

**Files:**
- Modify: `crates/ui/src/plugin.rs:160-240`
- Modify: `crates/app/src/dispatch/mouse.rs:1-430`
- Modify: `crates/app/src/dispatch/wysiwyg.rs:70-185`

**Interfaces:**
- Produces: `EditHitTarget::{TextCaret, SourceObject}`。
- Produces: `PluginQuery::HitTestEditTarget`、`PluginQuery::MoveEditTarget` 和 `PluginResponse::EditHitTarget`。
- Produces: `apply_edit_hit_target(tab, target)`，同时通知插件的 cursor/anchor byte。
- Preserves: Markdown WYSIWYG 的 `HitTestByte` 与 `VisualMove` fallback。

- [ ] **Step 1: 写出语义目标应用测试**

在 `crates/app/src/dispatch/wysiwyg.rs` 增加测试模块：

```rust
#[cfg(test)]
mod semantic_target_tests {
    use super::*;
    use ui::plugin::EditHitTarget;

    #[test]
    fn text_target_clears_selection_and_places_caret() {
        let mut doc = DocumentView::new(vec!["abcdef".into()], 80, 10.0);
        apply_target_to_document(&mut doc, EditHitTarget::TextCaret { byte_offset: 3 });
        assert_eq!(doc.cursor_offset().to_usize(), 3);
        assert!(doc.cursor().selection_anchor.is_none());
    }

    #[test]
fn source_object_target_selects_exact_source_range() {
        let mut doc = DocumentView::new(vec!["abcdef".into()], 80, 10.0);
        apply_target_to_document(&mut doc, EditHitTarget::SourceObject { source_range: 1..5 });
    assert_eq!(doc.selection_range(), Some((1, 5)));
}

#[test]
fn clear_focus_moves_cursor_outside_titles_and_clears_selection() {
    let mut doc = DocumentView::new(vec!["abcdef".into()], 80, 10.0);
    doc.cursor_move_to_offset(3);
    doc.cursor_mut().selection_anchor = Some(1);
    apply_target_to_document(&mut doc, EditHitTarget::ClearFocus);
    assert_eq!(doc.cursor_offset().to_usize(), doc.buffer_len());
    assert!(doc.cursor().selection_anchor.is_none());
}
}
```

- [ ] **Step 2: 运行测试确认缺少通用目标类型**

```bash
cargo test -p textora-app --lib dispatch::wysiwyg::semantic_target_tests
```

Expected: FAIL，缺少 `EditHitTarget` 和 `apply_target_to_document`。

- [ ] **Step 3: 扩展纯数据插件协议**

在 `crates/ui/src/plugin.rs` 增加：

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditHitTarget {
    TextCaret { byte_offset: usize },
    SourceObject { source_range: std::ops::Range<usize> },
    ClearFocus,
}
```

并增加查询/响应：

```rust
PluginQuery::HitTestEditTarget { x, y, offset_x, offset_y }
PluginQuery::MoveEditTarget { current_byte, direction, target_x }
PluginResponse::EditHitTarget(Option<EditHitTarget>)
```

这些类型不得引用 app 或 markdown 类型。

- [ ] **Step 4: 在 app 中统一应用语义目标**

在 `crates/app/src/dispatch/wysiwyg.rs` 实现：

```rust
pub(crate) fn apply_target_to_document(doc: &mut DocumentView, target: EditHitTarget) {
    match target {
        EditHitTarget::TextCaret { byte_offset } => {
            doc.cursor_move_to_offset(byte_offset);
            doc.cursor_mut().selection_anchor = None;
        }
        EditHitTarget::SourceObject { source_range } => {
            doc.cursor_move_to_offset(source_range.end);
            doc.cursor_mut().selection_anchor = Some(source_range.start);
        }
        EditHitTarget::ClearFocus => {
            doc.cursor_move_to_offset(doc.buffer_len());
            doc.cursor_mut().selection_anchor = None;
        }
    }
}

pub(crate) fn apply_edit_hit_target(tab: &mut DocItem, target: EditHitTarget) {
    apply_target_to_document(&mut tab.doc, target);
    let cursor = tab.doc.cursor_offset().to_usize();
    let anchor = tab.doc.cursor().selection_anchor;
    tab.plugin.handle_message(PluginMessage::SetSelAnchorByte(anchor), &mut tab.doc);
    tab.plugin.handle_message(PluginMessage::SetSelCursorByte(Some(cursor)), &mut tab.doc);
    tab.plugin.handle_message(PluginMessage::SetCursorByte(cursor), &mut tab.doc);
}
```

`dispatch_wysiwyg_navigation()` 先查询 `MoveEditTarget`；收到 `EditHitTarget` 时调用该 helper，收到 `None` 时再走现有 `VisualMove` 字节 fallback。

- [ ] **Step 5: 让鼠标优先使用语义命中并保留 Markdown fallback**

在 `crates/app/src/dispatch/mouse.rs` 的自绘编辑器按下路径中先查询：

```rust
let semantic_target = match tab.plugin.query(
    PluginQuery::HitTestEditTarget { x: px, y: py, offset_x, offset_y },
    &tab.doc,
) {
    PluginResponse::EditHitTarget(target) => target,
    _ => None,
};
```

鼠标按下时，`PluginResponse::EditHitTarget(Some(target))` 调用 `apply_edit_hit_target()`；`EditHitTarget(None)` 表示插件支持语义命中但当前禁止交互，直接消费；只有 `PluginResponse::None` 才继续使用当前两阶段 `HitTestByte`，保证 Markdown WYSIWYG 行为不变。拖动开始后只接受 `TextCaret { byte_offset }`，并调用现有 `set_wysiwyg_cursor_and_selection(tab, byte_offset, original_anchor)` 保持按下时的标题 anchor；`SourceObject` 与 `ClearFocus` 在拖动阶段忽略，不扩展为跨节点选择。

- [ ] **Step 6: 运行回归并提交**

```bash
cargo test -p textora-app --lib dispatch::wysiwyg
cargo test -p textora-app --lib -- mouse
cargo test -p textora-markdown --lib view::tests
cargo check -p textora-app
cargo fmt
git add crates/ui/src/plugin.rs crates/app/src/dispatch/mouse.rs crates/app/src/dispatch/wysiwyg.rs
git commit -m "feat(editor): add semantic edit targets"
```

Expected: 所有命令通过；Markdown 点击和导航回归不变。

---

### Task 3: 将结构快捷键映射为事务意图

**Files:**
- Modify: `crates/ui/src/plugin.rs:110-130,283-330,405-420`
- Modify: `crates/app/src/events.rs:45-85`

**Interfaces:**
- Produces: `EditIntent::{PromoteObject, DemoteObject, SelectObject}`；`SelectObject` 表示 Escape 从标题编辑切换到整节点选择。
- Produces: `KeyIntentMapper::map_key()` 和 `ViewPlugin::key_intent_mapper()`。
- Preserves: 非 mmap 插件的 `Cmd+[` / `Cmd+]` 继续映射 NavigateBack/NavigateForward。

- [ ] **Step 1: 在 UI 协议测试中写按键意图 mapper 的失败测试**

在 `crates/ui/src/plugin.rs` 测试模块加入一个 stub：

```rust
struct StructuralKeyMapper;

impl KeyIntentMapper for StructuralKeyMapper {
    fn map_key(&self, key: &KeyCode, modifiers: &Modifiers) -> Option<EditIntent> {
        match (key, modifiers.cmd || modifiers.ctrl) {
            (KeyCode::Char('['), true) => Some(EditIntent::PromoteObject),
            (KeyCode::Char(']'), true) => Some(EditIntent::DemoteObject),
            _ => None,
        }
    }
}

#[test]
fn structural_key_mapper_returns_transactional_intent() {
    let modifiers = Modifiers { cmd: true, ..Modifiers::NONE };
    assert_eq!(
        StructuralKeyMapper.map_key(&KeyCode::Char(']'), &modifiers),
        Some(EditIntent::DemoteObject)
    );
}
```

- [ ] **Step 2: 运行测试确认协议缺失**

```bash
cargo test -p textora-ui --lib plugin::tests::structural_key_mapper_returns_transactional_intent
```

Expected: FAIL，缺少 `KeyIntentMapper` 或 `EditIntent` 变体。

- [ ] **Step 3: 定义只返回意图、不写文档的按键协议**

在 `crates/ui/src/plugin.rs` 增加：

```rust
pub trait KeyIntentMapper {
    fn map_key(
        &self,
        key: &crate::core::widget::KeyCode,
        modifiers: &crate::core::widget::Modifiers,
    ) -> Option<EditIntent>;
}
```

`EditIntent` 增加 `PromoteObject`、`DemoteObject`、`SelectObject`；`ViewPlugin` 增加默认返回 `None` 的 `key_intent_mapper()`。保留旧 `KeyInterceptor` 到 Task 7，避免中间提交破坏编译。

- [ ] **Step 4: 在全局快捷键映射前执行插件事务意图**

在 `crates/app/src/events.rs` 中，取得 `KeyCode`/`Modifiers` 后先只读查询 mapper：

```rust
let mapped_intent = app.workspace.active_entry().and_then(|tab| {
    tab.plugin
        .key_intent_mapper()
        .and_then(|mapper| mapper.map_key(&key_code, &modifiers))
});
if let Some(intent) = mapped_intent {
    let effect = app.dispatch_transactional_edit(intent, None);
    app.apply_effect(effect);
    return Vec::new();
}
```

把当前提前执行的 `let fallback_cmd = key_to_command(...)` 移到该分支之后。这样只有实现 mapper 的插件覆盖 `Cmd+[` / `Cmd+]`，其他插件仍由 `key_to_command()` 产生 NavigateBack/NavigateForward。

- [ ] **Step 5: 编译、回归现有快捷键并提交**

```bash
cargo test -p textora-ui --lib plugin::tests
cargo test -p textora-app --lib input::tests
cargo test -p textora-app --lib app_lifecycle::tests
cargo check -p textora-app
cargo fmt
git add crates/ui/src/plugin.rs crates/app/src/events.rs
git commit -m "refactor(editor): route plugin structural keys through edit intents"
```

Expected: mmap 尚未启用 mapper；其他插件的历史导航快捷键保持原行为。

---

### Task 4: 精确 MMF 源码范围与解析诊断

**Files:**
- Modify: `crates/markdown/src/mmf/model.rs:1-50`
- Modify: `crates/markdown/src/mmf/parser.rs:1-330`
- Modify: `crates/markdown/src/mmf/canvas.rs:390-420`（更新 Node 测试构造器）

**Interfaces:**
- Produces: `Node::{heading_marker_range, child_insertion_byte, subtree_source_range}`。
- Produces: `MmfDiagnostic { kind, line, column, message }`。
- Preserves: `Node::source_range` 暂时作为 `subtree_source_range` 的兼容别名，Task 6 删除旧字段。

- [ ] **Step 1: 写空标题、代码块井号和结构范围失败测试**

在 `crates/markdown/src/mmf/parser.rs` 增加：

```rust
#[test]
fn empty_heading_keeps_zero_length_title_range() {
    let source = "# Root\n##\n";
    let child = &parse(source).expect("empty title is valid").root.children[0];
    assert_eq!(child.title, "");
    assert_eq!(child.title_byte_range.start, child.title_byte_range.end);
    assert_eq!(&source[child.heading_marker_range.clone()], "##");
}

#[test]
fn fenced_hash_line_is_note_content_not_a_child() {
    let source = "# Root\n\n```text\n## not a node\n```\n\n## Child\n";
    let tree = parse(source).expect("parse fenced note");
    assert_eq!(tree.root.children.len(), 1);
    assert_eq!(tree.root.children[0].title, "Child");
}

#[test]
fn node_ranges_distinguish_child_insertion_and_subtree_end() {
    let source = "# Root\nroot note\n## Parent\nparent note\n### Existing\nchild note\n## Next\n";
    let tree = parse(source).expect("parse ranges");
    let parent = &tree.root.children[0];
    assert_eq!(parent.child_insertion_byte, source.find("### Existing").unwrap());
    assert_eq!(parent.subtree_source_range.end, source.find("## Next").unwrap());
    assert_eq!(&source[parent.heading_marker_range.clone()], "##");
}

#[test]
fn non_level_one_root_reports_source_location() {
    let diagnostic = parse("## NotRoot\n").expect_err("root must use one heading marker");
    assert_eq!(diagnostic.kind, ParseErrorKind::HeadingLevelSkip);
    assert_eq!((diagnostic.line, diagnostic.column), (1, 1));
}
```

- [ ] **Step 2: 运行解析测试并确认失败**

```bash
cargo test -p textora-markdown --lib mmf::parser::tests
```

Expected: FAIL，Node 缺少新范围；代码块中的标题可能被错误解析。

- [ ] **Step 3: 用类型表达精确范围和诊断**

在 `model.rs` 增加：

```rust
#[derive(Debug, Clone)]
pub struct Node {
    pub title: String,
    pub children: Vec<Node>,
    pub props: Option<NodeProps>,
    pub note: Option<String>,
    pub source_range: Range<usize>,
    pub subtree_source_range: Range<usize>,
    pub title_byte_range: Range<usize>,
    pub heading_marker_range: Range<usize>,
    pub child_insertion_byte: usize,
    pub heading_level: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MmfDiagnostic {
    pub kind: ParseErrorKind,
    pub line: usize,
    pub column: usize,
    pub message: String,
}
```

`ParseErrorKind` 包含 `EmptyDocument`、`MultipleRoots`、`InvalidToml`、`HeadingLevelSkip`。所有解析错误通过构造函数填入 1-based 行列和稳定消息。

- [ ] **Step 4: 在解析器中计算 marker、插入锚点和子树范围**

给 `HeadingInfo` 增加：

```rust
heading_marker_range: Range<usize>,
line_number: usize,
```

`peek_heading()` 用 `source_start + leading_space_count` 计算标记范围。节点关闭时设置 `source_range` 与 `subtree_source_range`；整棵树构建完成后递归计算：

```rust
fn assign_child_insertion_bytes(node: &mut Node) {
    node.child_insertion_byte = node
        .children
        .first()
        .map(|child| child.subtree_source_range.start)
        .unwrap_or(node.subtree_source_range.end);
    for child in &mut node.children {
        assign_child_insertion_bytes(child);
    }
}
```

普通 fenced code block 内禁用 heading 探测，直到匹配关闭 fence；`toml node` 和 `toml mindmap` 继续走现有专用解析。

- [ ] **Step 5: 更新 canvas 测试 Node 构造并运行回归**

所有测试 Node 明确补充：

```rust
subtree_source_range: 0..0,
heading_marker_range: 0..0,
child_insertion_byte: 0,
```

Run:

```bash
cargo test -p textora-markdown --lib mmf::parser::tests
cargo test -p textora-markdown --lib mmf::canvas::tests
cargo check -p textora-markdown
```

Expected: 全部 PASS。

- [ ] **Step 6: 格式化并提交**

```bash
cargo fmt
git add crates/markdown/src/mmf/model.rs crates/markdown/src/mmf/parser.rs crates/markdown/src/mmf/canvas.rs
git commit -m "feat(markdown): track precise mmap source ranges"
```

---

### Task 5: grapheme 命中几何与真实视口裁剪

**Files:**
- Modify: `crates/markdown/src/grapheme_map.rs:1-125`
- Modify: `crates/markdown/src/mmf/layout.rs:50-245`
- Modify: `crates/markdown/src/mmf/canvas.rs:330-700`

**Interfaces:**
- Produces: `NodeHitGeometry` 和 `HitMap { nodes }`，grapheme 字节边界与 x 边缘一一对应。
- Produces: `LayoutTree::visible_node_indices(viewport, buffer)`。
- Replaces: `title_char_edges` 与当前返回全范围的 `visible_range()`。

- [ ] **Step 1: 写 Unicode 命中边界和裁剪失败测试**

在 `layout.rs` 测试中加入：

```rust
#[test]
fn hit_geometry_uses_grapheme_byte_boundaries() {
    let tree = parser::parse("# A👨\u{200D}👩\u{200D}👧中\n").expect("parse");
    let mut shaper = Shaper::new().expect("shaper");
    let constants = LayoutConstants::default();
    let layout = compute_layout(&tree, &mut shaper, &constants);
    let hit_map = build_hit_map(&tree, &layout, &mut shaper, &constants);
    let geometry = &hit_map.nodes[0];

    assert_eq!(geometry.grapheme_byte_offsets.first(), Some(&0));
    assert_eq!(geometry.grapheme_byte_offsets.last(), Some(&tree.root.title.len()));
    assert_eq!(geometry.grapheme_byte_offsets.len(), geometry.grapheme_edges.len());
}

#[test]
fn visible_indices_exclude_nodes_outside_viewport() {
    let tree = dummy_tree();
    let mut shaper = Shaper::new().expect("shaper");
    let layout = compute_layout(&tree, &mut shaper, &LayoutConstants::default());
    let visible = layout.visible_node_indices(Rect::new(0.0, 0.0, 400.0, 50.0), 0.0);
    assert!(visible.len() < layout.nodes.len());
}
```

- [ ] **Step 2: 运行测试确认现有 char 模型和全量裁剪失败**

```bash
cargo test -p textora-markdown --lib mmf::layout::tests
```

Expected: FAIL，缺少 `NodeHitGeometry`、grapheme offsets 或真实可见索引。

- [ ] **Step 3: 实现 grapheme 几何**

在 `layout.rs` 定义：

```rust
pub struct NodeHitGeometry {
    pub card_rect: Rect,
    pub title_rect: Rect,
    pub grapheme_byte_offsets: Vec<usize>,
    pub grapheme_edges: Vec<f32>,
    pub title_byte_range: Range<usize>,
    pub subtree_source_range: Range<usize>,
}

pub struct HitMap {
    pub nodes: Vec<NodeHitGeometry>,
}
```

先在 `grapheme_map.rs` 从现有 UAX#29 状态机提取通用 helper：

```rust
pub(crate) fn grapheme_byte_boundaries(text: &str) -> Vec<usize> {
    let source_bytes_by_char: Vec<usize> = text
        .char_indices()
        .map(|(byte, _)| byte)
        .chain(std::iter::once(text.len()))
        .collect();
    build_visual_grapheme_map(text, &source_bytes_by_char)
        .as_slice()
        .to_vec()
}
```

`layout.rs` 使用该 helper 枚举标题 grapheme 边界；每个相邻字节范围调用 `shaper.grapheme_advance(&title[start..end])` 累加 x。边界数组必须包含起点和末尾 sentinel，空标题为 `[0]` 和单个文本起始 x。

- [ ] **Step 4: 增加按 y 排序索引并替换全量渲染范围**

`LayoutTree` 增加：

```rust
pub y_sorted_indices: Vec<usize>,
```

`compute_layout()` 完成后按 `(node.y, node.x)` 稳定排序。实现：

```rust
pub fn visible_node_indices(&self, viewport: Rect, buffer: f32) -> Vec<usize> {
    let top = viewport.y - buffer;
    let bottom = viewport.y + viewport.h + buffer;
    self.y_sorted_indices
        .iter()
        .copied()
        .filter(|&index| {
            let node = &self.nodes[index];
            node.y + node.h >= top && node.y <= bottom
        })
        .collect()
}
```

`canvas.rs` 的卡片、连线和文字渲染函数改接收 `&[usize]`，按索引访问 DFS 节点与布局节点，不再假设一个连续区间。

- [ ] **Step 5: 运行布局与 canvas 测试并提交**

```bash
cargo test -p textora-markdown --lib mmf::layout::tests
cargo test -p textora-markdown --lib mmf::canvas::tests
cargo test -p textora-markdown --lib grapheme_map::tests
cargo check -p textora-markdown
cargo fmt
git add crates/markdown/src/grapheme_map.rs crates/markdown/src/mmf/layout.rs crates/markdown/src/mmf/canvas.rs
git commit -m "fix(markdown): use grapheme mmap hit geometry"
```

Expected: Unicode 和裁剪测试 PASS，现有主题/连线测试继续通过。

---

### Task 6: 纯 MMF 编辑策略

**Files:**
- Modify: `crates/markdown/src/mmf/edit.rs:1-250`

**Interfaces:**
- Produces: `plan_mindmap_edit(tree, source, request) -> EditPlan`。
- Produces: `plan_new_sibling`、`plan_new_child`、`plan_promote_subtree`、`plan_demote_subtree`、`plan_delete_subtree`。
- Produces: 纯 planner；旧 `handle_intercept_key` 写文档路径只保留到 Task 7，确保本任务提交可独立编译。

- [ ] **Step 1: 用纯字符串输入写完整结构编辑失败测试**

将 `edit.rs` 测试改为直接断言 `EditPlan`，至少包含：

```rust
fn request(selection: Range<usize>, intent: EditIntent) -> EditRequest {
    EditRequest {
        source_generation: 1,
        cursor_byte: selection.end,
        selection: Some(selection),
        intent,
    }
}

fn request_at(cursor_byte: usize, intent: EditIntent) -> EditRequest {
    EditRequest { source_generation: 1, cursor_byte, selection: None, intent }
}

fn assert_transaction_inserts(plan: EditPlan, byte: usize, expected_text: &str) {
    let EditPlan::Apply(transaction) = plan else {
        panic!("expected apply transaction");
    };
    assert_eq!(transaction.replacements.len(), 1);
    assert_eq!(transaction.replacements[0].range, byte..byte);
    assert_eq!(transaction.replacements[0].text, expected_text);
}

fn assert_deletes_range(plan: EditPlan, expected_range: Range<usize>) {
    let EditPlan::Apply(transaction) = plan else {
        panic!("expected apply transaction");
    };
    assert_eq!(transaction.replacements.len(), 1);
    assert_eq!(transaction.replacements[0].range, expected_range);
    assert!(transaction.replacements[0].text.is_empty());
}

#[test]
fn selected_node_typing_replaces_only_title() {
    let source = "# Root\n## Parent\n### Child\n";
    let tree = parser::parse(source).expect("parse");
    let parent = &tree.root.children[0];
    let request = request(
        parent.subtree_source_range.clone(),
        EditIntent::InsertText("Renamed".into()),
    );

    assert_eq!(
        plan_mindmap_edit(&tree, source, &request),
        EditPlan::Apply(EditTransaction::replace(
            request.source_generation,
            parent.title_byte_range.clone(),
            "Renamed".into(),
            parent.title_byte_range.start + "Renamed".len(),
        ))
    );
}

#[test]
fn tab_inserts_empty_child_at_child_insertion_byte() {
    let source = "# Root\n## Parent\nparent note\n### Existing\n";
    let tree = parser::parse(source).expect("parse");
    let parent = &tree.root.children[0];
    let request = request_at(parent.title_byte_range.end, EditIntent::Indent);
    let plan = plan_mindmap_edit(&tree, source, &request);
    assert_transaction_inserts(plan, parent.child_insertion_byte, "###\n");
}

#[test]
fn demote_changes_current_subtree_markers_as_one_transaction() {
    let source = "# Root\n## First\n## Second\n### Leaf\n";
    let tree = parser::parse(source).expect("parse");
    let second = &tree.root.children[1];
    let request = request_at(second.title_byte_range.start, EditIntent::DemoteObject);
    let EditPlan::Apply(transaction) = plan_mindmap_edit(&tree, source, &request) else {
        panic!("demote should return a transaction");
    };
    assert_eq!(transaction.replacements.len(), 2);
    assert!(transaction.replacements.iter().all(|replacement| replacement.text == "#"));
}

#[test]
fn selected_parent_delete_removes_whole_subtree() {
    let source = "# Root\n## Parent\n### Child\n## Next\n";
    let tree = parser::parse(source).expect("parse");
    let parent = &tree.root.children[0];
    let request = request(parent.subtree_source_range.clone(), EditIntent::DeleteForward);
    assert_deletes_range(plan_mindmap_edit(&tree, source, &request), parent.subtree_source_range.clone());
}
```

同时覆盖根节点不可删/升降、无前一同级不可降级、编辑态删除不越过标题、Enter 新建同级和换行风格保留。
再增加两个无文本修改断言：整节点选中 + `InsertParagraphBreak` 返回 `SetSelection(Caret(title.end))`；标题编辑 + `SelectObject` 返回 `SetSelection(Range { anchor: subtree.start, cursor: subtree.end })`。

- [ ] **Step 2: 运行测试确认旧代码依赖 DocViewMut 且语义不符**

```bash
cargo test -p textora-markdown --lib mmf::edit::tests
```

Expected: FAIL，缺少纯 planner，或旧实现写入“新节点”而不是空标题。

- [ ] **Step 3: 实现焦点节点匹配和单 replacement 操作**

核心匹配函数：

```rust
fn focused_node<'a>(tree: &'a Tree, request: &EditRequest) -> Option<&'a Node> {
    let nodes = collect_nodes_dfs(&tree.root);
    if let Some(selection) = &request.selection {
        return nodes.into_iter().find(|node| node.subtree_source_range == *selection);
    }
    nodes.into_iter().find(|node| {
        request.cursor_byte >= node.title_byte_range.start
            && request.cursor_byte <= node.title_byte_range.end
    })
}
```

整节点输入替换标题；整节点 Delete 删除 `subtree_delete_range()`；编辑态字符、Backspace/Delete 若在标题内则生成标题范围内事务，否则返回 `Consume`。编辑态 Enter 在非根节点的子树尾插入同级空标题，Tab 在 `child_insertion_byte` 插入空子节点。整节点态 Enter 只返回标题末尾 caret；编辑态 `SelectObject` 只返回当前子树 Range；两者不产生 Undo。

- [ ] **Step 4: 实现升降级多 replacement**

降级必须先通过 `find_siblings()` 确认存在前一个同级；随后对当前节点 DFS 子树生成：

```rust
TextReplacement {
    range: node.heading_marker_range.end..node.heading_marker_range.end,
    text: "#".into(),
}
```

升级为每个节点删除 `heading_marker_range.end - 1..heading_marker_range.end`。事务后选择使用调整后重新映射的子树范围；标题编辑态则保持相对标题字节位置。

- [ ] **Step 5: 运行纯 planner 测试并保持中间提交可编译**

删除旧测试桩 `SimpleDoc` 和所有直接调用 `exec_*` 的测试；暂时保留被 `MindmapView::KeyInterceptor` 引用的旧函数，Task 7 在 view 切换协议的同一提交中删除它们。运行：

```bash
cargo test -p textora-markdown --lib mmf::edit::tests
cargo check -p textora-markdown
cargo fmt
git add crates/markdown/src/mmf/edit.rs
git commit -m "refactor(markdown): make mmap edits transactional"
```

Expected: 纯 planner 测试全部 PASS；新增 planner 不写 app 或 DocumentView，旧入口仅为下一任务的编译兼容层。

---

### Task 7: MindmapView 迁入统一 WYSIWYG 协议

**Files:**
- Modify: `crates/markdown/src/mindmap_view.rs:1-370`
- Modify: `crates/markdown/src/mmf/edit.rs`（删除 `exec_indent`、`exec_outdent`、`exec_new_sibling`、`exec_new_child`、`handle_intercept_key` 和 `DocViewMut` import）

**Interfaces:**
- Produces: `MindmapDocumentState::{Ready, Invalid}` 和派生的 `MindmapFocus`。
- Implements: `EditPolicy`、`KeyIntentMapper`、语义命中、语义导航、`CursorScreenPos`。
- Removes: `KeyInterceptor`、`HitTestCanvas`、`CursorRect`、`CanvasMove` 的 mmap 实现。

- [ ] **Step 1: 在 mindmap_view 中写插件能力、语义命中和焦点派生失败测试**

在 `mindmap_view.rs` 测试模块加入可复用的拥有型文档桩和构造/布局 helper：

```rust
struct MindmapTestDoc {
    text: String,
    lines: Vec<String>,
}

impl MindmapTestDoc {
    fn new(source: &str) -> Self {
        Self {
            text: source.to_owned(),
            lines: source.split('\n').map(str::to_owned).collect(),
        }
    }
}

impl DocView for MindmapTestDoc {
    fn line_count(&self) -> usize { self.lines.len() }
    fn doc_line_text(&self, line: usize) -> Cow<'_, str> {
        Cow::Borrowed(self.lines.get(line).map(String::as_str).unwrap_or(""))
    }
    fn doc_text_in_range(&self, range: Range<usize>) -> Cow<'_, str> {
        Cow::Borrowed(&self.text[range])
    }
    fn line_byte_offset(&self, line: usize) -> usize {
        self.lines.iter().take(line).map(|line| line.len() + 1).sum()
    }
    fn line_byte_length(&self, line: usize) -> usize {
        self.lines.get(line).map(String::len).unwrap_or(0)
    }
    fn scroll_y(&self) -> f32 { 0.0 }
    fn viewport_height(&self) -> f32 { 800.0 }
}

impl DocViewMut for MindmapTestDoc {
    fn set_scroll_y(&mut self, _scroll_y: f32) {}
    fn replace_range(&mut self, range: Range<usize>, text: &str) {
        self.text.replace_range(range, text);
        self.lines = self.text.split('\n').map(str::to_owned).collect();
    }
}

fn view_with_source(source: &str) -> (MindmapView, MindmapTestDoc) {
    let mut view = MindmapView::new();
    let mut doc = MindmapTestDoc::new(source);
    view.handle_message(
        PluginMessage::UpdateSource { text: source.to_owned(), generation: 1 },
        &mut doc,
    );
    (view, doc)
}

fn render_test_view(view: &mut MindmapView, doc: &MindmapTestDoc) {
    let mut shaper = Shaper::new().expect("test shaper");
    let theme = Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
    let _ = view.render(
        doc,
        Rect::new(0.0, 0.0, 1200.0, 800.0),
        &theme,
        &mut shaper,
        1.0,
    );
}

fn laid_out_child_card_padding() -> (MindmapView, MindmapTestDoc, (f32, f32)) {
    let (mut view, doc) = view_with_source("# Root\n## Child\n");
    render_test_view(&mut view, &doc);
    let geometry = &view.ready_hit_map().nodes[1];
    let point = (geometry.card_rect.x + 2.0, geometry.card_rect.y + 2.0);
    (view, doc, point)
}
```

给 `MindmapView` 增加仅供同文件测试使用的 `ready_tree()` / `ready_hit_map()` 私有 helper；非 Ready 状态用 `expect("test requires ready mmap state")` 明确失败。随后加入测试：

```rust
#[test]
fn mindmap_is_an_editable_custom_renderer() {
    let view = MindmapView::new();
    assert!(view.allows_editing());
    assert!(view.handles_own_rendering());
    assert!(!view.shows_cursor());
    assert!(view.needs_cursor_blink_wakeup());
}

#[test]
fn exact_subtree_selection_derives_node_selected_focus() {
    let source = "# Root\n## Child\n";
    let (view, _doc) = view_with_source(source);
    let child_range = view.ready_tree().root.children[0].subtree_source_range.clone();
    let focus = view.derive_focus(child_range.end, Some(child_range.clone()));
    assert!(matches!(focus, MindmapFocus::NodeSelected { node_index: 1 }));
}

#[test]
fn card_padding_hit_returns_source_object() {
    let (view, doc, point) = laid_out_child_card_padding();
    let expected = view.ready_tree().root.children[0].subtree_source_range.clone();
    let response = view.query(
        PluginQuery::HitTestEditTarget {
            x: point.0,
            y: point.1,
            offset_x: 0.0,
            offset_y: 0.0,
        },
        &doc,
    );
    assert!(matches!(
        response,
        PluginResponse::EditHitTarget(Some(EditHitTarget::SourceObject { source_range }))
            if source_range == expected
    ));
}
```

- [ ] **Step 2: 运行测试确认旧 view 仍是不可编辑画布**

```bash
cargo test -p textora-markdown --lib mindmap_view::tests
```

Expected: FAIL，`allows_editing()` 仍为 false，缺少状态枚举与语义查询。

- [ ] **Step 3: 以互斥枚举替换 Option 缓存组合**

定义：

```rust
enum MindmapDocumentState {
    Ready {
        generation: u32,
        source: String,
        tree: Tree,
        layout: Option<LayoutTree>,
        hit_map: Option<HitMap>,
    },
    Invalid {
        generation: u32,
        diagnostic: MmfDiagnostic,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum MindmapFocus {
    None,
    NodeSelected { node_index: usize },
    TitleEditing { node_index: usize, cursor_byte: usize },
    TitleTextSelected { node_index: usize, range: Range<usize> },
}
```

`UpdateSource` 成功时进入 `Ready`，失败时进入 `Invalid` 并丢弃旧布局。`SetCursorByte`、`SetSelAnchorByte`、`SetSelCursorByte`、`SetPreedit`、`SetCursorVisible` 只保存同步输入，不修改源码。

- [ ] **Step 4: 实现统一编辑与快捷键接口**

```rust
impl ui::plugin::EditPolicy for MindmapView {
    fn plan_edit(&self, request: &EditRequest) -> EditPlan {
        let MindmapDocumentState::Ready { generation, source, tree, .. } = &self.document_state
        else {
            return EditPlan::Consume;
        };
        if request.source_generation != *generation {
            return EditPlan::Consume;
        }
        mmf::edit::plan_mindmap_edit(tree, source, request)
    }
}

impl ui::plugin::KeyIntentMapper for MindmapView {
    fn map_key(&self, key: &KeyCode, modifiers: &Modifiers) -> Option<EditIntent> {
        let primary = modifiers.cmd || modifiers.ctrl;
        match (key, primary) {
            (KeyCode::Char('['), true) => Some(EditIntent::PromoteObject),
            (KeyCode::Char(']'), true) => Some(EditIntent::DemoteObject),
            (KeyCode::Escape, false) => Some(EditIntent::SelectObject),
            _ => None,
        }
    }
}
```

`ViewPlugin` 返回上述 policy/mapper，并设置可编辑自绘能力。删除 `KeyInterceptor` 实现。

- [ ] **Step 5: 实现语义命中、导航和光标屏幕矩形**

- 标题矩形命中：用 `grapheme_edges` 找最近边界，通过 `title_byte_range.start + grapheme_byte_offsets[index]` 返回 `TextCaret`。
- 卡片其余区域：返回 `SourceObject { subtree_source_range }`。
- Ready 画布空白：返回 `ClearFocus`；Invalid 状态返回 `EditHitTarget(None)`，阻止 app 回落到 Markdown 字节命中。
- `MoveEditTarget` 根据 `MindmapFocus` 实现已确认导航模型；节点态方向键返回 SourceObject，标题态左右返回 TextCaret，标题态上下返回相邻节点 SourceObject。
- `CursorScreenPos` 从同一 `NodeHitGeometry`、scroll 和 render bounds 计算 `(x, y, width, height)`。

- [ ] **Step 6: 运行 view、编辑和 app 编译回归并提交**

```bash
cargo test -p textora-markdown --lib mindmap_view::tests
cargo test -p textora-markdown --lib mmf::edit::tests
cargo check -p textora-app
cargo fmt
git add crates/markdown/src/mindmap_view.rs crates/markdown/src/mmf/edit.rs
git commit -m "feat(markdown): enable mmap wysiwyg editing"
```

Expected: mmap 使用新协议；旧直接写文档与旧 Canvas 查询不再被 mmap 引用。

---

### Task 8: 焦点、占位符、IME、错误态与最终集成

**Files:**
- Modify: `crates/markdown/src/mmf/layout.rs`
- Modify: `crates/markdown/src/mmf/canvas.rs`
- Modify: `crates/markdown/src/mindmap_view.rs`

**Interfaces:**
- Produces: `MindmapRenderProjection`，将焦点、标题选择、预编辑文字和闪烁相位传入 layout/canvas。
- Produces: 空标题占位符与 Invalid 错误画布。
- Completes: 同一几何来源下的绘制、命中、光标和 IME 候选窗定位。

- [ ] **Step 1: 写空标题、整卡选择、IME 投影和错误态失败测试**

在 `mindmap_view.rs` 使用 Task 7 的 `view_with_source()` / `render_test_view()` helper 加入：

```rust
#[test]
fn empty_title_projects_placeholder_without_changing_source_range() {
    let (mut view, mut doc) = view_with_source("# Root\n##\n");
    render_test_view(&mut view, &doc);
    let empty_title_byte = view.ready_tree().root.children[0].title_byte_range.start;
    view.handle_message(PluginMessage::SetCursorByte(empty_title_byte), &mut doc);

    let projection = view.build_render_projection();
    assert_eq!(projection.projected_title(1), EMPTY_TITLE_PLACEHOLDER);
    let title_range = &view.ready_tree().root.children[0].title_byte_range;
    assert_eq!(title_range.start, title_range.end);
}

#[test]
fn node_selection_projects_card_highlight_without_text_caret() {
    let (mut view, mut doc) = view_with_source("# Root\n## Child\n");
    let range = view.ready_tree().root.children[0].subtree_source_range.clone();
    view.handle_message(PluginMessage::SetSelAnchorByte(Some(range.start)), &mut doc);
    view.handle_message(PluginMessage::SetSelCursorByte(Some(range.end)), &mut doc);
    view.handle_message(PluginMessage::SetCursorByte(range.end), &mut doc);

    let projection = view.build_render_projection();
    assert_eq!(projection.selected_node_index(), Some(1));
    assert!(projection.caret().is_none());
}

#[test]
fn selected_node_preedit_replaces_visual_title() {
    let (mut view, mut doc) = view_with_source("# Root\n## Original\n");
    let range = view.ready_tree().root.children[0].subtree_source_range.clone();
    view.handle_message(PluginMessage::SetSelAnchorByte(Some(range.start)), &mut doc);
    view.handle_message(PluginMessage::SetSelCursorByte(Some(range.end)), &mut doc);
    view.handle_message(
        PluginMessage::SetPreedit { text: "ni".into(), cursor: Some((2, 2)) },
        &mut doc,
    );

    assert_eq!(view.build_render_projection().projected_title(1), "ni");
}

#[test]
fn invalid_source_discards_previous_layout() {
    let (mut view, mut doc) = view_with_source("# Root\n");
    render_test_view(&mut view, &doc);
    view.handle_message(
        PluginMessage::UpdateSource { text: String::new(), generation: 2 },
        &mut doc,
    );
    assert!(matches!(view.document_state, MindmapDocumentState::Invalid { .. }));
}
```

在 `canvas.rs` 另加一个稳定绘制断言：向 `render_cards_and_connectors()` 传入 `selected_node_index = Some(1)` 后，节点 1 的最后一个 `StrokeRect` 使用 `theme.mindmap.node.selected_border`；传入 `caret = None` 时不产生 caret `FillRect`。

- [ ] **Step 2: 运行测试确认当前 canvas 没有焦点和 preedit 投影**

```bash
cargo test -p textora-markdown --lib mmf::canvas::tests
cargo test -p textora-markdown --lib mindmap_view::tests
```

Expected: FAIL，缺少 placeholder、选中态、预编辑投影或 Invalid 渲染。

- [ ] **Step 3: 定义渲染投影并让布局测量预编辑标题**

在 `layout.rs` 定义：

```rust
pub struct ProjectedTitle<'a> {
    pub node_index: usize,
    pub text: &'a str,
}
```

`compute_layout()` 和 `build_hit_map()` 接收 `Option<ProjectedTitle<'_>>`；活动节点使用投影文字测量卡片宽度，但源码范围仍来自原 Node。整节点选中 + preedit 使用纯 preedit 文本，标题编辑 + preedit 使用 `标题前缀 + preedit + 标题后缀`。

- [ ] **Step 4: 渲染互斥焦点状态**

在 `canvas.rs` 定义：

```rust
pub const EMPTY_TITLE_PLACEHOLDER: &str = "输入主题";

pub struct MindmapRenderProjection<'a> {
    pub focus: &'a MindmapFocus,
    pub projected_titles: Vec<Cow<'a, str>>,
    pub preedit_text: &'a str,
    pub preedit_cursor: Option<(usize, usize)>,
    pub cursor_visible: bool,
    pub caret: Option<(usize, usize)>,
}

impl MindmapRenderProjection<'_> {
    pub fn projected_title(&self, node_index: usize) -> &str {
        self.projected_titles[node_index].as_ref()
    }

    pub fn selected_node_index(&self) -> Option<usize> {
        match self.focus {
            MindmapFocus::NodeSelected { node_index } => Some(*node_index),
            _ => None,
        }
    }

    pub fn caret(&self) -> Option<(usize, usize)> {
        self.caret
    }
}
```

渲染顺序固定为连接线、卡片、整卡选中、标题选择、标题/placeholder/preedit、光标。`NodeSelected` 不画文本 caret；`TitleEditing` 只在 `cursor_visible` 时画 caret；placeholder 使用主题弱化前景色且不参与源码字节计算。

- [ ] **Step 5: 渲染 Invalid 状态并完成 IME 矩形一致性**

`MindmapView::render()` 遇到 `Invalid` 时只绘制画布背景、诊断 message、`line:column` 和“使用视图切换按钮进入源码修复”，不复用旧 Tree/Layout。`CursorScreenPos` 在 preedit 存在时使用投影几何和 preedit cursor，确保返回 caret 与实际绘制位置一致。

- [ ] **Step 6: 运行 markdown 全量测试并提交**

```bash
cargo test -p textora-markdown --lib mmf
cargo test -p textora-markdown --lib mindmap_view::tests
cargo check -p textora-app
cargo fmt
git add crates/markdown/src/mmf/layout.rs crates/markdown/src/mmf/canvas.rs crates/markdown/src/mindmap_view.rs
git commit -m "feat(markdown): render mmap editing states"
```

Expected: mmap parser/edit/layout/canvas/view 测试全部 PASS，app 编译通过。

---

### Task 9: app 端到端回归与全面验证

**Files:**
- Modify: `crates/app/src/app_tests.rs:1660-3020`
- Modify: `test_data/sample.mmap.md`（仅在缺少空标题、属性、备注和三层子树样例时扩充）

**Interfaces:**
- Consumes: Tasks 1-8 的最终插件协议和 mmap 实现。
- Produces: 可复现的 app 集成回归与人工验证样例。

- [ ] **Step 1: 增加真实 MindmapView 的 app 集成测试**

在 `app_tests.rs` 增加构造和选择 helper：

```rust
#[cfg(feature = "markdown")]
fn app_with_mmap_source(source: &str) -> App {
    let mut app = App::new(None);
    let doc = DocumentView::new(source.split('\n').map(str::to_owned).collect(), 80, 10.0);
    app.workspace.push_entry_for_test(DocItem::new(
        doc,
        Box::new(textora_markdown::mindmap_view::MindmapView::new()),
    ));
    let _ = app.workspace.switch_to(0);
    app.sync_plugin_state();
    render_active_wysiwyg_plugin_for_test(&mut app);
    app
}

#[cfg(feature = "markdown")]
fn select_mmap_source_object(app: &mut App, source_range: Range<usize>) {
    let tab = app.workspace.active_entry_mut().expect("active mmap tab");
    tab.doc.cursor_move_to_offset(source_range.end);
    tab.doc.cursor_mut().selection_anchor = Some(source_range.start);
    app.sync_plugin_state();
}

#[cfg(feature = "markdown")]
fn undo_active_mmap_edit(app: &mut App) {
    let tab = app.workspace.active_entry_mut().expect("active mmap tab");
    let _ = execute_edit_command_v2(&EditCommand::Undo, &mut tab.doc, &[]);
    app.sync_plugin_state();
}
```

随后加入以下真实测试体：

```rust
#[test]
#[cfg(feature = "markdown")]
fn mmap_selected_node_typing_replaces_title_then_undo_restores_it() {
    let source = "# Root\n## Parent\n### Child\n## Next\n";
    let tree = textora_markdown::mmf::parser::parse(source).expect("parse fixture");
    let parent_range = tree.root.children[0].subtree_source_range.clone();
    let mut app = app_with_mmap_source(source);
    select_mmap_source_object(&mut app, parent_range);

    app.dispatch_transactional_edit_for_test(EditCommand::InsertChar("Renamed".into()));
    assert_eq!(
        app.workspace.active_doc().expect("active document").full_text(),
        "# Root\n## Renamed\n### Child\n## Next\n"
    );

    undo_active_mmap_edit(&mut app);
    assert_eq!(app.workspace.active_doc().expect("active document").full_text(), source);
}

#[test]
#[cfg(feature = "markdown")]
fn mmap_tab_creates_empty_child_and_enter_creates_empty_sibling() {
    let source = "# Root\n## Parent\n";
    let tree = textora_markdown::mmf::parser::parse(source).expect("parse fixture");
    let mut app = app_with_mmap_source(source);
    {
        let tab = app.workspace.active_entry_mut().expect("active mmap tab");
        tab.doc.cursor_move_to_offset(tree.root.children[0].title_byte_range.end);
        tab.doc.cursor_mut().selection_anchor = None;
    }
    app.sync_plugin_state();

    app.dispatch_transactional_edit_for_test(EditCommand::Tab);
    let after_child = app.workspace.active_doc().expect("active document").full_text();
    assert!(after_child.contains("###\n"));

    app.dispatch_transactional_edit_for_test(EditCommand::InsertNewline);
    let after_sibling = app.workspace.active_doc().expect("active document").full_text();
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
    assert_eq!(
        app.workspace.active_doc().expect("active document").full_text(),
        "# Root\n## First\n### Second\n#### Leaf\n"
    );

    undo_active_mmap_edit(&mut app);
    assert_eq!(app.workspace.active_doc().expect("active document").full_text(), source);
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
    assert_eq!(
        app.workspace.active_doc().expect("active document").full_text(),
        "# Root\n## Next\n"
    );

    undo_active_mmap_edit(&mut app);
    assert_eq!(app.workspace.active_doc().expect("active document").full_text(), source);
}

#[test]
#[cfg(feature = "markdown")]
fn mmap_preedit_does_not_change_document_until_commit() {
    let source = "# Root\n## Original\n";
    let tree = textora_markdown::mmf::parser::parse(source).expect("parse fixture");
    let selected_range = tree.root.children[0].subtree_source_range.clone();
    let mut app = app_with_mmap_source(source);
    select_mmap_source_object(&mut app, selected_range);
    app.preedit_text = "ni".into();
    app.preedit_cursor = Some((2, 2));
    app.sync_plugin_state();

    assert_eq!(app.workspace.active_doc().expect("active document").full_text(), source);

    app.dispatch_transactional_edit_for_test(EditCommand::InsertChar("你".into()));
    assert_eq!(
        app.workspace.active_doc().expect("active document").full_text(),
        "# Root\n## 你\n"
    );
}

#[test]
#[cfg(feature = "markdown")]
fn mmap_invalid_source_exposes_no_edit_target() {
    let mut app = app_with_mmap_source("# Root\n");
    {
        let tab = app.workspace.active_entry_mut().expect("active mmap tab");
        tab.doc.select_all();
        let _ = execute_edit_command_v2(&EditCommand::InsertText(String::new()), &mut tab.doc, &[]);
    }
    app.sync_plugin_state();

    let tab = app.workspace.active_entry().expect("active mmap tab");
    assert!(matches!(
        tab.plugin.query(
            ui::plugin::PluginQuery::HitTestEditTarget {
                x: 10.0,
                y: 10.0,
                offset_x: 0.0,
                offset_y: 0.0,
            },
            &tab.doc,
        ),
        ui::plugin::PluginResponse::EditHitTarget(None)
    ));
}
```

这些测试必须使用真实 `MindmapView`，断言 `DocumentView::full_text()`、选择区间、generation 和 Undo/Redo，而不是只检查 mock 调用次数。鼠标坐标到语义目标的精确映射已经分别由 Task 2 的 app 测试与 Task 7 的真实 mmap 命中测试覆盖，避免在端到端层重复脆弱的像素 fixture。

- [ ] **Step 2: 运行新增集成测试并修复仅限测试暴露的接线问题**

```bash
cargo test -p textora-app --lib -- mmap_
```

Expected: 新增测试全部 PASS。若失败，只修正 Tasks 1-8 范围内的协议接线；不得增加 mmap 专用 app 状态。

- [ ] **Step 3: 运行分层回归**

```bash
cargo test -p textora-ui --lib
cargo test -p textora-markdown --lib
cargo test -p textora-app --lib
cargo check -p textora-app
```

Expected: 全部 PASS，无 warning 级别新增问题。

- [ ] **Step 4: 执行格式检查和项目全面验证**

```bash
cargo fmt --check
./scripts/verify.sh
```

Expected: 两条命令退出码均为 0。

- [ ] **Step 5: 按人工协议验证真实交互**

打开 `test_data/sample.mmap.md`，依次验证：

1. 点击标题中部，英文、CJK、emoji 光标均落在正确 grapheme 边界。
2. 点击卡片留白整卡选中；输入字符和中文 IME 替换原标题。
3. 编辑态 Backspace/Delete 只删文字；整卡态删除整棵子树。
4. Enter 创建空同级，Tab 创建空子节点，均显示“输入主题”占位符。
5. `Cmd+[` / `Cmd+]` 调整整棵子树；根节点和无前序同级操作无变化。
6. 每个结构操作 Undo 一次完整恢复，Redo 一次完整重放。
7. 滚动画布后点击、caret 和 IME 候选窗位置仍一致。
8. 切到源码制造解析错误，返回 mmap 显示错误态；修复后自动恢复画布。
9. 保存、关闭、重新打开后源码和画布一致。

- [ ] **Step 6: 提交最终集成测试**

```bash
git add crates/app/src/app_tests.rs test_data/sample.mmap.md
git commit -m "test(app): cover mmap wysiwyg editing"
```

提交前再次运行：

```bash
git status --short
```

Expected: 无未提交文件；若 `test_data/sample.mmap.md` 未修改，只暂存 `app_tests.rs`。
