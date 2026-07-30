# View Routing 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 重构视图路由规则 — 按文件扩展名决定默认视图和切换目标，Tab 级记忆上次使用的视图。

**Architecture:** 新增静态视图路由表 `view_route(path) -> ViewRoute`，PluginRegistry 新增 `create_by_name` 按名创建插件，`PersistedTab.was_preview` 替换为 `active_plugin: Option<String>`，TitleBarInput 的 `is_md`/`is_preview` 重构为 `can_toggle`/`toggled`/`toggle_label`。

**Tech Stack:** Rust

## Global Constraints

- 全程使用中文回复
- 每次提交前必须编译通过
- 遵守 `cargo fmt` 和项目代码洁癖规范
- 严禁滥用 `.unwrap()`，必须用 `.expect("原因")`

---

### Task 1: PluginRegistry 新增 create_by_name

**Files:**
- Modify: `crates/ui/src/plugin.rs`

**Interfaces:**
- Produces: `PluginRegistry::create_by_name(&self, name: &str, fallback: Box<dyn ViewPlugin>) -> Box<dyn ViewPlugin>`

- [ ] **Step 1: 添加 create_by_name 方法**

在 `crates/ui/src/plugin.rs` 的 `impl PluginRegistry` 块中，`create_editor_for_file` 方法之后添加：

```rust
/// 按插件名查找工厂并创建插件。找不到则返回 fallback。
pub fn create_by_name(
    &self,
    name: &str,
    fallback: Box<dyn ViewPlugin>,
) -> Box<dyn ViewPlugin> {
    self.factories
        .iter()
        .find(|f| f.name() == name)
        .map(|f| f.create())
        .unwrap_or(fallback)
}
```

- [ ] **Step 2: 编译验证**

```bash
cargo check -p ui 2>&1 | head -20
```

预期：编译通过。

- [ ] **Step 3: 提交**

```bash
git add crates/ui/src/plugin.rs
git commit -m "feat(plugin): add PluginRegistry::create_by_name for name-based factory lookup"
```

---

### Task 2: 视图路由表 + Workspace 核心变更

**Files:**
- Modify: `crates/app/src/workspace.rs`

**Interfaces:**
- Consumes: `PluginRegistry::create_by_name` (Task 1)
- Produces: `view_route(path) -> ViewRoute`, 修改后的 `push_entry_for_file`, `switch_plugin`, `save_snapshot`, `restore_with_viewport`, `can_toggle(path) -> Option<&str>`

- [ ] **Step 1: 定义 ViewRoute 结构体和路由表函数**

在 `crates/app/src/workspace.rs` 顶部、`PersistedTab` 定义之前添加：

```rust
/// 视图路由条目：文件匹配 → (默认插件名, 切换目标插件名)
struct ViewRoute {
    default_plugin: &'static str,
    toggle_target: Option<&'static str>,
}

/// 根据文件路径返回视图路由。
fn view_route(path: &std::path::Path) -> ViewRoute {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if name.ends_with(".mmap.md") {
        ViewRoute { default_plugin: "mindmap", toggle_target: Some("markdown_editor") }
    } else {
        match path.extension().and_then(|e| e.to_str()) {
            Some("md") | Some("markdown") => {
                ViewRoute { default_plugin: "markdown_editor", toggle_target: Some("editor") }
            }
            Some("txt") => {
                ViewRoute { default_plugin: "editor", toggle_target: Some("novel_view") }
            }
            _ => ViewRoute { default_plugin: "editor", toggle_target: None },
        }
    }
}
```

- [ ] **Step 2: 修改 PersistedTab — was_preview → active_plugin**

在 `PersistedTab` 结构体中，替换 `was_preview` 字段：

```rust
// 旧：
/// 关闭时是否处于预览/novel 模式。
#[serde(default)]
pub(crate) was_preview: bool,

// 新：
/// 关闭时使用的插件名。None 表示用路由表默认值。
#[serde(default)]
pub(crate) active_plugin: Option<String>,
```

- [ ] **Step 3: 修改 push_entry_for_file — 使用路由表默认插件**

```rust
pub(crate) fn push_entry_for_file(&mut self, dv: DocumentView, path: &std::path::Path) {
    let route = view_route(path);
    let plugin = self.registry.create_by_name(route.default_plugin, Box::new(EditorPlugin));
    self.entries.push(DocItem::new(dv, plugin));
}
```

- [ ] **Step 4: 修改 open_file_with_viewport — 使用路由表默认插件**

将第 372 行：
```rust
let plugin = self.registry.create_for_file(Some(path), Box::new(EditorPlugin));
```
改为：
```rust
let route = view_route(path);
let plugin = self.registry.create_by_name(route.default_plugin, Box::new(EditorPlugin));
```

- [ ] **Step 5: 重写 switch_plugin — 基于路由表而非二元编辑/预览**

用以下代码替换现有的 `switch_plugin` 方法（第 187–218 行）：

```rust
pub(crate) fn switch_plugin(&mut self) {
    if self.active_index >= self.entries.len() {
        return;
    }
    let tab = &mut self.entries[self.active_index];
    let path = tab.file_path().cloned();
    let route = match path.as_deref() {
        Some(p) => view_route(p),
        None => return, // untitled tab — no toggle
    };
    let toggle_target = match route.toggle_target {
        Some(t) => t,
        None => return, // no toggle for this file type
    };

    let current_name = tab.plugin.name().to_string();
    let is_default = current_name == route.default_plugin;

    if is_default {
        // 从默认视图切换到目标视图：缓存当前插件
        tab.preview_scroll_y = tab.query_float(ui::plugin::PluginQuery::ScrollY);
        tab.cached_preview = Some(std::mem::replace(
            &mut tab.plugin,
            self.registry.create_by_name(toggle_target, Box::new(EditorPlugin)),
        ));
    } else {
        // 从目标视图切回默认视图：优先从缓存恢复
        if let Some(mut cached) = tab.cached_preview.take() {
            let scroll_y = tab.preview_scroll_y;
            cached.handle_message(
                ui::plugin::PluginMessage::SetScrollY(scroll_y),
                &mut tab.doc,
            );
            tab.plugin = cached;
        } else {
            tab.plugin = self
                .registry
                .create_by_name(route.default_plugin, Box::new(EditorPlugin));
        }
    }
}
```

- [ ] **Step 6: 新增 can_toggle 方法（替代 has_preview_plugin）**

```rust
/// 返回当前 tab 的切换目标视图名。None 表示不可切换。
pub(crate) fn toggle_target(&self) -> Option<&'static str> {
    let path = self.active_entry().and_then(|t| t.file_path())?;
    view_route(path).toggle_target
}

/// 当前 tab 是否处于切换后的视图（非默认视图）。
pub(crate) fn is_toggled(&self) -> bool {
    let entry = match self.active_entry() {
        Some(e) => e,
        None => return false,
    };
    let path = match entry.file_path() {
        Some(p) => p,
        None => return false,
    };
    entry.plugin.name() != view_route(path).default_plugin
}
```

- [ ] **Step 7: 修改 save_snapshot — was_preview → active_plugin**

将第 589 行：
```rust
let is_preview = !t.plugin.allows_editing();
```
和第 611 行：
```rust
was_preview: is_preview,
```
改为：
```rust
let active_plugin = Some(t.plugin.name().to_string());
```
和：
```rust
active_plugin,
```

同时保留 `preview_anchor_text` 和 `preview_anchor_offset` 的采集逻辑（第 590–598 行），条件从 `is_preview` 改为 `!t.plugin.allows_editing()`。

- [ ] **Step 8: 修改 restore_with_viewport — was_preview → active_plugin**

将第 755–788 行的恢复逻辑替换为：

```rust
let route = ts.file_path.as_deref().map(view_route);
let plugin_name = ts.active_plugin.as_deref()
    .or(route.as_ref().map(|r| r.default_plugin))
    .unwrap_or("editor");
let plugin = self.registry.create_by_name(plugin_name, Box::new(EditorPlugin));

let mut item = DocItem::new(doc, plugin);

// 如果恢复的插件是只读视图且有锚点信息，恢复滚动位置
if !item.plugin.allows_editing()
    && let Some(ref text) = anchor_text
{
    item.plugin.handle_message(
        ui::plugin::PluginMessage::RestoreScrollAnchor {
            text: text.clone(),
            offset: anchor_offset,
        },
        &mut item.doc,
    );
}

// 如果当前是默认视图且有切换目标，预缓存切换目标插件
if let Some(target) = route.as_ref().and_then(|r| r.toggle_target) {
    if plugin_name != target {
        item.cached_preview = Some(
            self.registry.create_by_name(target, Box::new(EditorPlugin))
        );
    }
}
```

- [ ] **Step 9: 编译验证**

```bash
cargo check -p app 2>&1 | head -40
```

预期：编译通过。

- [ ] **Step 10: 提交**

```bash
git add crates/app/src/workspace.rs
git commit -m "feat(workspace): view routing table with per-tab plugin memory"
```

---

### Task 3: TitleBar 字段重构

**Files:**
- Modify: `crates/ui/src/widgets/title_bar.rs`

**Interfaces:**
- Consumes: 新的 `TitleBarInput` 字段 (`can_toggle`, `toggled`, `toggle_label`)
- Produces: 重命名的 `TitleBarAction::ToggleView` (替代 `ToggleMarkdownPreview`)

- [ ] **Step 1: 更新 TitleBarInput 结构体**

替换第 31–34 行：

```rust
/// 当前文件是否可切换视图（有 toggle_target）。
pub can_toggle: bool,
/// 当前是否处于切换后的视图（控制按钮高亮）。
pub toggled: bool,
/// 切换按钮 tooltip 文本（如 "基础编辑"、"小说模式"）。
pub toggle_label: Option<String>,
```

删除 `is_md: bool` 和 `is_preview: bool`。

- [ ] **Step 2: 重命名 TitleBarAction**

第 43–46 行，将：
```rust
/// Toggle between edit and preview mode for markdown files.
ToggleMarkdownPreview,
```
改为：
```rust
/// 切换当前文件的视图模式。
ToggleView,
```

- [ ] **Step 3: 更新 set_rect — 用 can_toggle 替代 is_md**

第 102 行 `if input.is_md {` 改为 `if input.can_toggle {`。

- [ ] **Step 4: 更新 paint — 图标颜色和 tooltip**

第 190 行 `if input.is_md && self.toggle_rect.w > 0.0 {` 改为 `if input.can_toggle && self.toggle_rect.w > 0.0 {`。

图标颜色逻辑（第 193–209 行的 icon_color）：将 `input.is_preview` 改为 `input.toggled`。

- [ ] **Step 5: 更新 on_event MouseDown**

第 273–278 行，将 `TitleBarAction::ToggleMarkdownPreview` 改为 `TitleBarAction::ToggleView`。

- [ ] **Step 6: 更新 tooltip_at**

第 309–313 行，将 tooltip 文本改为使用 `input.toggle_label`：

```rust
fn tooltip_at(&self, pos: (f32, f32)) -> Option<TooltipHint> {
    if let Some(ref input) = self.input {
        if self.toggle_rect.contains(pos) {
            return input.toggle_label.as_ref().map(|label| {
                TooltipHint::simple(label.clone(), None)
            });
        }
    }
    None
}
```

（注意删掉原来的 `is_preview` 判断和硬编码的 "编辑 ⌘⇧M" / "预览 ⌘⇧M" 文本）

- [ ] **Step 7: 更新所有测试用例中的 TitleBarInput 构造**

文件中所有测试函数（约第 371–670 行）都需要更新 `TitleBarInput` 构造：

- 删除 `is_md: false,` 和 `is_preview: false,`
- 替换为 `can_toggle: false, toggled: false, toggle_label: None,`

将旧的 `is_md: true, is_preview: true` 模式替换为 `can_toggle: true, toggled: true, toggle_label: Some("预览".into()),`。

将旧的 `is_md: true, is_preview: false` 模式替换为 `can_toggle: true, toggled: false, toggle_label: Some("预览".into()),`。

- [ ] **Step 8: 编译验证**

```bash
cargo check -p ui 2>&1 | head -20
```

预期：编译通过。

- [ ] **Step 9: 提交**

```bash
git add crates/ui/src/widgets/title_bar.rs
git commit -m "refactor(title_bar): replace is_md/is_preview with can_toggle/toggled/toggle_label"
```

---

### Task 4: App 层连线 — renderer, events, commands

**Files:**
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/app/src/events.rs`
- Modify: `crates/app/src/dispatch/editor.rs`
- Modify: `crates/app/src/dispatch/commands.rs`
- Modify: `crates/app/src/input.rs`

- [ ] **Step 1: 更新 app_renderer.rs 中 TitleBarInput 构造**

将第 339–352 行：
```rust
let file_path = self.workspace.active_doc().and_then(|dv| dv.file_path.clone());
let has_preview = self.workspace.has_preview_plugin();
let hamburger_right = ui::constants::TRAFFIC_LIGHT_TOTAL_W * dpi;
let sidebar_left = self.ui_shell.sidebar_editor_left_offset().max(hamburger_right);
let titlebar_x = self.ui_shell.sidebar_editor_left_offset().max(0.5);
self.ui_shell.set_title_bar_input(ui::title_bar::TitleBarInput {
    file_path,
    sidebar_left,
    titlebar_x,
    is_md: has_preview,
    is_preview: is_readonly_view,
    toc_visible: self.workspace.active_entry().is_some_and(|t| t.toc_visible),
    toc_enabled: is_readonly_view,
});
```
改为：
```rust
let file_path = self.workspace.active_doc().and_then(|dv| dv.file_path.clone());
let toggle_target = self.workspace.toggle_target();
let can_toggle = toggle_target.is_some();
let toggled = self.workspace.is_toggled();
let toggle_label = toggle_target.map(|name| {
    match name {
        "editor" => "基础编辑".to_string(),
        "novel_view" => "小说模式".to_string(),
        "markdown_editor" => "MD编辑".to_string(),
        _ => name.to_string(),
    }
});
let hamburger_right = ui::constants::TRAFFIC_LIGHT_TOTAL_W * dpi;
let sidebar_left = self.ui_shell.sidebar_editor_left_offset().max(hamburger_right);
let titlebar_x = self.ui_shell.sidebar_editor_left_offset().max(0.5);
// is_readonly_view 已在前面计算（第 227 行），保持不变
let toc_enabled = is_readonly_view;
self.ui_shell.set_title_bar_input(ui::title_bar::TitleBarInput {
    file_path,
    sidebar_left,
    titlebar_x,
    can_toggle,
    toggled,
    toggle_label,
    toc_visible: self.workspace.active_entry().is_some_and(|t| t.toc_visible),
    toc_enabled,
});
```

删除第 340 行的 `let has_preview = self.workspace.has_preview_plugin();`

- [ ] **Step 2: 更新 events.rs — 重命名 Action**

将第 148–151 行：
```rust
TitleBarAction::ToggleMarkdownPreview => {
    actions.push(AppAction::ExecuteAppCommands(vec![AppCommand::Edit(
        crate::input::EditCommand::ToggleMarkdownPreview,
    )]));
}
```
改为：
```rust
TitleBarAction::ToggleView => {
    actions.push(AppAction::ExecuteAppCommands(vec![AppCommand::Edit(
        crate::input::EditCommand::ToggleView,
    )]));
}
```

- [ ] **Step 3: 重命名 — input.rs, commands.rs, editor.rs**

在 `crates/app/src/input.rs` 第 65 行：
```rust
// 旧
ToggleMarkdownPreview,
// 新
ToggleView,
```

在第 262 行快捷键映射处：
```rust
"m" | "M" if shift => Some(EditCommand::ToggleView),
```

在 `crates/app/src/dispatch/commands.rs` 中（搜索 `ToggleMarkdownPreview` 引用），将对应的 pattern 改为 `EditCommand::ToggleView`。

在 `crates/app/src/dispatch/editor.rs` 第 292 行：
```rust
EditCommand::ToggleMarkdownPreview => {
```
改为：
```rust
EditCommand::ToggleView => {
```

同时，将 `switch_plugin` 之后的逻辑简化——不再需要 `allows_editing()` 检查来判断是否是编辑器（因为现在两个方向都可能允许编辑）。简化为无条件 resize + init_display_map：

```rust
EditCommand::ToggleView => {
    self.workspace.switch_plugin();
    let h = self.screen_height();
    let visible_rows = self.visible_rows(h);
    let viewport_height = self.visible_height_lines(h);
    if let Some(dv) = self.workspace.active_doc_mut() {
        dv.resize(visible_rows, viewport_height);
    }
    self.init_display_map(self.workspace.active_index());
    self.frame_cache.advance_cache.clear();
    self.frame_cache.cluster_pool.clear();
    effect = effect.merge(AppEffect::REDRAW);
    return effect;
}
```

删除原来的 `is_editor` 变量和条件判断。

- [ ] **Step 4: 删除 workspace.rs 中废弃的 has_preview_plugin 方法**

移除 `workspace.rs` 第 220–225 行的 `has_preview_plugin` 方法（已被 `toggle_target` 替代）。

- [ ] **Step 5: 编译验证**

```bash
cargo check 2>&1 | head -40
```

预期：编译通过。

- [ ] **Step 6: 提交**

```bash
git add crates/app/src/app_renderer.rs crates/app/src/events.rs \
        crates/app/src/dispatch/editor.rs crates/app/src/dispatch/commands.rs \
        crates/app/src/input.rs crates/app/src/workspace.rs
git commit -m "refactor(app): wire new view routing into renderer, events, and dispatch"
```

---

### Task 5: 全量编译 + 测试验证

- [ ] **Step 1: 全量编译**

```bash
cargo build 2>&1 | tail -20
```

预期：编译通过，无 warning（或仅有预先存在的 warning）。

- [ ] **Step 2: 运行测试**

```bash
cargo test 2>&1 | tail -30
```

预期：所有测试通过。如 TitleBar 测试有失败，检查测试中 `TitleBarInput` 的字段是否都已更新。

- [ ] **Step 3: 运行完整验证脚本**

```bash
./scripts/verify.sh
```

- [ ] **Step 4: fmt**

```bash
cargo fmt -- --check
```

如有格式问题：
```bash
cargo fmt
git add -A
git commit -m "chore: cargo fmt"
```

- [ ] **Step 5: 最终提交（如有修复）**

如验证过程中有修复，提交修复。否则此步骤跳过。

---

### Task 6: (可选) 手动验证

手动验证以下场景：

- [ ] `.md` 文件打开 → WYSIWYG MD 编辑视图，标题栏有切换按钮，tooltip 显示"基础编辑"
- [ ] 点击切换按钮 → 切换到基础编辑视图，按钮高亮
- [ ] 再次点击 → 切回 MD 编辑视图，按钮变灰
- [ ] `.txt` 文件打开 → 基础编辑视图，切换按钮 tooltip 显示"小说模式"
- [ ] 点击切换 → 切换到 Novel 视图
- [ ] `.mmap.md` 文件打开 → 思维导图视图，切换按钮 tooltip 显示"MD编辑"
- [ ] `.rs` 文件打开 → 基础编辑，无切换按钮
- [ ] 将 `.txt` 切换到 novel 视图 → 关闭 tab → 重新打开 → 自动恢复为 novel 视图
- [ ] `Cmd+Shift+M` 快捷键仍然正常工作
