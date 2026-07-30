# Navigator Trait v2 重构 — 代码审查

> **审查范围**: 7 个提交 (cdfe716e → dc7c6f89)，28 文件，+699/-677 行
> **审查日期**: 2026-06-22

## 概要

将 Navigator trait 从混杂了 UI 渲染/滚动/动画的"胖接口"重构为纯数据导航接口。核心变化：

- **Navigator trait** — 94→57 行，砍掉 `render()`/`hit_test()`/`hover()`/`scroll()`/`tick()`/`thickness()`
- **TabBarNavigator** — 143 行整个删除，消除冗余包装层
- **SmoothScroll** — 新增通用动画插值器，TabBar/Sidebar 共用
- **TabBarWidget** — 自主管理 `scroll_target`，`set_input()` 内做 autoscroll
- **Workspace** — 删除 `navigator` 字段，直接 `impl Navigator for Workspace`

编译通过，822 个测试全部通过（含 6 个新增测试）。

---

## 架构评估

### 设计原则符合度

| 原则 | 符合 | 说明 |
|------|------|------|
| Navigator = 纯导航 | ✅ | 删除了 render/scroll/hit_test/hover/tick/thickness |
| TabBarWidget 管渲染 | ✅ | 自持 scroll_target，set_input 内 autoscroll |
| App 管动画 | ✅ | SmoothScroll 独立于 TabBar/Sidebar |
| 多 Workspace 预留 | ✅ | App 架构上支持 `Vec<Box<dyn Navigator>>` |
| crates/ui 不依赖 crates/app | ✅ | 红线未破 |

### 数据流清晰度

```
旧: Workspace → TabBarNavigator(持 scroll_offset/scroll_target + TabBarWidget)
                  → render() 里映射 NavEntry→TabInfo，手动布局+绘制
                  → tick() 做动画

新: Workspace → items() → App 映射 NavEntry→TabInfo → TabBarWidget.set_input()
     TabBarWidget.set_input() → autoscroll → 更新 self.state.scroll_target
     App → tab_scroll.set_target(ui_shell.tab_bar_scroll_target())
     App → tab_scroll.tick() → 插值 → current() → TabBarWidget 渲染用
```

新流程职责边界清晰，每层只做自己该做的事。

---

## 逐文件点评

### `crates/app/src/navigator.rs` — 纯导航 trait ✅

57 行，干净。`NavEntry` 是 UI 投影 struct（不含 DocumentView 引用），`NavEffect` 用 enum 替代了 `WorkspaceEffect`。

**注意**: `NavEntry` 相比旧版删除了 `language` 字段。`TabInfo` 中仍保留 `language: String`，TabBar 渲染时 language 字段传空字符串。确认这不是功能退化——旧版 `language` 从未被有效填充过，删除是正确的清理。

### `crates/app/src/smooth_scroll.rs` — 新增 ✅

39 行，简洁。常量提取规范（`SNAP_THRESHOLD`/`LERP_FACTOR`）。`tick()` 返回 `bool` 语义清晰。可供 Sidebar 复用。

**建议**: `LERP_FACTOR = 0.35` 是经验值，建议加一行注释说明选择依据：
```rust
/// lerp factor: 0.35 gives smooth deceleration; higher=snappier, lower=slower
const LERP_FACTOR: f32 = 0.35;
```

### `crates/app/src/tab.rs` — `Tab → DocItem` ✅

重命名干净。新增 `doc_title()` 方法，正确使用 `file_name()` 或 `"untitled"` 后备。有单元测试覆盖。

### `crates/app/src/workspace.rs` — 核心重构 ✅

- `tabs → entries` 全量重命名，一致性好
- `#[serde(rename = "tabs")]` 保证向后兼容 ✅
- `impl Navigator for Workspace` 正确委托给现有方法
- `len()` 被 override 为 `self.entries.len()`（避免 `items()` 的 Vec 分配）
- 所有 `WorkspaceEffect` → `NavEffect` 替换完整

**小问题**: `Navigator::close()` 中 `self.close_entry(index).unwrap_or(NavEffect::None)` 会静默吞掉 pinned/越界错误。这在旧代码中也有同样行为，不是新问题，但值得在 trait 文档中说明语义：close 是 best-effort。

### `crates/ui/src/widgets/tab_bar/` — scroll 归属

**state.rs**: `scroll_target` + `scroll_by()`/`scroll_target()`/`set_scroll_target()` 三个方法，接口干净。

**widget.rs**: `set_input()` 中 autoscroll 逻辑内联得漂亮——布局后立即计算目标，调用方无需关心。公开 `scroll_by()` 和 `scroll_target()` 供 UiShell 桥接。

新增 3 个 scroll 测试覆盖了边界情况（layout 前 clamp to 0、layout 后正常滚动、直接 set）。

### `crates/app/src/ui_shell.rs` — 桥接方法 ⚠️

`tab_bar_scroll_by()` 和 `tab_bar_scroll_target()` 通过遍历 dock children + downcast 找到 TabBarWidget：

```rust
for child in &self.dock.children {
    if let Some(tbw) = child.widget.as_any_mut().downcast_mut::<TabBarWidget>() {
```

**权衡**: 当前 dock children 数量很小（< 20），线性搜索无害。如果未来 dock 变得复杂，可考虑在 UiShell 中缓存 TabBarWidget 引用。

### `crates/app/src/dispatch/tabs.rs` — 效果处理

`handle_workspace_effect` 现在接收 `NavEffect` 而非 `WorkspaceEffect`，语义上更准确。变体名从 `ActiveTabChanged`/`LayoutChanged` 改为 `ActiveChanged`/`ItemsChanged`，去掉了 UI 暗示。

---

## 发现的问题

### 🔴 必须修复

**`app.rs:97` 遗留注释**：
```rust
/// Preview entry index (managed by plugin, not pure navigation).
pub(crate) modifiers: winit::keyboard::ModifiersState,
```
这条注释是为 `preview_index: Option<usize>` 字段准备的，但该字段从未添加到 App。注释现在悬在 `modifiers` 上方，造成误导。应删除。

### 🟡 建议修复

1. **`preview_index` 位置**: Spec 建议移到 App 层，但实际留在 Workspace。这合理——Workspace 的 `switch_to()` 需要用它在切走时自动关闭预览标签。但值得在代码注释中说明为什么留在 Workspace。

2. **TabBarWidget `autoscroll_target` 每帧调用**: 当前每次 `set_input()` 都计算 autoscroll，但只在 active_index 变化时才需要。可加一个 `Option<usize>` 缓存上次 active_index，只在变化时计算。当前开销微乎其微，非阻塞。

3. **SmoothScroll 没有 reset 方法**: 如果未来需要强制跳到目标位置（如 workspace 切换后），需要 `pub fn snap_to(&mut self, value: f32)`。目前不需要，但接口可考虑预留。

### 🟢 已验证 OK

- `Workspace::len()` override 避免 `items()` 的 per-frame 分配 ✅
- `#[serde(rename = "tabs")]` 向后兼容持久化格式 ✅
- TabBarWidget autoscroll 在 `set_input()` 内部、外部调用方无感 ✅
- 所有 6 处旧 navigator 调用点已替换 ✅
- `NavEffect::merge()` 逻辑从 `WorkspaceEffect::merge()` 完整移植 ✅

---

## 测试覆盖

| 区域 | 状态 | 说明 |
|------|------|------|
| DocItem::doc_title() | ✅ 新增 2 个测试 | 有路径/无路径两种情况 |
| TabBarState::scroll_by() | ✅ 新增 3 个测试 | layout 前 clamp、正常滚动、直接 set |
| Workspace navigator 功能 | ✅ 已有测试完整覆盖 | switch_to/close/pin/go_back/go_forward |
| 持久化兼容 | ✅ `snapshot_filename` roundtrip + 旧格式兼容测试 | |
| 关闭逻辑 | ✅ dirty/pinned/out-of-range 三种情况 | |
| 回归测试 | ✅ app_tests 全部通过 | |

822 个测试全通过，无新增失败。

---

## 总结

重构质量高，职责分离清晰。TabBarNavigator（143 行冗余包装）的删除简化了数据流，SmoothScroll 提取为可复用的通用组件。唯一需要修复的是 `app.rs:97` 的遗留注释。

**建议**: 修复遗留注释后即可合并。
