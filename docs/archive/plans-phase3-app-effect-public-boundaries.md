# Phase 3 AppEffect / Dispatch / Public Boundaries Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** 让 AppAction dispatch 只产生可合并 AppEffect 并由顶层应用一次，同时把 edit-plus-app 的公共 API 收缩为真实外部契约。

**Architecture:** 先扩展无分配 AppEffect 与 Settings persistence，再逐域迁移 tabs、editor、chrome、viewport 和 scroll，使所有 handler 只返回 effect。最后用静态测试锁定 single-apply 边界，并通过 root re-export 与隐藏 dev_support 收缩 crate public surface。

**Tech Stack:** Rust、winit、现有 AppAction/AppEffect/WorkspaceEffect/Settings、app 单元测试、integration tests、Criterion benchmarks。

---

**设计依据：** docs/superpowers/specs/2026-06-20-phase3-app-effect-public-boundaries-design.md

**前置条件：**

- 完成 docs/plans-settings-dpi-remediation.md。
- 完成 docs/plans-logical-settings-physical-metrics.md。
- Settings 在本计划开始时已保存逻辑尺寸，App::ui_metrics() 已是物理尺寸唯一派生入口。

**共同验收约束：**

- 每个任务最多修改 3 个文件。
- 行为变化先写失败测试，再写最小实现。
- 每次提交前运行定向测试和 cargo check -p edit-plus-app。
- 公共 API 任务运行 cargo check -p edit-plus-app --all-targets。
- 不修改 lifecycle/render loop 的自主 redraw 调度。
- 不把文件 I/O、对话框、剪贴板包装进 AppEffect。
- 不处理 Phase 4 UI 边界、ThemeRegistry、warning 或 CI 门禁。

## 文件职责映射

- crates/app/src/app_effect.rs：effect 字段、常量、merge 代数与固定执行步骤。
- crates/app/src/app.rs：apply_effect、窗口 chrome 同步和顶层 effect 执行。
- crates/app/src/settings_io.rs：PersistedSettings 与逻辑 Settings 的单一映射/保存入口。
- crates/app/src/app_dispatch.rs：AppAction 路由、single apply、IME post-hook。
- crates/app/src/dispatch/commands.rs：AppCommand effect 合并。
- crates/app/src/dispatch/editor.rs：EditCommand、编辑领域状态与 effect。
- crates/app/src/dispatch/tabs.rs：WorkspaceEffect、tab/file/dialog。
- crates/app/src/dispatch/chrome.rs：popup、sidebar、tab chrome、settings toggles。
- crates/app/src/dispatch/viewport.rs：scrollbar、viewport、preview/TOC scroll。
- crates/app/src/app_scroll.rs：滚轮和光标滚动的领域 mutation。
- crates/app/src/dispatch_boundary_tests.rs：源码级 single-apply/禁止直调门禁。
- crates/app/src/lib.rs：内部模块声明、root re-export、dev_support。
- crates/app/src/main.rs：只消费 root public API。
- crates/app/tests/render_smoke.rs：通过 dev_support 使用 MeasureFromShaper。
- crates/app/benches/*.rs：通过 dev_support 使用 benchmark 类型。

## 迁移不变量

1. AppEffect 只表达可合并 follow-up，不携带路径、错误、cursor 或闭包。
2. dispatch 迁移期间，每个 AppAction 的用户可见行为和调用顺序保持不变。
3. handler 可以修改领域状态和执行同步 I/O，但不能 apply AppEffect。
4. 最终只有 App::dispatch 在 AppAction 调用链中调用一次 apply_effect。
5. lifecycle 中现有 apply_effect/needs_redraw 不属于本计划静态禁区。
6. 公共模块私有化必须在 main、tests、bench 全部迁移后执行。

### Task 1: 扩展 AppEffect 字段、常量和执行顺序

**Files:**
- Modify/Test: crates/app/src/app_effect.rs

- [ ] **Step 1: 写新增字段与代数失败测试**

在 app_effect.rs 测试模块加入：

~~~rust
#[test]
fn merge_includes_persistence_and_window_chrome() {
    let effect = AppEffect::PERSIST_SETTINGS
        .merge(AppEffect::PERSIST_WORKSPACE)
        .merge(AppEffect::SYNC_WINDOW_CHROME);

    assert!(effect.persist_settings);
    assert!(effect.persist_workspace);
    assert!(effect.sync_window_chrome);
    assert!(effect.redraw);
}

#[test]
fn merge_obeys_boolean_union_laws() {
    let x = AppEffect::RESHAPE.merge(AppEffect::PERSIST_SETTINGS);
    let y = AppEffect::UPDATE_TITLE.merge(AppEffect::PERSIST_WORKSPACE);
    let z = AppEffect::SYNC_WINDOW_CHROME;

    assert_eq!(x.merge(AppEffect::NONE), x);
    assert_eq!(x.merge(x), x);
    assert_eq!(x.merge(y), y.merge(x));
    assert_eq!(x.merge(y).merge(z), x.merge(y.merge(z)));
}

#[test]
fn execution_steps_have_fixed_order() {
    let effect = AppEffect::RESHAPE
        .merge(AppEffect::SYNC_WINDOW_CHROME)
        .merge(AppEffect::UPDATE_TITLE)
        .merge(AppEffect::PERSIST_SETTINGS)
        .merge(AppEffect::PERSIST_WORKSPACE);

    assert_eq!(
        effect.steps().collect::<Vec<_>>(),
        vec![
            AppEffectStep::Reshape,
            AppEffectStep::SyncWindowChrome,
            AppEffectStep::UpdateTitle,
            AppEffectStep::PersistSettings,
            AppEffectStep::PersistWorkspace,
            AppEffectStep::Redraw,
        ]
    );
}
~~~

- [ ] **Step 2: 运行测试并确认缺少字段/步骤**

~~~bash
cargo test -p edit-plus-app --lib app_effect::tests -- --nocapture
~~~

Expected: FAIL，PERSIST_SETTINGS、SYNC_WINDOW_CHROME 或 steps 不存在。

- [ ] **Step 3: 实现完整 effect**

~~~rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppEffectStep {
    Reshape,
    SyncWindowChrome,
    UpdateTitle,
    PersistSettings,
    PersistWorkspace,
    Redraw,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AppEffect {
    pub(crate) redraw: bool,
    pub(crate) reshape: bool,
    pub(crate) update_title: bool,
    pub(crate) persist_workspace: bool,
    pub(crate) persist_settings: bool,
    pub(crate) sync_window_chrome: bool,
}

impl AppEffect {
    pub(crate) const NONE: Self = Self {
        redraw: false,
        reshape: false,
        update_title: false,
        persist_workspace: false,
        persist_settings: false,
        sync_window_chrome: false,
    };
    pub(crate) const REDRAW: Self = Self { redraw: true, ..Self::NONE };
    pub(crate) const RESHAPE: Self =
        Self { redraw: true, reshape: true, ..Self::NONE };
    pub(crate) const UPDATE_TITLE: Self =
        Self { redraw: true, update_title: true, ..Self::NONE };
    pub(crate) const PERSIST_WORKSPACE: Self =
        Self { persist_workspace: true, ..Self::NONE };
    pub(crate) const PERSIST_SETTINGS: Self =
        Self { persist_settings: true, ..Self::NONE };
    pub(crate) const SYNC_WINDOW_CHROME: Self =
        Self { redraw: true, sync_window_chrome: true, ..Self::NONE };

    pub(crate) const fn merge(self, other: Self) -> Self {
        Self {
            redraw: self.redraw || other.redraw,
            reshape: self.reshape || other.reshape,
            update_title: self.update_title || other.update_title,
            persist_workspace: self.persist_workspace || other.persist_workspace,
            persist_settings: self.persist_settings || other.persist_settings,
            sync_window_chrome: self.sync_window_chrome || other.sync_window_chrome,
        }
    }

    pub(crate) fn steps(self) -> impl Iterator<Item = AppEffectStep> {
        [
            self.reshape.then_some(AppEffectStep::Reshape),
            self.sync_window_chrome.then_some(AppEffectStep::SyncWindowChrome),
            self.update_title.then_some(AppEffectStep::UpdateTitle),
            self.persist_settings.then_some(AppEffectStep::PersistSettings),
            self.persist_workspace.then_some(AppEffectStep::PersistWorkspace),
            self.redraw.then_some(AppEffectStep::Redraw),
        ]
        .into_iter()
        .flatten()
    }
}
~~~

- [ ] **Step 4: 运行测试和编译**

~~~bash
cargo test -p edit-plus-app --lib app_effect::tests -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/app_effect.rs
git commit -m "refactor(app): define complete effect algebra"
~~~

### Task 2: 建立逻辑 Settings 的单一持久化映射

**Files:**
- Modify/Test: crates/app/src/settings_io.rs

- [ ] **Step 1: 写全字段映射和几何保留失败测试**

在 settings_io.rs 测试模块加入：

~~~rust
#[test]
fn apply_editor_settings_updates_editor_fields_only() {
    let mut persisted = PersistedSettings {
        sidebar_width: 333.0,
        window_x: Some(10),
        window_y: Some(20),
        window_width: Some(900),
        window_height: Some(700),
        ..PersistedSettings::default()
    };
    let mut settings = ui::settings::Settings::new();
    settings.view_mode = ViewMode::Tabs;
    settings.theme_mode = ThemeMode::Dark;
    settings.show_line_numbers = false;
    settings.word_wrap = false;
    settings.show_status_bar = true;
    settings.font_family = "Test Mono".into();
    settings.font_size = 19.0;
    settings.line_height_ratio = 1.5;
    settings.line_height = 28.5;
    settings.tab_width = 8;

    persisted.apply_editor_settings(&settings);

    assert_eq!(persisted.view_mode, ViewMode::Tabs);
    assert_eq!(persisted.theme_mode, ThemeMode::Dark);
    assert!(!persisted.show_line_numbers);
    assert!(!persisted.word_wrap);
    assert!(persisted.show_status_bar);
    assert_eq!(persisted.font_family, "Test Mono");
    assert_eq!(persisted.font_size, 19.0);
    assert_eq!(persisted.line_height_ratio, 1.5);
    assert_eq!(persisted.tab_width, 8);
    assert_eq!(persisted.sidebar_width, 333.0);
    assert_eq!(persisted.window_x, Some(10));
    assert_eq!(persisted.window_y, Some(20));
    assert_eq!(persisted.window_width, Some(900));
    assert_eq!(persisted.window_height, Some(700));
}
~~~

- [ ] **Step 2: 运行测试并确认方法不存在**

~~~bash
cargo test -p edit-plus-app --lib settings_io::tests::apply_editor_settings_updates_editor_fields_only -- --exact
~~~

Expected: FAIL，apply_editor_settings 不存在。

- [ ] **Step 3: 实现映射和保存入口**

~~~rust
impl PersistedSettings {
    pub(crate) fn apply_editor_settings(
        &mut self,
        settings: &ui::settings::Settings,
    ) {
        self.view_mode = settings.view_mode;
        self.theme_mode = settings.theme_mode;
        self.show_line_numbers = settings.show_line_numbers;
        self.word_wrap = settings.word_wrap;
        self.show_status_bar = settings.show_status_bar;
        self.font_family = settings.font_family.clone();
        self.font_size = settings.font_size;
        self.line_height_ratio = settings.line_height_ratio;
        self.tab_width = settings.tab_width;
    }
}

pub(crate) fn save_editor_settings(
    settings: &ui::settings::Settings,
) -> std::io::Result<()> {
    let mut persisted = load()?;
    persisted.apply_editor_settings(settings);
    save(&persisted)
}

pub(crate) fn ensure_exists() -> std::io::Result<()> {
    let path = settings_toml_path();
    if path.exists() {
        Ok(())
    } else {
        save_to(&path, &PersistedSettings::default())
    }
}
~~~

不得覆盖 sidebar 或 window geometry 字段。

- [ ] **Step 4: 运行 settings_io 测试和编译**

~~~bash
cargo test -p edit-plus-app --lib settings_io::tests -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/settings_io.rs
git commit -m "refactor(app): centralize settings persistence mapping"
~~~

### Task 3: 让 apply_effect 按固定步骤执行

**Files:**
- Modify/Test: crates/app/src/app.rs
- Modify/Test: crates/app/src/app_tests.rs

- [ ] **Step 1: 写 reshape/redraw 与无窗口 chrome 测试**

在 app_tests.rs 加入：

~~~rust
#[test]
fn apply_effect_runs_reshape_before_redraw_without_window() {
    let mut app = App::new(None);
    app.needs_redraw = false;
    let generation = app.reshape_generation;

    app.apply_effect(AppEffect::RESHAPE);

    assert_eq!(app.reshape_generation, generation + 1);
    assert!(app.needs_redraw);
}

#[test]
fn apply_window_chrome_effect_is_safe_without_window() {
    let mut app = App::new(None);
    app.needs_redraw = false;

    app.apply_effect(AppEffect::SYNC_WINDOW_CHROME);

    assert!(app.needs_redraw);
}
~~~

测试模块增加 use crate::app_effect::AppEffect。

- [ ] **Step 2: 运行测试并确认新增步骤未执行**

~~~bash
cargo test -p edit-plus-app --lib app::app_tests::apply_window_chrome_effect_is_safe_without_window -- --exact
~~~

Expected: FAIL，SYNC_WINDOW_CHROME 不被旧 apply_effect 处理。

- [ ] **Step 3: 实现 persistence/chrome helper 与步骤循环**

在 App impl 中加入：

~~~rust
pub(crate) fn persist_editor_settings(&self) -> std::io::Result<()> {
    crate::settings_io::save_editor_settings(&self.settings)
}

pub(crate) fn sync_window_chrome(&self) {
    let Some(window) = self.window.as_ref() else {
        return;
    };
    match self.settings.view_mode {
        ui::view_mode::ViewMode::Sidebar => {
            crate::sys::macos_titlebar::enable_full_size_content(window);
        }
        ui::view_mode::ViewMode::Tabs => {
            crate::sys::macos_titlebar::disable_full_size_content(window);
        }
    }
}
~~~

apply_effect 改为：

~~~rust
pub(crate) fn apply_effect(&mut self, effect: crate::app_effect::AppEffect) {
    use crate::app_effect::AppEffectStep;

    for step in effect.steps() {
        match step {
            AppEffectStep::Reshape => self.invalidate_reshape(),
            AppEffectStep::SyncWindowChrome => self.sync_window_chrome(),
            AppEffectStep::UpdateTitle => self.update_window_title(),
            AppEffectStep::PersistSettings => {
                if let Err(error) = self.persist_editor_settings() {
                    eprintln!("[settings] save error: {error}");
                }
            }
            AppEffectStep::PersistWorkspace => self.persist_workspace_state(),
            AppEffectStep::Redraw => {
                self.needs_redraw = true;
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
        }
    }
}
~~~

循环内不得 return；settings 保存失败后仍继续 workspace/redraw。

- [ ] **Step 4: 运行 App 测试和编译**

~~~bash
cargo test -p edit-plus-app --lib app::app_tests::apply_effect -- --nocapture
cargo test -p edit-plus-app --lib app_effect::tests -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/app.rs crates/app/src/app_tests.rs
git commit -m "refactor(app): execute effects in one ordered pipeline"
~~~

### Task 4: 删除 command/tab 路径的嵌套 apply

**Files:**
- Modify/Test: crates/app/src/app_dispatch.rs
- Modify/Test: crates/app/src/dispatch/tabs.rs

- [ ] **Step 1: 写 batch close 只返回 effect 的失败测试**

在 dispatch/tabs.rs 测试模块加入：

~~~rust
#[test]
fn batch_close_returns_effect_without_applying_it() {
    let mut app = App::new(None);
    app.workspace.new_empty_tab(600.0);
    app.workspace.new_empty_tab(600.0);
    app.workspace.switch_to(0);
    app.needs_redraw = false;

    let effect = app.execute_batch_close(&[1]);

    assert!(effect.redraw);
    assert!(effect.persist_workspace);
    assert!(!app.needs_redraw);
}
~~~

- [ ] **Step 2: 运行测试并确认 execute_batch_close 返回 ()**

~~~bash
cargo test -p edit-plus-app --lib dispatch::tabs::tests::batch_close_returns_effect_without_applying_it -- --exact
~~~

Expected: FAIL，无法从 () 读取 effect 字段。

- [ ] **Step 3: 让所有嵌套入口返回 effect**

dispatch/tabs.rs：

~~~rust
pub(crate) fn execute_batch_close(
    &mut self,
    indices: &[usize],
) -> AppEffect {
    if indices.is_empty() {
        return AppEffect::NONE;
    }
    let mut sorted = indices.to_vec();
    sorted.sort_by(|left, right| right.cmp(left));
    for &index in &sorted {
        self.record_tab_to_history(index);
    }
    let mut workspace_effect = crate::workspace::WorkspaceEffect::None;
    for &index in &sorted {
        if let Ok(next) = self.workspace.close_tab(index) {
            workspace_effect = workspace_effect.merge(next);
        }
    }
    self.save_history();
    self.handle_workspace_effect(workspace_effect)
        .merge(AppEffect::REDRAW)
}

pub(crate) fn open_settings_file(&mut self) -> AppEffect {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let path = std::path::PathBuf::from(home)
        .join(".edit+")
        .join("settings.toml");
    if let Err(error) = crate::settings_io::ensure_exists() {
        eprintln!("[settings] save error: {error}");
        return AppEffect::NONE;
    }
    match self.open_file(&path) {
        Ok(effect) => effect,
        Err(error) => {
            eprintln!("Failed to open settings.toml: {error}");
            AppEffect::NONE
        }
    }
}

pub(crate) fn dispatch_context_menu_action(
    &mut self,
    action: ui::widgets::popup_menu::ContextMenuAction,
    tab_index: usize,
) -> AppEffect {
    use ui::widgets::popup_menu::ContextMenuAction;
    match action {
        ContextMenuAction::Close => self.try_close_tab_with_prompt(tab_index),
        ContextMenuAction::CloseOthers
        | ContextMenuAction::CloseRight
        | ContextMenuAction::CloseAll => {
            self.try_close_multiple_with_prompt(action, tab_index)
        }
        _ => {
            let workspace_effect =
                self.workspace.execute_context_menu_action(action, tab_index);
            self.handle_workspace_effect(workspace_effect)
        }
    }
}
~~~

app_dispatch.rs：

~~~rust
pub(crate) fn execute_commands(
    &mut self,
    commands: Vec<AppCommand>,
    event_loop: &ActiveEventLoop,
) -> AppEffect {
    commands.into_iter().fold(AppEffect::NONE, |effect, command| {
        effect.merge(self.dispatch_app_command(command, event_loop))
    })
}

pub(crate) fn dispatch_menu_action(
    &mut self,
    action: crate::native_menu::MenuAction,
    event_loop: &ActiveEventLoop,
) {
    let commands = crate::menu_handler::dispatch_menu_action(action);
    self.dispatch(AppAction::ExecuteAppCommands(commands), event_loop);
}
~~~

在 Task 12 合并 single apply 前，AppAction::ExecuteAppCommands 分支临时保持：

~~~rust
AppAction::ExecuteAppCommands(commands) => {
    let effect = self.execute_commands(commands, event_loop);
    self.apply_effect(effect);
}
~~~

apply_effect 仍位于顶层 AppAction match，不得放回 execute_commands 或 dispatch 子模块。

- [ ] **Step 4: 扫描并运行测试**

~~~bash
rg -n "apply_effect\(" crates/app/src/dispatch/tabs.rs
cargo test -p edit-plus-app --lib dispatch::tabs -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: 扫描无输出；测试和编译 PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/app_dispatch.rs crates/app/src/dispatch/tabs.rs
git commit -m "refactor(app): return effects from command and tab dispatch"
~~~

### Task 5: 将编辑完成后的 reshape 延迟到 apply_effect

**Files:**
- Modify/Test: crates/app/src/app.rs
- Modify/Test: crates/app/src/dispatch/editor.rs

- [ ] **Step 1: 写 cursor-only reset 失败测试**

在 app.rs 增加测试模块：

~~~rust
#[cfg(test)]
mod edit_reset_tests {
    use super::reset_cursor_after_edit;
    use crate::cursor_motion::CursorRenderState;

    #[test]
    fn cursor_reset_does_not_own_reshape_generation() {
        let mut state = CursorRenderState::default();
        state.sticky_x_dirty = false;
        let before = state.cursor_blink_instant;

        reset_cursor_after_edit(&mut state);

        assert!(state.sticky_x_dirty);
        assert!(state.cursor_blink_instant >= before);
    }
}
~~~

- [ ] **Step 2: 运行测试并确认 helper 不存在**

~~~bash
cargo test -p edit-plus-app --lib app::edit_reset_tests::cursor_reset_does_not_own_reshape_generation -- --exact
~~~

Expected: FAIL，reset_cursor_after_edit 不存在。

- [ ] **Step 3: 分离 cursor reset 与 reshape effect**

用以下函数替换 reset_after_edit：

~~~rust
pub(crate) fn reset_cursor_after_edit(
    cursor_render_state: &mut crate::cursor_motion::CursorRenderState,
) {
    cursor_render_state.sticky_x_dirty = true;
    cursor_render_state.cursor_blink_instant = std::time::Instant::now();
}
~~~

dispatch/editor.rs 删除 reset_after_edit import。编辑成功分支改为：

~~~rust
reset_cursor_after_edit(&mut dv.cursor_render_state);
effect = effect.merge(AppEffect::RESHAPE);
~~~

删除对 reshape_generation、pending_reshapes、reshape_worker 的直接传入。

- [ ] **Step 4: 运行 editor/app 测试和静态扫描**

~~~bash
rg -n "reset_after_edit|reshape_generation|pending_reshapes|invalidate_reshape" crates/app/src/dispatch/editor.rs
cargo test -p edit-plus-app --lib dispatch::editor -- --nocapture
cargo test -p edit-plus-app --lib app::edit_reset_tests -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: 扫描无输出；测试和编译 PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/app.rs crates/app/src/dispatch/editor.rs
git commit -m "refactor(app): defer edit reshape through effects"
~~~

### Task 6: 让 Zoom 返回 effect 而不是直接应用

**Files:**
- Modify/Test: crates/app/src/app_reshape.rs
- Modify/Test: crates/app/src/dispatch/commands.rs

- [ ] **Step 1: 改写 Zoom 测试为“返回但不应用”**

在 app_reshape.rs 测试模块加入：

~~~rust
#[test]
fn apply_zoom_returns_reshape_without_applying_it() {
    let mut app = App::new(None);
    let generation = app.reshape_generation;
    app.needs_redraw = false;

    let effect = app.apply_zoom(20.0);

    assert!(effect.reshape);
    assert!(effect.redraw);
    assert_eq!(app.settings.font_size, 20.0);
    assert_eq!(app.reshape_generation, generation);
    assert!(!app.needs_redraw);
}
~~~

- [ ] **Step 2: 运行测试并确认返回类型/副作用失败**

~~~bash
cargo test -p edit-plus-app --lib app_reshape::tests::apply_zoom_returns_reshape_without_applying_it -- --exact
~~~

Expected: FAIL，apply_zoom 返回 () 且直接修改 generation/redraw。

- [ ] **Step 3: 改造 Zoom**

~~~rust
pub(crate) fn apply_zoom(
    &mut self,
    logical_font_size: f32,
) -> crate::app_effect::AppEffect {
    let logical_font_size = logical_font_size.clamp(6.0, 72.0);
    self.settings.set_font_size(logical_font_size);
    let metrics = self.ui_metrics();
    if let Some(text) = self.text.as_mut() {
        text.shaper.set_font_size(metrics.font_size);
    }
    for index in 0..self.workspace.len() {
        self.workspace
            .view_mut(index)
            .unwrap()
            .doc_mut()
            .display
            .render_cache
            .invalidate_all();
    }
    let screen_height = self.screen_height();
    let visible_rows = self.visible_rows(screen_height);
    let viewport_height = self.visible_height_lines(screen_height);
    for index in 0..self.workspace.len() {
        let document = self.workspace.view_mut(index).unwrap().doc_mut();
        document.resize(visible_rows, viewport_height);
        document
            .display
            .viewport
            .clamp_anchor(&document.display.display_map, metrics.line_height);
        document
            .display
            .viewport
            .derive_scroll_top(&document.display.display_map, metrics.line_height);
    }
    crate::app_effect::AppEffect::RESHAPE
        .merge(crate::app_effect::AppEffect::PERSIST_SETTINGS)
}
~~~

commands.rs 中 ZoomIn/ZoomOut/ZoomReset 分支改为：

~~~rust
crate::menu_handler::AppCommand::ZoomIn => {
    effect = effect.merge(self.apply_zoom(self.settings.font_size + 1.0));
}
crate::menu_handler::AppCommand::ZoomOut => {
    effect = effect.merge(self.apply_zoom(
        (self.settings.font_size - 1.0).max(6.0),
    ));
}
crate::menu_handler::AppCommand::ZoomReset => {
    effect = effect.merge(self.apply_zoom(15.0));
}
~~~

- [ ] **Step 4: 运行 Zoom/command 测试和编译**

~~~bash
cargo test -p edit-plus-app --lib app_reshape::tests -- --nocapture
cargo test -p edit-plus-app --lib dispatch::commands -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/app_reshape.rs crates/app/src/dispatch/commands.rs
git commit -m "refactor(app): return zoom effects"
~~~

### Task 7: 提取 popup/tab/sidebar chrome handler

**Files:**
- Create/Test: crates/app/src/dispatch/chrome.rs
- Modify: crates/app/src/app_dispatch.rs
- Modify: crates/app/src/lib.rs

- [ ] **Step 1: 写 sidebar width/pin effect 失败测试**

在新 chrome.rs 先写：

~~~rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_width_and_pin_return_redraw_without_applying() {
        let mut app = App::new(None);
        app.needs_redraw = false;

        let width_effect =
            app.dispatch_chrome_action(ChromeDispatchAction::SetSidebarWidth(260.0));
        let pin_effect =
            app.dispatch_chrome_action(ChromeDispatchAction::ToggleSidebarPin);

        assert_eq!(app.ui_shell.sidebar_width(), 260.0);
        assert!(width_effect.redraw);
        assert!(pin_effect.redraw);
        assert!(!app.needs_redraw);
    }

    #[test]
    fn sidebar_resize_end_requests_workspace_persistence() {
        let mut app = App::new(None);
        let effect =
            app.dispatch_chrome_action(ChromeDispatchAction::SidebarResizeEnd);
        assert!(effect.persist_workspace);
    }
}
~~~

测试模块最终保留在 chrome.rs 文件末尾，生产实现放在 #[cfg(test)] 之前。

- [ ] **Step 2: 注册空模块并确认类型不存在**

在 lib.rs dispatch 内增加 pub(crate) mod chrome；运行：

~~~bash
cargo test -p edit-plus-app --lib dispatch::chrome::tests -- --nocapture
~~~

Expected: FAIL，ChromeDispatchAction/dispatch_chrome_action 不存在。

- [ ] **Step 3: 定义 chrome action 并移动基础分支**

~~~rust
use crate::app::App;
use crate::app_effect::AppEffect;

pub(crate) enum TabScrollDirection {
    Left,
    Right,
}

pub(crate) enum ChromeDispatchAction {
    OpenPopup(ui::widgets::popup_menu::PopupMenu),
    ClearPopup,
    OpenOverflow,
    ScrollTab(TabScrollDirection),
    HoverTab(Option<usize>),
    SidebarResizeStart,
    SidebarResizeEnd,
    SetSidebarWidth(f32),
    ToggleSidebarPin,
    OpenSidebarSettingsMenu,
}

impl App {
    pub(crate) fn dispatch_chrome_action(
        &mut self,
        action: ChromeDispatchAction,
    ) -> AppEffect {
        match action {
            ChromeDispatchAction::OpenPopup(menu) => {
                let rect = menu.menu_rect;
                self.ui_shell
                    .push_overlay(Box::new(ui::PopupMenuWidget::new(menu)), rect);
                AppEffect::REDRAW
            }
            ChromeDispatchAction::ClearPopup => {
                self.ui_shell.clear_overlays();
                AppEffect::REDRAW
            }
            ChromeDispatchAction::OpenOverflow => {
                let screen = (self.screen_width(), self.screen_height());
                let metrics = self.ui_metrics();
                if let Some(layout) = self.ui_shell.tab_bar_layout() {
                    let entries = layout
                        .tabs
                        .iter()
                        .map(|entry| ui::widgets::popup_menu::OverflowEntry {
                            tab_index: entry.index,
                            title: entry.title.clone(),
                        })
                        .collect::<Vec<_>>();
                    if layout.dropdown_rect_px.w > 0.0 {
                        let menu = ui::widgets::popup_menu::PopupMenu::overflow_px(
                            &entries,
                            layout.dropdown_rect_px,
                            screen,
                            self.workspace.active_index(),
                            metrics.dpi,
                        );
                        let rect = menu.menu_rect;
                        self.ui_shell.push_overlay(
                            Box::new(ui::PopupMenuWidget::new(menu)),
                            rect,
                        );
                    }
                }
                AppEffect::REDRAW
            }
            ChromeDispatchAction::ScrollTab(direction) => {
                if let Some(layout) = self.ui_shell.tab_bar_layout() {
                    let viewport_width =
                        layout.clip_right_px - layout.clip_left_px;
                    let step = viewport_width * 0.7;
                    let target = match direction {
                        TabScrollDirection::Left => {
                            self.workspace.tab_scroll_target - step
                        }
                        TabScrollDirection::Right => {
                            self.workspace.tab_scroll_target + step
                        }
                    };
                    self.workspace
                        .start_scroll_animation(target, layout.max_scroll);
                }
                self.update_tab_layout(false).merge(AppEffect::REDRAW)
            }
            ChromeDispatchAction::HoverTab(index) => {
                self.ui_shell.set_tab_bar_hovered(index);
                AppEffect::NONE
            }
            ChromeDispatchAction::SetSidebarWidth(width) => {
                self.ui_shell.sidebar_cfg_mut().width = width;
                AppEffect::REDRAW
            }
            ChromeDispatchAction::SidebarResizeEnd => {
                AppEffect::PERSIST_WORKSPACE
            }
            ChromeDispatchAction::SidebarResizeStart => AppEffect::NONE,
            ChromeDispatchAction::ToggleSidebarPin => {
                let pinned = !self.ui_shell.sidebar_pinned();
                self.ui_shell.set_sidebar_pinned(pinned);
                self.ui_shell.sidebar_persistent.visibility = if pinned {
                    ui::widgets::sidebar::Visibility::Pinned
                } else {
                    ui::widgets::sidebar::Visibility::Hidden
                };
                if !pinned {
                    self.ui_shell.sidebar_persistent.suppress_hover_enter = true;
                }
                AppEffect::REDRAW
            }
            ChromeDispatchAction::OpenSidebarSettingsMenu => {
                let screen_width = self.screen_width();
                let screen_height = self.screen_height();
                let button = self.ui_shell.sidebar_persistent.settings_btn_rect;
                let button = (button.w > 0.0 && button.h > 0.0)
                    .then_some(button);
                let metrics = self.ui_metrics();
                let input =
                    ui::widgets::sidebar::SidebarSettingsInput::from(
                        &self.settings,
                    );
                self.ui_shell.sidebar_persistent.open_menu =
                    ui::widgets::sidebar::build_settings_menu(
                        button,
                        &input,
                        screen_width,
                        screen_height,
                        &metrics,
                    );
                self.ui_shell.sidebar_persistent.menu_leave_at = None;
                AppEffect::REDRAW
            }
        }
    }
}
~~~

app_dispatch.rs 对应 AppAction 分支只负责构造 ChromeDispatchAction 和处理返回的 effect。

Task 12 前，每个对应 match arm使用同一临时形式保持行为：

~~~rust
let effect = self.dispatch_chrome_action(chrome_action);
self.apply_effect(effect);
~~~

apply_effect 只允许出现在 app_dispatch.rs，不允许进入 chrome.rs。

- [ ] **Step 4: 运行 chrome 测试和编译**

~~~bash
cargo test -p edit-plus-app --lib dispatch::chrome::tests -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/dispatch/chrome.rs crates/app/src/app_dispatch.rs crates/app/src/lib.rs
git commit -m "refactor(app): extract chrome dispatch"
~~~

### Task 8: 将 Settings actions 迁入 chrome effect

**Files:**
- Modify/Test: crates/app/src/dispatch/chrome.rs
- Modify: crates/app/src/app_dispatch.rs
- Modify: crates/app/src/dispatch/editor.rs

- [ ] **Step 1: 写 view mode/word wrap effect 失败测试**

在 chrome.rs 测试模块加入：

~~~rust
#[test]
fn view_mode_returns_persist_chrome_and_redraw() {
    let mut app = App::new(None);
    app.needs_redraw = false;

    let effect = app.dispatch_settings_action(
        SettingsDispatchAction::SetViewMode(ui::view_mode::ViewMode::Tabs),
    );

    assert_eq!(app.settings.view_mode, ui::view_mode::ViewMode::Tabs);
    assert!(effect.persist_settings);
    assert!(effect.sync_window_chrome);
    assert!(effect.redraw);
    assert!(!app.needs_redraw);
}

#[test]
fn word_wrap_returns_persist_and_reshape() {
    let mut app = App::new(None);
    let before = app.settings.word_wrap;
    let effect =
        app.dispatch_settings_action(SettingsDispatchAction::ToggleWordWrap);

    assert_eq!(app.settings.word_wrap, !before);
    assert!(effect.persist_settings);
    assert!(effect.reshape);
}
~~~

- [ ] **Step 2: 运行测试并确认 settings action 不存在**

~~~bash
cargo test -p edit-plus-app --lib dispatch::chrome::tests::view_mode_returns_persist_chrome_and_redraw -- --exact
~~~

Expected: FAIL，SettingsDispatchAction 不存在。

- [ ] **Step 3: 定义并穷尽处理 Settings actions**

~~~rust
pub(crate) enum SettingsDispatchAction {
    SetViewMode(ui::view_mode::ViewMode),
    SetThemeMode(ui::settings::ThemeMode),
    ToggleLineNumbers,
    ToggleWordWrap,
    ToggleStatusBar,
}

pub(crate) fn dispatch_settings_action(
    &mut self,
    action: SettingsDispatchAction,
) -> AppEffect {
    self.ui_shell.sidebar_set_open_menu(None);
    match action {
        SettingsDispatchAction::SetViewMode(mode) => {
            self.settings.view_mode = mode;
            self.settings.version = self.settings.version.wrapping_add(1);
            self.ui_shell.sidebar_set_hovered(None);
            self.ui_shell.set_dragging_sidebar(false);
            AppEffect::PERSIST_SETTINGS
                .merge(AppEffect::SYNC_WINDOW_CHROME)
        }
        SettingsDispatchAction::SetThemeMode(mode) => {
            self.settings.theme_mode = mode;
            self.settings.version = self.settings.version.wrapping_add(1);
            self.rebuild_theme_state();
            AppEffect::PERSIST_SETTINGS.merge(AppEffect::REDRAW)
        }
        SettingsDispatchAction::ToggleLineNumbers => {
            self.settings.show_line_numbers = !self.settings.show_line_numbers;
            self.settings.version = self.settings.version.wrapping_add(1);
            AppEffect::PERSIST_SETTINGS.merge(AppEffect::REDRAW)
        }
        SettingsDispatchAction::ToggleWordWrap => {
            self.settings.set_word_wrap(!self.settings.word_wrap);
            for index in 0..self.workspace.len() {
                self.workspace
                    .view_mut(index)
                    .unwrap()
                    .doc_mut()
                    .display
                    .render_cache
                    .invalidate_all();
            }
            AppEffect::PERSIST_SETTINGS.merge(AppEffect::RESHAPE)
        }
        SettingsDispatchAction::ToggleStatusBar => {
            self.settings.show_status_bar = !self.settings.show_status_bar;
            self.settings.version = self.settings.version.wrapping_add(1);
            AppEffect::PERSIST_SETTINGS.merge(AppEffect::REDRAW)
        }
    }
}

pub(crate) fn rebuild_theme_state(&mut self) {
    let system_theme = self
        .window
        .as_ref()
        .and_then(|window| window.theme())
        .unwrap_or(winit::window::Theme::Dark);
    self.current_theme = ui::Theme::resolve(
        self.settings.theme_mode,
        system_theme,
        &self.active_theme_pair,
        &mut self.theme_registry,
    );
}

pub(crate) fn handle_sidebar_key_action(
    &mut self,
    action: ui::widgets::sidebar::SidebarAction,
) -> AppEffect {
    use ui::widgets::sidebar::SidebarAction;
    let persistence = match action {
        SidebarAction::TogglePin | SidebarAction::PersistConfig => {
            AppEffect::PERSIST_WORKSPACE
        }
        SidebarAction::StartResize
        | SidebarAction::ResizeTo(_)
        | SidebarAction::EndResize
        | SidebarAction::SetWidth(_)
        | SidebarAction::SwitchTab(_)
        | SidebarAction::CloseTab(_)
        | SidebarAction::NewDocument
        | SidebarAction::OpenDocument
        | SidebarAction::OpenSettingsMenu
        | SidebarAction::SetViewMode(_)
        | SidebarAction::OpenSettingsFile
        | SidebarAction::ToggleViewMode
        | SidebarAction::Context { .. }
        | SidebarAction::Hovered
        | SidebarAction::ToggleLineNumbers
        | SidebarAction::ToggleWordWrap
        | SidebarAction::ToggleStatusBar
        | SidebarAction::SetThemeMode(_)
        | SidebarAction::ContextMenuPx { .. } => AppEffect::NONE,
    };
    persistence.merge(AppEffect::REDRAW)
}
~~~

app_dispatch.rs 删除五个旧 apply_* 方法和重复 settings_io load/save。dispatch/editor.rs 的两个 sidebar key 调用改为：

~~~rust
effect = effect.merge(self.handle_sidebar_key_action(action));
~~~

- [ ] **Step 4: 扫描重复 persistence 并运行测试**

~~~bash
rg -n "settings_io::(load|save)" crates/app/src/app_dispatch.rs
rg -n "needs_redraw|request_redraw|persist_workspace_state|apply_effect" \
  crates/app/src/dispatch/chrome.rs
cargo test -p edit-plus-app --lib dispatch::chrome::tests -- --nocapture
cargo test -p edit-plus-app --lib dispatch::editor -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: 两条扫描无输出；测试和编译 PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/dispatch/chrome.rs crates/app/src/app_dispatch.rs crates/app/src/dispatch/editor.rs
git commit -m "refactor(app): return effects from settings dispatch"
~~~

### Task 9: 提取 scrollbar/viewport/heading handler

**Files:**
- Create/Test: crates/app/src/dispatch/viewport.rs
- Modify: crates/app/src/app_dispatch.rs
- Modify: crates/app/src/lib.rs

- [ ] **Step 1: 写 preview heading 与空 document effect 测试**

在 viewport.rs 加入：

~~~rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_without_active_view_returns_none() {
        let mut app = App::new(None);
        let effect =
            app.dispatch_viewport_action(ViewportDispatchAction::ScrollViewportBy(1.0));
        assert_eq!(effect, AppEffect::NONE);
    }

    #[test]
    fn scrollbar_start_drag_requests_redraw() {
        let mut app = App::new(None);
        let effect = app.dispatch_viewport_action(
            ViewportDispatchAction::Scrollbar(
                ui::widgets::scrollbar::ScrollbarAction::StartDrag,
            ),
        );
        assert_eq!(effect, AppEffect::REDRAW);
    }
}
~~~

测试模块最终保留在 viewport.rs 文件末尾，生产实现放在 #[cfg(test)] 之前。

- [ ] **Step 2: 注册模块并确认类型不存在**

lib.rs dispatch 增加 pub(crate) mod viewport；运行：

~~~bash
cargo test -p edit-plus-app --lib dispatch::viewport::tests -- --nocapture
~~~

Expected: FAIL，ViewportDispatchAction 不存在。

- [ ] **Step 3: 定义并移动非滚轮 viewport 分支**

~~~rust
pub(crate) enum ViewportDispatchAction {
    Scrollbar(ui::widgets::scrollbar::ScrollbarAction),
    UpdateScrollTop(f64),
    ScrollViewportBy(f64),
    JumpToHeading(usize),
}

impl App {
    pub(crate) fn dispatch_wheel_scroll(
        &mut self,
        delta: winit::event::MouseScrollDelta,
    ) -> AppEffect {
        self.handle_scroll(delta);
        AppEffect::NONE
    }

    pub(crate) fn dispatch_viewport_action(
        &mut self,
        action: ViewportDispatchAction,
    ) -> AppEffect {
        match action {
            ViewportDispatchAction::Scrollbar(
                ui::widgets::scrollbar::ScrollbarAction::StartDrag,
            ) => AppEffect::REDRAW,
            ViewportDispatchAction::Scrollbar(_) => AppEffect::NONE,
            ViewportDispatchAction::UpdateScrollTop(scroll_top) => {
                let line_height = self.ui_metrics().line_height;
                let viewport_height = self.ui_shell.editor_rect().h;
                if let Some(crate::view::View::Markdown(view)) =
                    self.workspace.active_view_mut()
                {
                    let max_scroll =
                        (view.preview.content_height - viewport_height).max(0.0);
                    let pixel_scroll =
                        (scroll_top as f32 * line_height).clamp(0.0, max_scroll);
                    let changed =
                        (view.preview.scroll_y - pixel_scroll).abs() > 0.5;
                    view.preview.scroll_y = pixel_scroll;
                    return if changed {
                        AppEffect::REDRAW
                    } else {
                        AppEffect::NONE
                    };
                }
                let Some(document) = self.workspace.active_doc_mut() else {
                    return AppEffect::NONE;
                };
                document.display.viewport.set_scroll_top(
                    scroll_top,
                    &document.display.display_map,
                    line_height,
                );
                self.last_scroll_time = std::time::Instant::now();
                AppEffect::RESHAPE
            }
            ViewportDispatchAction::ScrollViewportBy(amount) => {
                let line_height = self.ui_metrics().line_height;
                let Some(document) = self.workspace.active_doc_mut() else {
                    return AppEffect::NONE;
                };
                let page_pixels =
                    document.display.viewport.visible_rows.max(1) as f32
                        * line_height;
                let pixels =
                    if amount > 0.0 { page_pixels } else { -page_pixels };
                document.display.viewport.scroll_pixels(
                    pixels,
                    &document.display.display_map,
                    line_height,
                );
                document.display.viewport.clamp_anchor(
                    &document.display.display_map,
                    line_height,
                );
                document.display.viewport.derive_scroll_top(
                    &document.display.display_map,
                    line_height,
                );
                AppEffect::REDRAW
            }
            ViewportDispatchAction::JumpToHeading(index) => {
                if let Some(crate::view::View::Markdown(view)) =
                    self.workspace.active_view_mut()
                {
                    view.preview.scroll_to_heading(index);
                    AppEffect::REDRAW
                } else {
                    AppEffect::NONE
                }
            }
        }
    }
}
~~~

app_dispatch.rs 对应四类 AppAction 只映射到 ViewportDispatchAction。

Task 12 前，viewport match arm在 app_dispatch.rs 立即 apply 返回值；HandleScroll 先调用 dispatch_wheel_scroll，其直接 redraw 行为仍由旧 handle_scroll 保持。

- [ ] **Step 4: 运行 viewport 测试和编译**

~~~bash
rg -n "needs_redraw|request_redraw|invalidate_reshape|apply_effect" crates/app/src/dispatch/viewport.rs
cargo test -p edit-plus-app --lib dispatch::viewport::tests -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: 扫描无输出；测试和编译 PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/dispatch/viewport.rs crates/app/src/app_dispatch.rs crates/app/src/lib.rs
git commit -m "refactor(app): extract viewport dispatch"
~~~

### Task 10: 让滚轮处理返回 AppEffect

**Files:**
- Modify/Test: crates/app/src/app_scroll.rs
- Modify: crates/app/src/dispatch/viewport.rs

- [ ] **Step 1: 改写现有 pixel scroll 测试**

在 app_scroll.rs 的 pixel_scroll_uses_instance_line_height 测试末尾改为：

~~~rust
app.needs_redraw = false;
let effect = app.handle_scroll(MouseScrollDelta::PixelDelta(
    PhysicalPosition::new(0.0, -36.0),
));

let scroll_top = app.workspace.active_doc().unwrap().display.viewport.scroll_top;
assert!((scroll_top - 1.0).abs() < 0.01, "scroll_top={scroll_top}");
assert_eq!(effect, AppEffect::REDRAW);
assert!(!app.needs_redraw);
~~~

- [ ] **Step 2: 运行测试并确认 handle_scroll 返回 ()**

~~~bash
cargo test -p edit-plus-app --lib app_scroll::tests::pixel_scroll_uses_instance_line_height -- --exact
~~~

Expected: FAIL，无法比较 () 与 AppEffect。

- [ ] **Step 3: 将所有滚轮退出路径映射为 effect**

签名：

~~~rust
pub(crate) fn handle_scroll(
    &mut self,
    delta: MouseScrollDelta,
) -> AppEffect
~~~

替换规则：

- tab bar/sidebar/TOC/Markdown/editor 实际滚动 → return AppEffect::REDRAW。
- 无 active view、无变化或被 guard 忽略 → return AppEffect::NONE。
- 删除函数内全部 self.needs_redraw 写入。

viewport.rs 增加：

~~~rust
pub(crate) fn dispatch_wheel_scroll(
    &mut self,
    delta: winit::event::MouseScrollDelta,
) -> AppEffect {
    self.handle_scroll(delta)
}
~~~

app_dispatch 的 HandleScroll 映射到该方法。

- [ ] **Step 4: 运行滚动测试与扫描**

~~~bash
cargo test -p edit-plus-app --lib app_scroll::tests -- --nocapture
cargo test -p edit-plus-app --lib dispatch::viewport::tests -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: 测试和编译 PASS。app_scroll 的其余 cursor helper 在 Task 11 收口后再做全文件扫描。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/app_scroll.rs crates/app/src/dispatch/viewport.rs
git commit -m "refactor(app): return effects from wheel scrolling"
~~~

### Task 11: 让光标滚动 helper 返回 effect

**Files:**
- Modify/Test: crates/app/src/app_scroll.rs
- Modify: crates/app/src/dispatch/editor.rs

- [ ] **Step 1: 写 page_down effect 失败测试**

在 app_scroll.rs 测试模块加入：

~~~rust
#[test]
fn page_down_returns_redraw_without_applying() {
    let mut app = App::new(None);
    let dv = DocumentView::new(
        (0..100).map(|index| format!("line {index}")).collect(),
        20,
        200.0,
    );
    app.workspace.push_view_for_test(View::Editor(dv));
    app.workspace.switch_to(0);
    app.needs_redraw = false;

    let effect = app.page_down();

    assert_eq!(effect, AppEffect::REDRAW);
    assert!(!app.needs_redraw);
}
~~~

- [ ] **Step 2: 运行测试并确认返回类型失败**

~~~bash
cargo test -p edit-plus-app --lib app_scroll::tests::page_down_returns_redraw_without_applying -- --exact
~~~

Expected: FAIL，page_down 返回 ()。

- [ ] **Step 3: 改造四个 helper 并合并调用结果**

以下方法全部返回 AppEffect：

~~~rust
pub(crate) fn move_cursor_visual(&mut self, delta: isize) -> AppEffect;
pub(crate) fn page_up(&mut self) -> AppEffect;
pub(crate) fn page_down(&mut self) -> AppEffect;
pub(crate) fn extend_selection_visual(&mut self, delta: isize) -> AppEffect;
~~~

成功 mutation 返回 REDRAW，无 active doc 返回 NONE；删除 needs_redraw。

dispatch/editor.rs 对每个调用使用：

~~~rust
effect = effect.merge(self.page_down());
~~~

dispatch/editor.rs 的六个 match arm改为：

~~~rust
EditCommand::MoveUp => {
    effect = effect.merge(self.move_cursor_visual(-1));
    return effect;
}
EditCommand::MoveDown => {
    effect = effect.merge(self.move_cursor_visual(1));
    return effect;
}
EditCommand::ExtendUp => {
    effect = effect.merge(self.extend_selection_visual(-1));
    return effect;
}
EditCommand::ExtendDown => {
    effect = effect.merge(self.extend_selection_visual(1));
    return effect;
}
EditCommand::PageUp => {
    effect = effect.merge(self.page_up());
    return effect;
}
EditCommand::PageDown => {
    effect = effect.merge(self.page_down());
    return effect;
}
~~~

- [ ] **Step 4: 扫描并运行 editor/scroll 测试**

~~~bash
rg -n "needs_redraw|request_redraw|invalidate_reshape|apply_effect" crates/app/src/app_scroll.rs
cargo test -p edit-plus-app --lib app_scroll::tests -- --nocapture
cargo test -p edit-plus-app --lib dispatch::editor -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: 扫描无输出；测试和编译 PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/app_scroll.rs crates/app/src/dispatch/editor.rs
git commit -m "refactor(app): return effects from cursor scrolling"
~~~

### Task 12: 建立顶层 single-apply router 和静态门禁

**Files:**
- Modify/Test: crates/app/src/app_dispatch.rs
- Create/Test: crates/app/src/dispatch_boundary_tests.rs
- Modify: crates/app/src/lib.rs

- [ ] **Step 1: 写源码边界失败测试**

创建 dispatch_boundary_tests.rs：

~~~rust
fn dispatch_sources() -> [(&'static str, &'static str); 9] {
    [
        ("app_dispatch.rs", include_str!("app_dispatch.rs")),
        ("app_scroll.rs", include_str!("app_scroll.rs")),
        ("commands.rs", include_str!("dispatch/commands.rs")),
        ("editor.rs", include_str!("dispatch/editor.rs")),
        ("mouse.rs", include_str!("dispatch/mouse.rs")),
        ("search.rs", include_str!("dispatch/search.rs")),
        ("tabs.rs", include_str!("dispatch/tabs.rs")),
        ("chrome.rs", include_str!("dispatch/chrome.rs")),
        ("viewport.rs", include_str!("dispatch/viewport.rs")),
    ]
}

fn production_source(source: &str) -> &str {
    source.split("#[cfg(test)]").next().unwrap_or(source)
}

#[test]
fn only_router_applies_effect() {
    for (name, source) in dispatch_sources() {
        let source = production_source(source);
        let count = source.match_indices("apply_effect(").count();
        let expected = usize::from(name == "app_dispatch.rs");
        assert_eq!(count, expected, "{name}");
    }
}

#[test]
fn dispatch_domains_do_not_apply_global_followups_directly() {
    let forbidden = [
        "needs_redraw =",
        "request_redraw(",
        "invalidate_reshape(",
        "update_window_title(",
        "persist_workspace_state(",
        "settings_io::save(",
    ];
    for (name, source) in dispatch_sources() {
        let source = production_source(source);
        for needle in forbidden {
            assert!(
                !source.contains(needle),
                "{name} contains forbidden call {needle}"
            );
        }
    }
}
~~~

lib.rs 加：

~~~rust
#[cfg(test)]
mod dispatch_boundary_tests;
~~~

- [ ] **Step 2: 运行门禁并确认当前 router 失败**

~~~bash
cargo test -p edit-plus-app --lib dispatch_boundary_tests -- --nocapture
~~~

Expected: FAIL，app_dispatch 仍有多个 apply/direct effect。

- [ ] **Step 3: 重写 route + single apply**

app_dispatch.rs 使用：

~~~rust
use crate::app_effect::AppEffect;
use crate::dispatch::chrome::{
    ChromeDispatchAction, SettingsDispatchAction, TabScrollDirection,
};
use crate::dispatch::viewport::ViewportDispatchAction;

pub(crate) fn dispatch(
    &mut self,
    action: AppAction,
    event_loop: &ActiveEventLoop,
) {
    let effect = self.reduce_action(action, event_loop);
    self.apply_effect(effect);
    self.update_ime_cursor_area();
}

fn reduce_action(
    &mut self,
    action: AppAction,
    event_loop: &ActiveEventLoop,
) -> AppEffect {
    match action {
        AppAction::RequestRedraw => AppEffect::REDRAW,
        AppAction::SetCursor(cursor) => {
            if let Some(window) = &self.window {
                window.set_cursor(cursor);
            }
            AppEffect::NONE
        }
        AppAction::ExecuteAppCommands(commands) => {
            self.execute_commands(commands, event_loop)
        }
        AppAction::OpenPopupMenu(menu) => self.dispatch_chrome_action(
            ChromeDispatchAction::OpenPopup(menu),
        ),
        AppAction::ExecuteContextMenuAction(action, index) => {
            self.dispatch_context_menu_action(action, index)
        }
        AppAction::OpenPopupOverflow => self.dispatch_chrome_action(
            ChromeDispatchAction::OpenOverflow,
        ),
        AppAction::ClearPopupMenu => self.dispatch_chrome_action(
            ChromeDispatchAction::ClearPopup,
        ),
        AppAction::UpdateMousePos(x, y) => {
            self.mouse.pos = (x, y);
            AppEffect::NONE
        }
        AppAction::HandleScroll(delta) => self.dispatch_wheel_scroll(delta),
        AppAction::EditorMouseInput { state, px, py, hit } => {
            self.dispatch_editor_mouse_input(state, px, py, hit)
        }
        AppAction::EditorCursorMoved { px, py, hit } => {
            self.dispatch_editor_cursor_moved(px, py, hit)
        }
        AppAction::SwitchTab(index) => {
            let workspace_effect = self.workspace.switch_to(index);
            self.handle_workspace_effect(workspace_effect)
        }
        AppAction::CloseTab(index) => {
            self.try_close_tab_with_prompt(index)
        }
        AppAction::NewEmptyTab => self.new_empty_tab(),
        AppAction::TogglePin => {
            let workspace_effect = self.workspace.toggle_pin();
            self.handle_workspace_effect(workspace_effect)
        }
        AppAction::ScrollTabLeft => self.dispatch_chrome_action(
            ChromeDispatchAction::ScrollTab(TabScrollDirection::Left),
        ),
        AppAction::ScrollTabRight => self.dispatch_chrome_action(
            ChromeDispatchAction::ScrollTab(TabScrollDirection::Right),
        ),
        AppAction::HoverTab(index) => self.dispatch_chrome_action(
            ChromeDispatchAction::HoverTab(index),
        ),
        AppAction::ScrollbarAction(action) => self.dispatch_viewport_action(
            ViewportDispatchAction::Scrollbar(action),
        ),
        AppAction::UpdateScrollTop(scroll_top) => self.dispatch_viewport_action(
            ViewportDispatchAction::UpdateScrollTop(scroll_top),
        ),
        AppAction::ScrollViewportBy(amount) => self.dispatch_viewport_action(
            ViewportDispatchAction::ScrollViewportBy(amount),
        ),
        AppAction::SetViewMode(mode) => self.dispatch_settings_action(
            SettingsDispatchAction::SetViewMode(mode),
        ),
        AppAction::OpenSettingsFile => self.open_settings_file(),
        AppAction::ToggleLineNumbers => self.dispatch_settings_action(
            SettingsDispatchAction::ToggleLineNumbers,
        ),
        AppAction::ToggleWordWrap => self.dispatch_settings_action(
            SettingsDispatchAction::ToggleWordWrap,
        ),
        AppAction::ToggleStatusBar => self.dispatch_settings_action(
            SettingsDispatchAction::ToggleStatusBar,
        ),
        AppAction::SetThemeMode(mode) => self.dispatch_settings_action(
            SettingsDispatchAction::SetThemeMode(mode),
        ),
        AppAction::SidebarResizeStart => self.dispatch_chrome_action(
            ChromeDispatchAction::SidebarResizeStart,
        ),
        AppAction::SidebarResizeEnd => self.dispatch_chrome_action(
            ChromeDispatchAction::SidebarResizeEnd,
        ),
        AppAction::SetSidebarWidth(width) => self.dispatch_chrome_action(
            ChromeDispatchAction::SetSidebarWidth(width),
        ),
        AppAction::OpenSidebarSettingsMenu => self.dispatch_chrome_action(
            ChromeDispatchAction::OpenSidebarSettingsMenu,
        ),
        AppAction::ToggleSidebarPin => self.dispatch_chrome_action(
            ChromeDispatchAction::ToggleSidebarPin,
        ),
        AppAction::SearchBarAction(action) => {
            self.dispatch_search_action(action)
        }
        AppAction::JumpToHeading(index) => self.dispatch_viewport_action(
            ViewportDispatchAction::JumpToHeading(index),
        ),
    }
}
~~~

静态测试的 app_dispatch 允许 apply_effect，但禁止数组中的六个 direct follow-up；为避免测试把顶层 apply 判为 forbidden，forbidden 不包含 apply_effect。

- [ ] **Step 4: 运行边界、App 与编译测试**

~~~bash
cargo test -p edit-plus-app --lib dispatch_boundary_tests -- --nocapture
cargo test -p edit-plus-app --lib app_dispatch -- --nocapture
cargo test -p edit-plus-app --lib dispatch -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/app_dispatch.rs crates/app/src/dispatch_boundary_tests.rs crates/app/src/lib.rs
git commit -m "refactor(app): enforce single effect application"
~~~

### Task 13: 定义 root 稳定 API 并迁移 binary

**Files:**
- Modify/Test: crates/app/src/lib.rs
- Modify: crates/app/src/main.rs
- Create/Test: crates/app/tests/public_api.rs

- [ ] **Step 1: 写 root API 编译测试**

创建 public_api.rs：

~~~rust
use edit_plus_app::{
    App, AppEvent, CliArgs, GpuError, headless_init, parse_args,
};

#[test]
fn root_exports_binary_contract() {
    let args = vec!["NoteR".to_string(), "--headless".to_string()];
    let cli: CliArgs = parse_args(&args);
    assert!(cli.headless);
    let _app = App::new(None);
    let _event: Option<AppEvent> = None;
    let _error: Option<GpuError> = None;
    let _init = headless_init;
}
~~~

- [ ] **Step 2: 运行测试并确认 CLI 不在 root**

~~~bash
cargo test -p edit-plus-app --test public_api --no-run
~~~

Expected: FAIL，CliArgs/parse_args 未从 crate root 导出。

- [ ] **Step 3: 增加 root re-export 并迁移 main**

lib.rs：

~~~rust
pub use app::App;
pub use app_event::AppEvent;
pub use cli::{CliArgs, parse_args};
pub use gpu::{GpuError, headless_init};
~~~

main.rs：

~~~rust
use edit_plus_app::{
    App, AppEvent, headless_init, parse_args,
};

let cli = parse_args(&args);
let mut app = App::new(cli.file);
~~~

其余 event loop 代码不变。

- [ ] **Step 4: 运行 public API、binary 和编译**

~~~bash
cargo test -p edit-plus-app --test public_api
cargo check -p edit-plus-app --bin NoteR
cargo check -p edit-plus-app
~~~

Expected: PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/lib.rs crates/app/src/main.rs crates/app/tests/public_api.rs
git commit -m "refactor(app): define root application API"
~~~

### Task 14: 建立隐藏 dev_support 并迁移 render smoke

**Files:**
- Modify/Test: crates/app/src/lib.rs
- Modify/Test: crates/app/tests/render_smoke.rs

- [ ] **Step 1: 先把 render smoke 改为目标路径**

将两处：

~~~rust
use edit_plus_app::measure_adapter::MeasureFromShaper;
~~~

改为：

~~~rust
use edit_plus_app::dev_support::MeasureFromShaper;
~~~

- [ ] **Step 2: 运行 no-run 并确认 dev_support 不存在**

~~~bash
cargo test -p edit-plus-app --test render_smoke --no-run
~~~

Expected: FAIL，dev_support 不存在。

- [ ] **Step 3: 在 lib.rs 增加唯一开发支持入口**

~~~rust
#[doc(hidden)]
pub mod dev_support {
    pub use crate::document_view::DocumentView;
    pub use crate::measure_adapter::MeasureFromShaper;
    pub use crate::snap_tree::{DisplayLineEntry, SnapTree};
}
~~~

不从 root 直接 re-export 这些类型。

- [ ] **Step 4: 运行 render smoke no-run 和编译**

~~~bash
cargo test -p edit-plus-app --test render_smoke --no-run
cargo check -p edit-plus-app
~~~

Expected: PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/lib.rs crates/app/tests/render_smoke.rs
git commit -m "refactor(app): expose hidden development support"
~~~

### Task 15: 迁移 DocumentView benchmarks

**Files:**
- Modify/Test: crates/app/benches/tab_bench.rs
- Modify/Test: crates/app/benches/scroll_bench.rs

- [ ] **Step 1: 替换 benchmark imports**

tab_bench.rs 顶部增加：

~~~rust
use edit_plus_app::dev_support::DocumentView;
~~~

并将：

~~~rust
edit_plus_app::document_view::DocumentView::from_file(...)
~~~

改为：

~~~rust
DocumentView::from_file(...)
~~~

scroll_bench.rs 同样增加 import，并把函数签名、from_file/new 的全部完整路径改为 DocumentView。

- [ ] **Step 2: 静态确认旧路径清零**

~~~bash
rg -n "edit_plus_app::document_view" crates/app/benches/tab_bench.rs crates/app/benches/scroll_bench.rs
~~~

Expected: 无输出。

- [ ] **Step 3: 编译两个 benchmark**

~~~bash
cargo check -p edit-plus-app --bench tab_bench
cargo check -p edit-plus-app --bench scroll_bench
~~~

Expected: PASS。

- [ ] **Step 4: 运行 app 编译**

~~~bash
cargo check -p edit-plus-app
~~~

Expected: PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/benches/tab_bench.rs crates/app/benches/scroll_bench.rs
git commit -m "refactor(app): route document benchmarks through dev support"
~~~

### Task 16: 私有化 app 模块并完成 all-targets 验收

**Files:**
- Modify/Test: crates/app/src/lib.rs
- Modify/Test: crates/app/benches/snap_tree_bench.rs

- [ ] **Step 1: 迁移 SnapTree benchmark**

将：

~~~rust
use edit_plus_app::snap_tree::{DisplayLineEntry, SnapTree};
~~~

改为：

~~~rust
use edit_plus_app::dev_support::{DisplayLineEntry, SnapTree};
~~~

- [ ] **Step 2: 将内部 pub mod 全部收缩**

lib.rs 保留的外部声明只能是：

~~~rust
#[doc(hidden)]
pub mod dev_support {
    pub use crate::document_view::DocumentView;
    pub use crate::measure_adapter::MeasureFromShaper;
    pub use crate::snap_tree::{DisplayLineEntry, SnapTree};
}

pub use app::App;
pub use app_event::AppEvent;
pub use cli::{CliArgs, parse_args};
pub use gpu::{GpuError, headless_init};
~~~

其余原 pub mod 全部改为 mod；已有 pub(crate) mod 可保持。不要修改模块文件内的类型可见性，先让父模块边界完成收缩。

- [ ] **Step 3: 静态验收 public surface**

~~~bash
rg -n "^pub mod " crates/app/src/lib.rs
rg -n "edit_plus_app::(document_view|measure_adapter|snap_tree|cli)::" \
  crates/app/src/main.rs crates/app/tests crates/app/benches
~~~

Expected:

- 第一条只输出 dev_support。
- 第二条无输出。

- [ ] **Step 4: 运行最终验证**

~~~bash
cargo test -p edit-plus-app --lib
cargo test -p edit-plus-app --test public_api
cargo test -p edit-plus-app --test smoke
cargo test -p edit-plus-app --test render_smoke --no-run
cargo check -p edit-plus-app --all-targets
cargo check -p edit-plus-app
git diff --check
~~~

Expected: 全部 PASS，git diff --check 无输出。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/lib.rs crates/app/benches/snap_tree_bench.rs
git commit -m "refactor(app): close application public boundaries"
~~~

## 设计覆盖矩阵

| 设计要求 | 任务 | 验收 |
|---|---:|---|
| AppEffect 新字段、代数和固定顺序 | 1、3 | 单元测试和 App apply 测试 |
| Settings 单一 persistence | 2、3、8 | 映射测试；dispatch 无 settings_io 直调 |
| commands/tabs 无嵌套 apply | 4 | 静态扫描和 batch close 测试 |
| editor/zoom 不提前 reshape | 5、6 | generation/needs_redraw 哨兵测试 |
| chrome/settings handler 返回 effect | 7、8 | 领域测试 |
| viewport/scroll helper 返回 effect | 9-11 | scroll/empty view 测试 |
| AppAction 顶层 single apply | 12 | 源码边界测试 |
| root 稳定 API | 13 | public_api integration test |
| dev-only 外部类型入口 | 14-16 | render/bench all-targets check |
| lib.rs 内部模块私有化 | 16 | pub mod 静态扫描 |

## 完成定义

- AppEffect 包含 redraw、reshape、title、两类 persistence 和 window chrome。
- merge 满足布尔 union 代数，steps 顺序固定。
- AppAction 领域 handler 不调用 apply_effect。
- app_dispatch 顶层每个 action 只调用一次 apply_effect。
- dispatch domain 和 app_scroll 无 direct redraw/reshape/title/persistence。
- settings toggles 只修改内存并返回 PERSIST_SETTINGS。
- lifecycle/render loop 现有调度不被强制 AppEffect 化。
- lib.rs 稳定入口仅为 App、AppEvent、CLI 和 headless GPU。
- dev_support 是唯一 pub mod，tests/bench 不再穿透内部模块。
- app lib、integration tests、bench/all-targets 和静态门禁全部通过。

## 实施后的边界测试建议

- 一个 command batch 同时触发 reshape、title、settings/workspace persistence。
- settings 保存失败后仍执行 workspace persistence 和 redraw。
- 无窗口 App 应用 title/chrome/redraw。
- dirty tab 对话框选择保存、放弃、取消。
- active tab 关闭后 effect 从最终 active state 更新标题和 persistence。
- Markdown preview 与 editor scrollbar 分支返回不同 effect。
- popup/sidebar 尚未布局或无 GPU 时返回 NONE，不 panic。
- macOS ViewMode 切换与非 macOS no-op。
- 所有 Criterion benchmark 在私有化后仍能编译。
