# Explicit UI Inputs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 ui crate 只解析/布局/绘制显式输入，不直接扫描主题目录，也不从 thread-local Settings 隐式获取 widget 配置。

**Architecture:** theme I/O 下沉到 app，ui registry 接收带来源路径的 TOML 字符串；Settings 先作为 app-owned 值存在，再通过小型 `UiMetrics`/widget context 逐组件注入。每次只迁移一个 widget 家族，最终删除 thread-local singleton 并缩小 ui 导出面。

**Tech Stack:** Rust、serde/toml、现有 ui widget input structs。

---

### Task 1: 分离 ThemeRegistry 解析与文件 I/O

**Files:**
- Modify: `crates/ui/src/theme.rs:190-430`
- Create: `crates/app/src/theme_loader.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] **Step 1: 将 UI 测试改为内存 source**

用以下类型替代依赖 tempdir 的 registry 解析测试：

```rust
#[derive(Debug, Clone)]
pub struct ThemeSource {
    pub id: String,
    pub path: PathBuf,
    pub content: String,
}
```

测试直接传 `ThemeSource { id: "custom".into(), path: "custom.toml".into(), content: VALID_TOML.into() }`，覆盖有效主题、非法 TOML、未知 extends 与多 source 继承。

- [ ] **Step 2: ThemeRegistry 改为纯解析 API**

```rust
pub fn register_sources(&mut self, sources: impl IntoIterator<Item = ThemeSource>) -> Vec<LoadError>;
```

`PendingTheme` 保存 `path/content/extends/is_dark`，`load_pending` 不再调用 `std::fs::read_to_string`。删除 `load_user_themes`、`scan_theme_header(path)` 和所有 `read_dir/read_to_string`。

- [ ] **Step 3: app theme_loader 负责目录读取**

```rust
pub(crate) fn load_theme_sources(dir: &Path) -> io::Result<Vec<ui::theme::ThemeSource>>;
```

只读取 `.toml`，按路径排序以保证诊断稳定，内置 id 冲突仍交给 registry；单个文件读取失败返回带 path 的 error，不静默跳过。

- [ ] **Step 4: 验证依赖边界并提交**

```bash
rg -n "std::fs|read_dir|read_to_string" crates/ui/src/theme.rs
cargo test -p edit-plus-ui theme::tests::
cargo test -p edit-plus-app theme_loader::tests::
```

Expected: `rg` 无输出；测试 PASS。

```bash
git add crates/ui/src/theme.rs crates/app/src/theme_loader.rs crates/app/src/lib.rs
git commit -m "refactor(ui): move theme file loading into app"
```

### Task 2: 定义稳定的 UiMetrics 显式输入

**Files:**
- Modify: `crates/ui/src/settings.rs`
- Modify: `crates/ui/src/lib.rs`

- [ ] **Step 1: 增加从 Settings 派生的不可变布局快照**

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiMetrics {
    pub dpi: f32,
    pub font_size: f32,
    pub line_height: f32,
    pub status_bar_height: f32,
    pub gutter_padding: f32,
    pub show_line_numbers: bool,
    pub show_status_bar: bool,
}

impl From<&Settings> for UiMetrics {
    fn from(settings: &Settings) -> Self {
        Self {
            dpi: settings.dpi_scale,
            font_size: settings.font_size,
            line_height: settings.line_height,
            status_bar_height: settings.status_bar_height,
            gutter_padding: settings.gutter_padding,
            show_line_numbers: settings.show_line_numbers,
            show_status_bar: settings.show_status_bar,
        }
    }
}
```

- [ ] **Step 2: 为派生值写纯测试**

测试 DPI=2、隐藏状态栏、隐藏行号，直接从局部 `Settings` 构造 `UiMetrics`，不得调用 `Settings::init/with`。

- [ ] **Step 3: 验证并提交**

```bash
cargo test -p edit-plus-ui settings::tests::ui_metrics -- --nocapture
git add crates/ui/src/settings.rs crates/ui/src/lib.rs
git commit -m "feat(ui): define explicit immutable UI metrics"
```

### Task 3: 迁移 tab bar

**Files:**
- Modify: `crates/ui/src/widgets/tab_bar/types.rs`
- Modify: `crates/ui/src/widgets/tab_bar/layout.rs`
- Modify: `crates/app/src/app_renderer.rs`

- [ ] **Step 1: tab bar 高度改成显式 DPI**

```rust
pub fn tab_bar_height(dpi: f32) -> f32 { 32.0 * dpi }
```

`layout_tabs` 已有 `TabBarCtx.dpi`，所有 font/padding 仅从 ctx 读取；删除 tab_bar 下 `Settings::with`。

- [ ] **Step 2: app 构造输入时传 metrics**

renderer 函数入口一次读取 app-owned settings，构造 `UiMetrics`，再完整创建：

```rust
let context = TabBarCtx {
    screen_w: self.screen_width(),
    screen_h: self.screen_height(),
    dpi: metrics.dpi,
};
```

不得让 ui widget 回读全局。

- [ ] **Step 3: 验证 tab bar 的 DPI 测试**

```bash
rg -n "Settings::" crates/ui/src/widgets/tab_bar
cargo test -p edit-plus-ui widgets::tab_bar::
cargo check -p edit-plus-app --all-targets
```

Expected: `rg` 无输出；PASS。

### Task 4: 迁移 sidebar 显式输入

**Files:**
- Modify: `crates/ui/src/widgets/sidebar/types.rs`
- Modify: `crates/ui/src/widgets/sidebar/state.rs`
- Modify: `crates/app/src/ui_shell.rs`

- [ ] **Step 1: 在 sidebar input 中加入所需配置快照**

```rust
#[derive(Debug, Clone, Copy)]
pub struct SidebarSettingsInput {
    pub dpi: f32,
    pub show_line_numbers: bool,
    pub word_wrap: bool,
    pub show_status_bar: bool,
    pub theme_mode: ThemeMode,
}
```

由 app 从自己的 Settings 构造；sidebar state/menu/hit testing 全部读取该 input。

- [ ] **Step 2: 删除 sidebar thread-local 读取**

把 `Settings::with` 中只取 dpi 的位置改用 `input.settings.dpi`；menu 使用 input 中四个设置值；修改设置仍返回 action，由 app 更新状态，ui 不调用 `Settings::with_mut`。

- [ ] **Step 3: 改写 widget tests**

测试直接构造 `SidebarSettingsInput`，删除 `Settings::init(Settings::new())`。覆盖 DPI=1/2、四个 toggle 当前值和不同线程构造相同输入得到相同布局。

- [ ] **Step 4: 验证并提交**

```bash
rg -n "Settings::" crates/ui/src/widgets/sidebar
cargo test -p edit-plus-ui widgets::sidebar::
cargo check -p edit-plus-app --all-targets
git add crates/ui/src/widgets/sidebar/types.rs crates/ui/src/widgets/sidebar/state.rs crates/app/src/ui_shell.rs
git commit -m "refactor(ui): inject sidebar settings input"
```

### Task 5: 清除 sidebar 入口与菜单的剩余全局读取

**Files:**
- Modify: `crates/ui/src/widgets/sidebar/menu.rs`
- Modify: `crates/ui/src/widgets/sidebar/mod.rs`
- Modify: `crates/app/src/ui_shell.rs`

- [ ] **Step 1: menu 接收 SidebarSettingsInput**

将 menu builder 签名增加 `settings: SidebarSettingsInput`，用其 `dpi/show_line_numbers/word_wrap/show_status_bar/theme_mode` 生成选中状态；删除 `Settings::with`。

- [ ] **Step 2: sidebar widget 入口传递同一份 settings input**

`sidebar/mod.rs` 的 hit-test 与热区计算使用 `input.settings.dpi`，调用 menu 时原样传 `input.settings`。app 的 `ui_shell` 构造唯一输入，避免同一帧多次读取可能不同的设置。

- [ ] **Step 3: 验证并提交**

```bash
rg -n "Settings::" crates/ui/src/widgets/sidebar
cargo test -p edit-plus-ui widgets::sidebar::
cargo check -p edit-plus-app --all-targets
git add crates/ui/src/widgets/sidebar/menu.rs crates/ui/src/widgets/sidebar/mod.rs crates/app/src/ui_shell.rs
git commit -m "refactor(ui): remove sidebar global settings reads"
```

### Task 6: 把 Settings 所有权移入 App 并删除 singleton

**Files:**
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/app_init.rs`
- Modify: `crates/ui/src/settings.rs`

- [ ] **Step 1: App 持有唯一 Settings 值**

```rust
pub(crate) settings: ui::settings::Settings,
```

初始化从 `settings_io` 加载并构造；mutation 方法通过 `&mut self.settings`，版本号与派生 line height 仍由 `Settings` 方法维护。

- [ ] **Step 2: 逐模块迁移 app 内 Settings::with**

每个提交最多改 3 个 app 文件，将读操作改成 `self.settings` 或函数参数 `settings: &Settings`，写操作改为 `self.settings.set_*`。纯 free function 只接收它实际需要的 `UiMetrics`/标量。按下列批次执行扫描与提交：

```bash
rg -n "Settings::(with|with_mut|init)" crates/app/src/app.rs crates/app/src/app_init.rs crates/app/src/app_lifecycle.rs
cargo test -p edit-plus-app --lib
git add crates/app/src/app.rs crates/app/src/app_init.rs crates/app/src/app_lifecycle.rs
git commit -m "refactor(app): own settings in application lifecycle"

rg -n "Settings::(with|with_mut|init)" crates/app/src/app_renderer.rs crates/app/src/render_pipeline.rs crates/app/src/md_preview.rs
git add crates/app/src/app_renderer.rs crates/app/src/render_pipeline.rs crates/app/src/md_preview.rs
git commit -m "refactor(app): pass settings into rendering"

rg -n "Settings::(with|with_mut|init)" crates/app/src/app_window.rs crates/app/src/events.rs crates/app/src/ui_shell.rs
git add crates/app/src/app_window.rs crates/app/src/events.rs crates/app/src/ui_shell.rs
git commit -m "refactor(app): pass settings into window UI"

rg -n "Settings::(with|with_mut|init)" crates/app/src/app_dispatch.rs crates/app/src/app_reshape.rs crates/app/src/app_scroll.rs
git add crates/app/src/app_dispatch.rs crates/app/src/app_reshape.rs crates/app/src/app_scroll.rs
git commit -m "refactor(app): pass settings into actions and reshape"
```

Expected: `rg` 无输出；测试 PASS。

- [ ] **Step 3: 删除 singleton API**

当全 workspace 搜索无调用后，删除 `Settings::with`、`with_mut`、`init`、`test_default` 和 `thread_local! SETTINGS`。Run:

```bash
rg -n "Settings::(with|with_mut|init)|thread_local!" crates/ui/src crates/app/src
cargo check --workspace --all-targets
cargo test --workspace
```

Expected: `rg` 无输出；构建测试 PASS。

- [ ] **Step 4: 提交所有权切换**

```bash
git add crates/app/src/app.rs crates/app/src/app_init.rs crates/ui/src/settings.rs
git commit -m "refactor(ui): make settings explicit app-owned state"
```

### Task 7: 缩小 ui 公共 API 并校正文档

**Files:**
- Modify: `crates/ui/src/lib.rs`
- Modify: `AGENTS.md`

- [ ] **Step 1: 只 re-export 稳定 widget 输入与 render API**

内部 layout/state/helper module 改为 `pub(crate)` 或 private；app 通过 `ui::widgets` 下每个 widget 模块的稳定 re-export 使用其 `Input`、`Action`、`Output` 类型，不直接依赖子模块内部类型。

- [ ] **Step 2: 更新架构说明**

把 Settings 描述从“全局单例”改为“app-owned 配置，通过 `UiMetrics`/widget input 注入”；theme 描述注明 ui 只解析 source，目录扫描位于 app。

- [ ] **Step 3: 总验收并提交**

```bash
rg -n "std::fs|Settings::(with|with_mut|init)" crates/ui/src
./scripts/verify.sh
git add crates/ui/src/lib.rs AGENTS.md
git commit -m "docs(ui): enforce explicit component boundaries"
```

Expected: `rg` 无输出，verify 全绿。
