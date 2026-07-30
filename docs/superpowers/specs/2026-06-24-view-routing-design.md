# 视图路由重构设计

## 目标

重新梳理文件打开时的默认视图选择、视图切换按钮行为、以及视图记忆逻辑。

## 视图路由规则

### 空态（无记忆）

| 文件扩展名 | 默认视图 | 切换目标 | 切换按钮 |
|-----------|---------|---------|---------|
| `*.mmap.md` | mindmap（思维导图） | markdown_editor（MD编辑） | 有 |
| `*.md` / `*.markdown` | markdown_editor（MD编辑） | editor（基础编辑） | 有 |
| `*.txt` | editor（基础编辑） | novel_view（小说） | 有 |
| 其他 | editor（基础编辑） | 无 | 无 |

### 有记忆

Tab 关闭时记住当前使用的插件名（`active_plugin`），下次恢复时直接使用记忆值，忽略路由表默认值。

## 核心变更

### 1. 视图路由表（`workspace.rs` 新增）

```rust
/// 根据文件路径返回路由信息。
fn view_route(path: &Path) -> ViewRoute {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.ends_with(".mmap.md") {
        ViewRoute { default_plugin: "mindmap", toggle_target: Some("markdown_editor") }
    } else {
        match path.extension().and_then(|e| e.to_str()) {
            Some("md") | Some("markdown") =>
                ViewRoute { default_plugin: "markdown_editor", toggle_target: Some("editor") },
            Some("txt") =>
                ViewRoute { default_plugin: "editor", toggle_target: Some("novel_view") },
            _ =>
                ViewRoute { default_plugin: "editor", toggle_target: None },
        }
    }
}
```

未知文件回落 `editor`，无切换按钮。

### 2. PluginRegistry 新增按名创建（`plugin.rs`）

```rust
impl PluginRegistry {
    /// 按插件名创建，找不到返回 fallback。
    fn create_by_name(&self, name: &str, fallback: Box<dyn ViewPlugin>) -> Box<dyn ViewPlugin> {
        self.factories.iter()
            .find(|f| f.name() == name)
            .map(|f| f.create())
            .unwrap_or(fallback)
    }
}
```

### 3. PersistedTab 变更（`workspace.rs`）

```rust
// 旧
was_preview: bool,

// 新：None 表示用路由表默认值
active_plugin: Option<String>,
```

`#[serde(default)]` 保证旧快照兼容（None → 回落路由表）。

### 4. Workspace 变更（`workspace.rs`）

**`open_file_with_viewport` / `push_entry_for_file`：**
路由表 → 默认插件名 → 按名创建。

**`switch_plugin`：**
当前插件名 → 路由表 `toggle_target` → 按名创建目标插件并替换。不再区分编辑/预览二元。

**保存快照：**
`was_preview` → `active_plugin: Some(plugin.name())`。

**恢复快照：**
`ts.active_plugin` 有值则按名恢复，`None` 走路由表默认。

### 5. TitleBarInput 变更（`title_bar.rs`）

```rust
// 旧
is_md: bool,
is_preview: bool,

// 新
can_toggle: bool,                    // 是否有切换按钮
toggled: bool,                       // 是否处于切换后的视图（控制高亮）
toggle_target_label: Option<String>, // 按钮 tooltip 文本
```

### 6. NovelViewFactory 注册位置不变

NovelViewFactory 仍在注册表中，但路由表 `*.txt` 默认走 `editor`，只有用户点击切换才创建 NovelView。

## 涉及文件

| 文件 | 变更 |
|------|------|
| `crates/app/src/workspace.rs` | 路由表、open/switch/save/restore 逻辑 |
| `crates/ui/src/plugin.rs` | 新增 `create_by_name` |
| `crates/ui/src/widgets/title_bar.rs` | TitleBarInput 字段重构 |
| `crates/app/src/app_window.rs` | `build_shell_inputs` 中 TitleBarInput 构造 |
| `crates/app/src/dispatch/editor.rs` | ToggleMarkdownPreview 适配 |
| `crates/app/src/events.rs` | ToggleMarkdownPreview → ToggleView |

## 不影响

- MarkdownViewFactory / MarkdownView（MD预览）保留不动，内部仍被 Novel 使用
- MindmapPluginFactory 匹配逻辑不变
- 各插件自身实现不变
- ViewMode（Sidebar/Tabs）逻辑不变
