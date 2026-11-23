# Plan：代码清理与常量统一

> 对应原方案 Phase 0-2。独立可执行，无需依赖其他 plan。

---

## 目标

消除仓库噪声、死代码、魔法数字，为后续重构扫清障碍。

---

## Phase A：快速胜利（30 分钟）

| # | 任务 | 操作 |
|---|------|------|
| A1 | 删除 `crash.log` 的 git 跟踪 | `git rm --cached crash.log` |
| A2 | 删除 `CLAUDE.md`（AGENTS.md 是其超集） | `git rm CLAUDE.md` |
| A3 | `ui/src/widgets/popup_menu.rs` 5 行重复 doc comment | 删 4 行留 1 行 |

---

## Phase B：死代码删除（2 小时）

### B1 确认无用的模块

| # | 文件 | 原因 |
|---|------|------|
| B1.1 | `core/src/terminal_stubs.rs` + `core/src/buffer/terminal_render.rs` | `terminal-render` feature 从未启用 |
| B1.2 | `core/src/helpers.rs` 中 `Size` 和 `Rect` 类型 | 全项目零引用（`Point` 保留） |
| B1.3 | `stdext/src/alloc.rs`、`stdext/src/glob.rs` | 全项目零引用 |
| B1.4 | `app/src/app_tests.rs`（孤儿） | 无 `mod app_tests` 声明；抽取共用 `mock_cluster()` 到 `test_helpers.rs`，其余测试合并入 `render_pipeline_tests.rs` 或在父模块中声明 |

### B2 未使用的函数/变量

| # | 文件 | 内容 |
|---|------|------|
| B2.1 | `app/src/app_renderer.rs` | 删除 `render_text_fragments`（零调用） |
| B2.2 | `app/src/app_renderer.rs:222` | 删除 `drop(lc)`（`usize` 是 Copy） |
| B2.3 | `app/src/app_lifecycle.rs` | 删除重复的 `WINDOW_TITLE`（保留 app.rs 中的） |
| B2.4 | `app/src/app_renderer.rs` | 删除未使用的 import 和局部变量 |
| B2.5 | `app/src/document_view/mod.rs` | 删除未使用的 `DisplayLineMap`、`RenderCache` import |
| B2.6 | `app/src/paint_backend.rs` | 删除未使用的 `is_whitespace_cluster` import |
| B2.7 | `app/src/reshape_worker.rs` | 删除赋值后从未读取的 `proxy` |
| B2.8 | `ui/src/widgets/sidebar.rs` | 删除死方法 `steal_state()` 和 `inject_state()`（生产只用 `inject_persistent`） |
| B2.9 | `app/src/document_view/mod.rs` | 删除 `sync_after_edit_full()`（仅增量方法被使用） |

### B3 修正错误的 `#[allow(dead_code)]`

| # | 文件 | 操作 |
|---|------|------|
| B3.1 | `app/src/workspace.rs:644` | 移除 `load_pinned()` 上的 `#[allow(dead_code)]`（实际在 app.rs:1298 被调用） |
| B3.2 | `app/src/menu_handler.rs:13` | 移除 `AppCommand` 上的 `#[allow(dead_code)]`（被大量使用） |

### B4 可见性修复

- `app/src/document_view/mod.rs`：`DisplayState` 和 `CursorState` 改为 `pub`，或暴露接口改为 `pub(crate)`

---

## Phase C：常量统一（3 小时）

### C1 新建 `ui/src/constants.rs`

```rust
// === 尺寸 ===
pub const BAR_HEIGHT: f32 = 28.0;     // 统一 HEADER_H / NEW_BTN_H / SETTINGS_BTN_H / SEARCH_BAR_HEIGHT
pub const ROW_HEIGHT: f32 = 24.0;
pub const SIDEBAR_MIN_WIDTH: f32 = 160.0;
pub const SIDEBAR_MAX_WIDTH: f32 = 400.0;
pub const SIDEBAR_DEFAULT_WIDTH: f32 = 220.0;
pub const SCROLLBAR_THUMB_MIN_HEIGHT: f32 = 25.0;

// === 间距 ===
pub const H_PADDING: f32 = 12.0;
pub const MEDIUM_GAP: f32 = 10.0;
pub const SMALL_GAP: f32 = 8.0;
pub const TINY_GAP: f32 = 4.0;
pub const MICRO_GAP: f32 = 2.0;

// === 字体 ===
pub const BODY_FONT_SIZE: f32 = 14.0;
pub const TITLE_FONT_SIZE: f32 = 13.0;
pub const CAPTION_FONT_SIZE: f32 = 10.0;
pub const LN_FONT_SCALE: f32 = 0.8;     // 行号字号缩放比
pub const BASELINE_RATIO: f32 = 0.8;     // 基线偏移比

// === 其他 ===
pub const BUTTON_SIZE: f32 = 16.0;
pub const CLOSE_BTN_SIZE: f32 = 12.0;
pub const UNDERLINE_ALPHA: f32 = 0.75;
pub const TRAFFIC_LIGHT_TOTAL_W: f32 = 96.0;
```

> **注意：** 垂直居中比例（status_bar 0.35 / title_bar 0.6 / popup_menu 0.65）各有差异，因为不同组件字体基线不同，**不强行统一**。

### C2 逐文件替换范围

| 文件 | 替换内容 |
|------|----------|
| `ui/src/sidebar.rs` | 所有 `const` 值引用 constants；`item_h = 28.0 * dpi`（line 456, 979）对 `ROW_H = 24.0` 存在值不一致，确认后修 |
| `ui/src/widgets/search_bar.rs` | `SEARCH_BAR_HEIGHT` → `constants::BAR_HEIGHT` |
| `ui/src/widgets/status_bar.rs` | 硬编码间距/字体 → constants |
| `ui/src/widgets/title_bar.rs` | 同上 |
| `ui/src/widgets/popup_menu.rs` | 同上 |
| `app/src/render_pipeline.rs` | `font_size() * 0.8`(4处) → `LN_FONT_SCALE`；`line_height * 0.8`(5处) → `BASELINE_RATIO` |
| `app/src/gutter.rs`、`decorations.rs`、`render_cache.rs` | 同上 |
| `app/src/app_renderer.rs` | `68.0 + 6.0 + 14.0 + 8.0` → `TRAFFIC_LIGHT_TOTAL_W` |
| `app/src/cursor_motion.rs` | 32.0 → `H_PADDING` |

### C3 统一 `test_theme()`（7 份 → 1 份，节省 ~210 行）

1. 在 `ui/src/theme.rs` 中加 `#[cfg(test)] pub fn test_theme() -> Theme`
2. 各 widget 测试文件删除本地定义，改为 `use crate::theme::test_theme;`

### C4 popup_menu 内联颜色

`ui/src/widgets/popup_menu.rs` 中 27 个硬编码 light theme 颜色 → 移入 `Theme::light()`。

---

## 验证

- 每项完成后 `cargo check --all-targets` 零错误
- `cargo test` 全部通过
- C 阶段完后运行应用做视觉回归

## 工作量

~5.5 小时，建议一次性连续执行。
