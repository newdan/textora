# Code Quality Remediation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 补全 Phase 3 和 Phase 4 遗留的代码质量问题，收缩 `app` 与 `ui` 库的对外 API、清理编译 Warnings、分离 `Sidebar` 配置读取与 `ThemeRegistry` 异常处理、并整改 CI 门禁。

**Architecture:** 按原子化阶段实施，首先消除当前存在的无用依赖和引用，然后修改 `lib.rs` 的可见性，最后剥离 UI 库内部隐式的读取与抛错逻辑。所有的 `cargo check/test` 必须在每一步之后验证通过。

**Tech Stack:** Rust、cargo 检查命令、winit 相关生态

---

### Task 1: 编译 Warning 清理

**Files:**
- Modify: `crates/app/src/*`
- Modify: `crates/ui/src/*`

- [ ] **Step 1: 批量修复未使用的导入**
通过 `cargo fix` 命令批量清理所有冗余的导入及冗余可变绑定。
```bash
cargo fix --lib -p edit-plus-ui --allow-dirty --allow-no-vcs
cargo fix --lib -p edit-plus-app --allow-dirty --allow-no-vcs
```

- [ ] **Step 2: 验证编译并提交**
```bash
cargo check --workspace --all-targets
git add crates/ui/src/ crates/app/src/
git commit -m "style: resolve unused imports and mutability warnings"
```
*Expected: 0 warnings*

### Task 2: 收缩 App 与 UI 公共 API 

**Files:**
- Modify: `crates/app/src/lib.rs`
- Modify: `crates/ui/src/lib.rs`

- [ ] **Step 1: App Crate API 收缩**
在 `crates/app/src/lib.rs` 中，将不属于对外公共接口的 `pub mod`（例如 `actions`, `app`, `app_dispatch`, `app_init` 等内部生命周期与逻辑模块）全部修改为 `pub(crate) mod`。
只保留：
```rust
pub use app::App;
pub use app_event::AppEvent;
pub use gpu::{GpuError, headless_init};
```

- [ ] **Step 2: UI Crate API 收缩**
在 `crates/ui/src/lib.rs` 中，将 `layout`, `render_geom`, `view_mode`, `text_renderer` 等内部工具模块从 `pub mod` 修改为 `pub(crate) mod`。

- [ ] **Step 3: 验证并提交**
```bash
cargo check --workspace --all-targets
git add crates/app/src/lib.rs crates/ui/src/lib.rs
git commit -m "refactor: enforce module visibility boundaries for app and ui"
```

### Task 3: SidebarSettingsInput 与 UiMetrics 职责拆分

**Files:**
- Modify: `crates/ui/src/widgets/sidebar/types.rs`
- Modify: `crates/ui/src/widgets/sidebar/state.rs`
- Modify: `crates/app/src/ui_shell.rs`

- [ ] **Step 1: 确认 SidebarSettingsInput 的输入独立性**
在 `sidebar/types.rs` 确保 `SidebarSettingsInput` 包含完整独立的值类型（如 `dpi`, `show_line_numbers`, `word_wrap`, `show_status_bar`, `theme_mode`）。

- [ ] **Step 2: 消除隐式回读**
在 `sidebar/state.rs` 或 `menu.rs` 中，确保没有直接回读 `UiMetrics` 行为状态（比如检查主题等），而是全部消费并响应来自 `app` 层（`ui_shell.rs`）传入的 `SidebarSettingsInput` 结构。

- [ ] **Step 3: 验证并提交**
```bash
cargo test -p edit-plus-ui widgets::sidebar::
cargo check -p edit-plus-app
git add crates/ui/src/widgets/sidebar/ crates/app/src/ui_shell.rs
git commit -m "refactor(ui): decouple sidebar input from general ui metrics"
```

### Task 4: ThemeRegistry 错误处理与 IO 剥离

**Files:**
- Modify: `crates/ui/src/theme.rs`
- Modify: `crates/app/src/theme_loader.rs`

- [ ] **Step 1: 消除 ThemeRegistry 中的 IO 与 panic**
审查 `crates/ui/src/theme.rs` 中是否有 `expect`，或静默忽略错误的做法。确保解析逻辑只接收 `ThemeSource` 的字符串内容，不自行调用 `std::fs` 去读取。所有的失败情况必须通过 `Result` 冒泡。

- [ ] **Step 2: 完善 app 层的 theme loader 异常捕获**
在 `crates/app/src/theme_loader.rs` 汇总读取，并把文件级别的 IO 错误记录到日志。

- [ ] **Step 3: 验证并提交**
```bash
rg -n "std::fs|expect\(" crates/ui/src/theme.rs
cargo test -p edit-plus-ui theme::tests::
git add crates/ui/src/theme.rs crates/app/src/theme_loader.rs
git commit -m "refactor(ui): remove IO and panics from theme registry"
```

### Task 5: 提交粒度和 CI 门禁整改

**Files:**
- Modify/Create: `scripts/verify.sh`

- [ ] **Step 1: 配置强制警告门禁**
创建或更新 `scripts/verify.sh` 脚本，保证其包含以下行以实现 CI 层面的防呆机制：
```bash
#!/usr/bin/env bash
set -e

echo "Running tests..."
cargo test --workspace

echo "Checking for warnings..."
cargo clippy --workspace --all-targets -- -D warnings
```

- [ ] **Step 2: 赋予执行权限并验证提交**
```bash
chmod +x scripts/verify.sh
./scripts/verify.sh
git add scripts/verify.sh
git commit -m "chore: strictly enforce no-warnings on CI"
```
