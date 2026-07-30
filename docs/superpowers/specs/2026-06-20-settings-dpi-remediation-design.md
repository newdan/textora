# Settings / DPI Remediation Design

## 目标

修复 Phase 4 Settings 所有权迁移后仍然存在的实例状态丢失问题，使 DPI、字体、行高、视图模式、TOC 和 gutter 等配置始终来自当前 `App` 实例，并通过显式、最小的数据接口传递给不拥有 App 的模块。

本轮优先恢复行为正确性，不同时重写 Settings 的逻辑单位/物理单位存储模型。Settings 永久保持逻辑单位的架构迁移留给后续独立设计。

## 范围

本轮包含：

- 清除 app 生产路径中除根初始化之外的 `Settings::new()` 默认读取。
- 修复 Tabs 模式顶部高度、IME 定位和 tab bar 滚动命中。
- 修复 Retina 下编辑器滚动、光标可见性、Markdown preview、TOC 和 gutter 的配置传播。
- 让 Workspace、Markdown preview 和 DocumentView 接收最小显式输入。
- 修复 Zoom 在 DPI 1/2 下逻辑步长、重置和持久化不一致。
- 删除 `ShellInputs` 中重复的 DPI 真值和无意义的 Settings clone。
- 增加回归测试与残留扫描。

本轮不包含：

- Phase 3 `AppEffect` 的完整收口。
- app/ui 公共 API 收缩。
- Settings 永久保持逻辑单位的整体重构。
- 与本轮无关的 core 重复测试名修复。

## 核心约束

1. `App` 是运行时 Settings 的唯一所有者。
2. App 方法直接读取 `self.settings`，且在取得 Workspace 可变借用前先复制所需标量。
3. 非 App 模块不得构造默认 Settings，也不得依赖完整 Settings；只接收完成职责所需的最小输入。
4. 当前 `Settings::apply_scale()` 已将 `font_size`、`line_height` 和 `toc_width` 转为物理像素。调用方不得再次对这些值乘 DPI。
5. 每个实施阶段最多修改三个文件。
6. 每个行为修复必须先有可稳定失败的回归测试。

## 组件设计

### App 热路径

`app_init`、`app_scroll`、`app_renderer`、`app_dispatch` 和 `dispatch/mouse` 使用当前 App 实例的 Settings。需要同时可变借用 Workspace 时，先提取 `dpi`、`line_height`、`font_size`、`view_mode` 等 `Copy` 值，以保持借用边界清晰。

`App::current_tab_bar_height()` 不再委托 Workspace 回读配置。它根据：

- `self.settings.view_mode`
- `self.settings.dpi_scale`
- `self.workspace.len()`

直接计算高度。Sidebar 模式恒为零；Tabs 模式只有多文档时返回 `tab_bar_height(dpi)`。

### Workspace viewport 输入

Workspace 不负责读取 UI 设置。引入 app 层纯数据：

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ViewportDimensions {
    pub(crate) visible_rows: usize,
    pub(crate) viewport_height: f64,
}
```

`Workspace::open_file`、`new_empty_tab` 和 `restore` 接收 `ViewportDimensions`，由 App 使用当前 Settings 和窗口高度计算。Workspace 只把这些值交给 `DocumentView`。

### Markdown preview 配置

Markdown preview 接收专用输入：

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MarkdownRenderSettings {
    pub(crate) font_size: f32,
    pub(crate) line_height: f32,
    pub(crate) toc_max_depth: u8,
}
```

`render` 和 heading 收集使用同一份配置。App renderer 从 `self.settings` 构造输入，避免渲染尺寸、TOC 内容和 hit-test 使用不同配置。

### DocumentView visible API

旧 visible helper 不再内部创建 Settings。需要根据像素锚点计算范围的方法显式接收 `line_height`。调用链内部原样传递该标量；测试使用明确的测试行高。

不在 Viewport 或 DocumentView 中新增 Settings 缓存，避免形成第二份运行时真值。

### Shell DPI

`ShellInputs` 只保留 `metrics: UiMetrics`，删除独立 `dpi` 字段。所有 shell/widget 路径统一读取 `inputs.metrics.dpi`。

`UiMetrics::from(&self.settings)` 直接借用当前 Settings，不 clone 包含字符串的完整对象。

### Zoom 单位

对外 Zoom 语义采用逻辑字号：

- Zoom In：逻辑字号增加 1。
- Zoom Out：逻辑字号减少 1，最小为 6。
- Zoom Reset：逻辑字号恢复 15。

在当前物理 Settings 模型下，进入 `apply_zoom` 前后通过 `dpi_scale` 做逻辑/物理换算。`settings.font_size` 和 `line_height` 继续保持物理像素，`logical_font_size()` 的持久化结果必须与 DPI 无关。

## 数据流

```text
PersistedSettings (逻辑单位)
        |
        v
App.settings --apply_scale--> 当前物理 Settings
        |
        +--> App 热路径直接读取 Copy 标量
        |
        +--> ViewportDimensions --> Workspace --> DocumentView
        |
        +--> MarkdownRenderSettings --> MdPreview
        |
        +--> UiMetrics --> UiShell / widgets
```

任何子模块都不得沿反方向重新构造或查询 Settings。

## 测试设计

测试使用能暴露默认回退的哨兵配置：

```text
dpi_scale = 2.0
font_size = 36.0
line_height = 58.248
view_mode = Tabs
show_line_numbers = false
toc_width = 480.0
toc_max_depth = 5
```

必须覆盖：

1. Tabs + 多文档 + DPI 2 时 tab bar 高度为 64；Sidebar 时为 0。
2. Tabs 模式 content top、IME Y 和 tab bar 滚动命中使用相同高度。
3. 光标移动、Page Up/Down、像素滚动和 anchor roundtrip 使用实例行高。
4. Preview padding、hit-test offset、TOC 命中宽度与渲染布局一致。
5. 隐藏行号和自定义字号影响 gutter，而不是回退默认设置。
6. Workspace open/new/restore 使用 App 计算的 viewport dimensions。
7. Markdown 样式使用当前字体/行高，TOC 接受深度 5 的标题。
8. Zoom 在 DPI 1 和 2 下都按逻辑 1pt 变化，reset 后逻辑字号为 15，最小逻辑字号为 6。
9. DPI 1→2→1 不产生累计缩放。
10. `ShellInputs` 只有一份 DPI 来源。

每项修复遵循：失败测试 → 最小实现 → 定向测试 → app 编译 → 提交。

## 静态验收

生产代码最终满足：

```bash
rg -n "Settings::new\(\)" crates/app/src -g '*.rs'
rg -n "self\.settings\.clone\(\)" crates/app/src -g '*.rs'
```

第一条只允许 App 根设置初始化和明确位于 `#[cfg(test)]` 下的测试夹具；第二条必须无输出。

动态验收：

```bash
cargo check -p edit-plus-app
cargo test -p edit-plus-app --lib
cargo test -p edit-plus-ui --lib
```

当前 `cargo check --workspace --all-targets` 会被既有的 core 重复测试名阻塞。本轮保持记录，但不扩大范围顺手修复。

## 阶段切分

1. Tabs、IME 和 preview offset。
2. 滚动与鼠标输入。
3. display map 初始化与 gutter。
4. Workspace 显式 viewport 输入。
5. Markdown preview 显式配置。
6. DocumentView visible API。
7. Zoom 逻辑单位修复。
8. Shell 单一 DPI 来源与残留清扫。

每阶段独立可测试、独立提交，且不超过三个修改文件。

## 后续工作

本轮稳定后另起设计，将 Settings 改为永远保存逻辑单位，并由 `(Settings, scale_factor)` 纯派生 `UiMetrics`。该工作不得与本轮行为修复混在同一提交序列中。
