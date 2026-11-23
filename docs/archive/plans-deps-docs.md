# Plan：依赖整理与文档归档

> 对应原方案 Phase 7-8。独立可执行，与其他 plan 无依赖。

---

## Part A：依赖管理统一（3 小时）

### A1 workspace.dependencies 统一

| # | 任务 |
|---|------|
| A1.1 | `cosmic-text` 移到 `[workspace.dependencies]`，app 和 shaping 改为 `cosmic-text = { workspace = true }` |
| A1.2 | `unicode_categories` 移到 workspace，app 和 ui 改为引用 workspace |
| A1.3 | `shaping` 中 re-export app 需要的 cosmic-text 类型（如 `fontdb::ID`），让 app 不直接依赖 `cosmic-text` |

### A2 stdext 清理

| # | 任务 |
|---|------|
| A2.1 | 删除 `stdext/src/alloc.rs`（零引用） |
| A2.2 | 删除 `stdext/src/glob.rs`（零引用） |
| A2.3 | 更新 `stdext/src/lib.rs`，移除对应 `pub mod` 声明 |

---

## Part B：文档归档（1 小时）

### B1 根目录清理

| # | 文件 | 操作 |
|---|------|------|
| B1.1 | `fix_app_tests.py`, `fix_popup_menu.py`, `fix_ui_shell.py`, `update_dock.rs` | 删除（一次性脚本） |
| B1.2 | `test_format.rs`, `test_visible_rows.rs` | 删除（测试片段） |
| B1.3 | `.git_log_10.txt` | 删除（调试遗留） |
| B1.4 | `plans.md` | 移入 `docs/plans-overview.md` |

### B2 docs/ 过期文档归档

创建 `docs/archive/`，移入：

**过期的阶段审计：**
`stage6_audit.md`, `stage7_review.md`, `stage_6_7_audit.md`, `progress_audit.md`, `audit_fix.md`, `audit_fix_v2.md`

**已解决的 bug 分析：**
`ghost_lines_root_cause_v2.md`, `scroll_bugs_root_cause.md`, `workspace_restore_bug_analysis.md`, `cursor-click-drift-investigation.md`

**已实施的设计文档：**
`viewport_0601.md`, `viewport-scroll-redesign.md`, `viewport_architecture_analysis.md`, `displayrow.md`, `displayrow_review.md`, `visual_doc_design.md`, `plans_large_file_scroll_perf.md`

**删除重复文档：**
`ui-skeleton-audit-2025-06-12.md`（保留 2026 版本）；`audit_ui_skeleton_2025_06_12.md`

### B3 保留文档

- `manual_test_protocol.md`（持续使用）
- `editor_performance_playbook.md`（参考）
- `plan-ui-split.md`（架构参考）
- `plans-sidebar-item-features.md`, `plans-splitter-widget.md`（活跃 plan）
- `docs/superpowers/specs/`（设计文档）

---

## Part C：测试文件重命名（30 分钟）

| 当前 | 改为 |
|------|------|
| `test_tests.rs` | `basic_tests.rs` |
| `test_b11_tests.rs` | 按内容命名（如 `resize_tests.rs`） |
| `test_boundary_tests.rs` | `boundary_tests.rs` |
| `test_cursor_visual_tests.rs` | `cursor_visual_tests.rs` (保留) |
| `test_perf_tests.rs` | `perf_tests.rs` |
| `test_stage7_tests.rs` | 按内容命名 |
| `test_word_wrap_tests.rs` | `word_wrap_tests.rs` |

去掉冗余 `test_` 前缀（文件名后缀 `_tests.rs` 已表明身份）。

同步更新所有 `#[path = "..."]` 引用。

---

## Part D：其他小修正（15 分钟）

| # | 任务 |
|---|------|
| D1 | `render_pipeline.rs:946` 的 `#[path = "render_pipeline_tests.rs"]` 前加注释说明原因 |
| D2 | `WINDOW_TITLE` 保留 `app.rs` 一处定义，删除 `app_lifecycle.rs` 的重复（如 `plans-cleanup.md` 中未完成） |

---

## 验证

- `cargo check --all-targets` 零错误
- `cargo test` 全通过（测试文件更名后路径更新）
- `git status` 确认无遗漏文件

## 工作量

~4.5 小时。各部分独立，可并行或拆分执行。
