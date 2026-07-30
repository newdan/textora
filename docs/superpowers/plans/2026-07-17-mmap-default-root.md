# 新建 MMAP 默认空根节点实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新建 MMAP 时生成一个标题为空且立即可输入的根节点，同时保持其他文档类型和已有文件行为不变。

**Architecture:** 在 app 层 `Workspace` 的按类型新建规格中补充初始源码与初始光标位置。MMAP 使用合法 MMF 源码 `#` 和光标字节 `1`；TXT、Markdown 继续使用空源码和字节 `0`。解析器、mindmap 插件和 UI 层不增加特殊分支。

**Tech Stack:** Rust、`DocumentView`、workspace 单元测试、Cargo。

## Global Constraints

- UI 层不得依赖或访问 app 层状态结构体。
- MMAP 初始源码必须为 `#`，初始光标必须位于零长度标题范围 `1..1`。
- 初始模板不计作用户编辑，文档必须保持干净。
- TXT 与 Markdown 的初始源码和光标行为保持不变。
- 不改变已有空 MMAP 文件的解析语义。
- 修改后必须通过 `cargo fmt`、workspace 相关测试和 app crate 编译检查。

## 文件结构

- Modify/Test: `crates/app/src/workspace.rs`：定义按类型的新建文档规格、构造 `DocumentView`，并放置 workspace 层回归测试。

---

### Task 1: 为新建 MMAP 提供可编辑空根节点

**Files:**
- Modify: `crates/app/src/workspace.rs:54-60,444-460`
- Test: `crates/app/src/workspace.rs` 内 `tests` 模块

**Interfaces:**
- Consumes: `DocumentView::new(Vec<String>, usize, f64)`、`DocumentView::set_cursor_offset_synced(usize)`、`NewDocumentKind`、现有插件注册表。
- Produces: 私有 `TypedUntitledSpec` 和 `typed_untitled_spec(NewDocumentKind) -> TypedUntitledSpec`，供 `Workspace::new_typed_untitled` 构造初始文档。

- [ ] **Step 1: 写入失败回归测试**

在 `workspace.rs` 的 `tests` 模块加入：

```rust
#[test]
fn new_mindmap_starts_with_editable_empty_root() {
    let mut workspace = Workspace::new();

    workspace.new_typed_untitled(ui::sidebar::NewDocumentKind::Mindmap, test_viewport());
    let entry = workspace.active_entry().expect("new mindmap must become active");

    assert_eq!(entry.doc.full_text(), "#");
    assert_eq!(entry.doc.cursor_offset().to_usize(), 1);
    assert!(!entry.doc.dirty);
    assert_eq!(entry.suggested_file_name(), Some("未命名.mmap.md"));
    assert_eq!(entry.plugin.name(), PLUGIN_MINDMAP);
}
```

- [ ] **Step 2: 运行测试并确认正确失败**

Run:

```bash
cargo test -p textora-app --lib workspace::tests::new_mindmap_starts_with_editable_empty_root -- --exact
```

Expected: FAIL；`full_text()` 的实际值为 `""`，期望值为 `"#"`。失败必须来自缺少 MMAP 初始源码，而不是编译或测试夹具错误。

- [ ] **Step 3: 实现按类型的初始源码与光标规格**

将现有二元组规格替换为语义化结构体：

```rust
struct TypedUntitledSpec {
    suggested_file_name: &'static str,
    plugin_name: &'static str,
    initial_text: &'static str,
    initial_cursor_byte: usize,
}

fn typed_untitled_spec(kind: NewDocumentKind) -> TypedUntitledSpec {
    match kind {
        NewDocumentKind::Text => TypedUntitledSpec {
            suggested_file_name: "未命名.txt",
            plugin_name: PLUGIN_EDITOR,
            initial_text: "",
            initial_cursor_byte: 0,
        },
        NewDocumentKind::Mindmap => TypedUntitledSpec {
            suggested_file_name: "未命名.mmap.md",
            plugin_name: PLUGIN_MINDMAP,
            initial_text: "#",
            initial_cursor_byte: 1,
        },
        NewDocumentKind::Markdown => TypedUntitledSpec {
            suggested_file_name: "未命名.md",
            plugin_name: PLUGIN_MARKDOWN_EDITOR,
            initial_text: "",
            initial_cursor_byte: 0,
        },
    }
}
```

再让 `Workspace::new_typed_untitled` 消费完整规格：

```rust
let spec = typed_untitled_spec(kind);
let mut document = DocumentView::new(
    vec![spec.initial_text.to_owned()],
    dims.visible_rows,
    dims.viewport_height,
);
document.set_cursor_offset_synced(spec.initial_cursor_byte);
document.dirty_snapshot_id = Some(crate::dirty_snapshot::snapshot_filename(
    &crate::dirty_snapshot::untitled_id(),
));
let plugin = self.registry.create_by_name(
    spec.plugin_name,
    Box::new(EditorPlugin::new()),
);
self.record_nav_step();
self.entries.push(DocItem::new_untitled(
    document,
    plugin,
    spec.suggested_file_name.to_owned(),
));
```

保留后续设置 `active_index` 和返回 `NavEffect::ActiveChanged` 的现有代码。

- [ ] **Step 4: 运行目标测试并确认通过**

Run:

```bash
cargo test -p textora-app --lib workspace::tests::new_mindmap_starts_with_editable_empty_root -- --exact
```

Expected: PASS。

- [ ] **Step 5: 验证 workspace 回归、格式与编译**

Run:

```bash
cargo test -p textora-app --lib workspace::tests
cargo fmt --all --check
cargo check -p textora-app
```

Expected: 三条命令全部成功；原有按类型命名、插件路由、工作区持久化测试保持通过，且无格式错误或编译警告。

- [ ] **Step 6: 提交实现**

```bash
git add crates/app/src/workspace.rs
git commit -m "feat(app): seed new mmap with empty root"
```

提交前用 `git diff --cached --check` 确认暂存内容只有 `workspace.rs`，不包含用户现有的 `.superpowers/sdd/task-4-report.md` 修改。
