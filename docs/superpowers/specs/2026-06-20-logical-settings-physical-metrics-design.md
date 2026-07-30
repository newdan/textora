# Logical Settings / Physical UiMetrics Design

## 目标

让 `Settings` 永久保存与窗口 DPI 无关的逻辑配置，让 `UiMetrics` 成为由 `(Settings, scale_factor)` 纯派生的物理布局快照，从根源上消除 DPI 状态丢失、重复缩放和逻辑/物理单位混用。

本设计在 `plans-settings-dpi-remediation.md` 的行为稳定整改完成后实施，不与当前回归修复混在同一提交序列中。

## 方案选择

采用单一逻辑 Settings：

- 不保留当前“Settings 存物理值”的模型，因为每个写入点都必须理解 DPI，容易复发。
- 不在 Settings 同时保存 logical/physical 两套字段，因为这会制造同步顺序和缓存失效问题。
- 由纯函数派生 UiMetrics；相同 Settings 与 DPI 必须得到相同输出。

## 单位契约

### Settings：逻辑单位

以下字段始终为逻辑单位：

- `font_size`
- `line_height`
- `status_bar_height`
- `gutter_padding`
- `toc_width`

以下字段是行为配置，不参与 DPI 缩放：

- `font_family` / `ui_font_family`
- `tab_width`
- `word_wrap`
- `show_line_numbers` / `show_status_bar`
- `view_mode` / `theme_mode`
- `line_height_ratio`
- `min_punctuation_width_ratio`
- `toc_max_depth`
- `max_line_bytes_for_shaping`
- `version`

Settings 不再保存 `dpi_scale`，也不再提供 `apply_scale()`、`logical_font_size()`、`logical_line_height()`。

### App：窗口 DPI 所有者

`App.scale_factor` 是当前窗口唯一 DPI 状态。初始化窗口或收到 `ScaleFactorChanged` 时只更新该字段，不修改 Settings。

App 提供：

```rust
pub(crate) fn ui_metrics(&self) -> ui::settings::UiMetrics {
    ui::settings::UiMetrics::from_settings(&self.settings, self.scale_factor as f32)
}
```

需要同一帧一致性的路径在帧入口构造一次 metrics 并向下传递，不在子函数中反复派生。

### UiMetrics：物理单位

`UiMetrics` 只包含布局、绘制和命中测试所需的物理值：

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiMetrics {
    pub dpi: f32,
    pub font_size: f32,
    pub line_height: f32,
    pub status_bar_height: f32,
    pub gutter_padding: f32,
    pub toc_width: f32,
    pub content_left_margin: f32,
    pub scrollbar_reserve: f32,
    pub show_line_numbers: bool,
    pub show_status_bar: bool,
}
```

构造入口：

```rust
pub fn from_settings(settings: &Settings, dpi: f32) -> Self
```

`dpi` 先规范到有效正数；非有限值、零或负数回退为 `1.0`。所有物理尺寸只在该函数内乘一次 DPI。

## 行为输入与布局输入分离

`word_wrap`、`theme_mode` 和 `view_mode` 不属于 metrics。Sidebar 使用独立输入：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarSettingsInput {
    pub show_line_numbers: bool,
    pub word_wrap: bool,
    pub show_status_bar: bool,
    pub theme_mode: ThemeMode,
    pub view_mode: ViewMode,
}
```

App 从同一份 Settings 构造 `UiMetrics` 与 `SidebarSettingsInput`。Widget 不回读 App 或 Settings。

## 数据流

```text
settings.toml
    ↓ deserialize
Settings（逻辑单位） ───────────┐
                                ├─ UiMetrics::from_settings(settings, dpi)
App.scale_factor（唯一 DPI） ───┘
                                        ↓
              ShellInputs / RenderContext / Widget inputs / ViewportDimensions
```

反向数据流仅允许 widget action：

```text
WidgetAction → App 更新 Settings → version/cache invalidation → 下一帧新快照
```

## 生命周期

### 启动

1. 读取 PersistedSettings。
2. 构造逻辑 Settings。
3. 创建窗口并读取 scale factor。
4. 派生 UiMetrics。
5. 使用物理 font/line height 初始化 TextState、display map、viewport 与 widgets。

Reshape worker 的初始 font family 来自 Settings，初始物理 font size 来自 UiMetrics。

### ScaleFactorChanged

1. 保存旧 metrics。
2. 更新 `App.scale_factor`。
3. 纯派生新 metrics。
4. 按新旧 metrics 比例调整仅以物理像素保存的瞬态尺寸，例如当前 sidebar drag width。
5. 失效 render cache、display map wrap、reshape generation 和 shell layout。
6. 请求重绘。

Settings 和持久化文件不发生变化。

### Zoom

Zoom 直接对 `Settings.font_size` 做逻辑 `±1` 或 reset 到 `15`。`set_font_size()` 继续按逻辑 `line_height_ratio` 更新逻辑 line height。下一次 metrics 派生得到物理值。

### 持久化

Settings 字段可直接写入 PersistedSettings，不再除以 DPI。窗口宽高、坐标仍按现有物理几何契约保存；sidebar 持久化宽度明确转换为逻辑单位。

## 缓存与版本

- Settings mutation 继续递增 `version`。
- UiMetrics 不维护独立可变版本；需要 cache key 时使用 Settings version 与 DPI bits。
- 字体、行高或 DPI 变化必须失效 display map、glyph/render cache 和异步 reshape generation。
- 只改变 theme/toggle 时按现有最小失效范围处理，不扩大为全部 reshape。

## 迁移策略

### 阶段 1：新增纯派生 API

扩展 UiMetrics，增加 `from_settings(settings, dpi)` 和纯单元测试。临时保留旧 `From<&Settings>`，使中间提交可编译。

### 阶段 2：迁移 App 与渲染路径

App 持有逻辑 Settings，窗口/生命周期更新 scale factor。按每批最多三个文件迁移：初始化、窗口、滚动、render pipeline、Markdown、Workspace viewport。

迁移期间不得同时在一个调用链混用逻辑 Settings 尺寸和物理 UiMetrics 尺寸。

### 阶段 3：迁移 widgets

tab bar、sidebar、status/search/title/TOC、popup 等只接收 UiMetrics 或自己的行为输入。删除 `UiMetrics.word_wrap/theme_mode`，新增 `SidebarSettingsInput`。

### 阶段 4：删除兼容层

全 workspace 无旧调用后删除：

- `Settings.dpi_scale`
- `Settings::apply_scale`
- `Settings::logical_font_size`
- `Settings::logical_line_height`
- `impl From<&Settings> for UiMetrics`

## 测试设计

### 纯单元测试

- Settings 在 DPI 变化模拟中数值完全不变。
- `UiMetrics::from_settings(settings, 2.0)` 对每个尺寸精确缩放一次。
- 无效 DPI 回退为 1。
- 同一输入重复派生完全相等。
- SidebarSettingsInput 只包含行为状态。

### App 集成测试

- 启动 DPI=2 时 TextState、display map、viewport 和 Shell 使用同一物理 font/line height。
- DPI 1→2→1 后逻辑 Settings 与持久化值不变，物理 metrics 可逆。
- ScaleFactorChanged 递增 reshape generation、清空相关 cache 并请求 redraw。
- Zoom 在 DPI 1/2 下逻辑步长相同。
- sidebar 宽度在 DPI 切换时只缩放一次，保存后逻辑宽度不变。

### 静态验收

```bash
rg -n "dpi_scale|apply_scale|logical_font_size|logical_line_height" crates/app/src crates/ui/src
rg -n "UiMetrics::from\(" crates/app/src crates/ui/src
rg -n "word_wrap|theme_mode|view_mode" crates/ui/src/settings.rs
```

最终期望：旧 DPI mutation API 无输出；UiMetrics 只通过 `from_settings` 构造；行为字段不再属于 UiMetrics。

## 边界情况

- scale factor 为 NaN、Infinity、0 或负数时按 1.0 派生。
- DPI 事件与窗口 resize 连续到达时，每个事件都从逻辑 Settings 重新派生，不基于上一次物理值累计。
- 无窗口测试环境使用 App 默认 scale factor 1.0。
- 用户配置极小字号仍按逻辑最小值约束，DPI 不影响约束阈值。
- 多窗口若未来出现，每个 App/窗口各自持有 scale factor；Settings 可复制但不共享物理状态。

## 不在本设计范围

- `AppEffect` dispatch 收口。
- app/ui 公共 API 收缩。
- ThemeRegistry 错误报告。
- warning 与 CI 门禁清理。
