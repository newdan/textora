# Phase 5 Maintenance — 遗留问题与建议

> Status: active
> Owner: Dan
> Created: 2026-06-21

本文档汇总 Phase 5 执行过程中最终审查发现的所有问题。
Critical / Important 已在分支内修复；Minor 和后续建议记录在此。

---

## 已修复

### Important #1: `compile_error!` 项目名不一致

- **文件:** `crates/app/src/main.rs:2`
- **问题:** 消息使用 "The NoteR application"，但 README / 仓库名是 "Edit+"，首次在 Linux 上构建的开发者会困惑
- **修复:** 改为 `"The Edit+ application (NoteR binary) currently supports macOS only; library crates remain portable."`
- **提交:** `707d1472`

### Minor #3: `docs/README.md` 命名约定与存量文件不一致

- **文件:** `docs/README.md`
- **修复:** 在命名约定中补充存量文件兼容说明："存量文件可能不完全符合此约定，新文件必须遵循"

### Minor #4: 依赖复查日期未关联提醒

- **文件:** `docs/dependency-policy.md`
- **修复:** 在文件末尾补充「复查提醒」章节，提示 2026-07-19 到期前运行 `dependency-report.sh`

### Task Minor #1: macOS target section 注释

- **文件:** `crates/app/src/main.rs`
- **修复:** 在 `#[cfg(not(target_os = "macos"))]` 前添加说明注释

### Task Minor #4: CONTRIBUTING 链接 AGENTS.md

- **文件:** `CONTRIBUTING.md`
- **修复:** 添加「更多约定」章节，链接 `AGENTS.md`

### Task Minor #5: CODE_REVIEW 状态头多余空行

- **文件:** `CODE_REVIEW_20250619.md`
- **修复:** 移除状态头与正文间的一个多余空行

---

### Important #2: `scripts/verify.sh` 兼容性 + 预存质量问题

- **文件:** `scripts/verify.sh`、`crates/core`、`crates/app`、`crates/ui` 等
- **问题:** verify.sh 三步检查（fmt / clippy / test）均有预存失败
- **修复:**
  - `cargo fmt`: 44 个文件格式漂移 → 全部 `cargo fmt --all` 修复
  - `cargo clippy`: 重复测试函数（`text_buffer_tests.rs`）→ 删除重复；`ResolveState` 变体大小差异 → `Box<ThemeDefinition>`；`from_persistent` 命名 → `restore_from_persistent`；bench API 过时 → 更新为 `TabBarWidgetInput`
  - `cargo test`: `invalid_utf8_file_name` 测试在 macOS APFS 上 panic → 加容错 skip
- **结果:** `cargo fmt --check` 通过、`cargo clippy` 零警告、`cargo test --workspace` 全绿（820 pass）

---

## 待处理

### Minor #5: `Cargo.toml` repository URL 需确认

- **文件:** `Cargo.toml` (workspace `repository` 字段)
- **问题:** 指向 `https://github.com/dan/edit-plus`，需确认该 URL 有效且可访问（private repo 或 URL 不准确会 404）
- **建议:** 快速确认 GitHub URL 存在

---

## 各任务审查 Minor 记录（不阻塞合并）

| Task | Minor 项 | 状态 |
|------|----------|------|
| 1 | macOS target section 可加注释说明意图 | ✅ 已修复 |
| 2 | `[profile.profiling]` 与 `[profile.bench]` 间多一空行 | 跳过（已是标准格式） |
| 3 | 依赖路径数据来自 implementer 报告，diff 中无法独立验证原始命令输出 | 无需修复 |
| 4 | CONTRIBUTING 未链接 `AGENTS.md` | ✅ 已修复 |
| 5 | `CODE_REVIEW_20250619.md` 状态头与正文间有两行空行 | ✅ 已修复 |
| 6 | implementer 报告称移除 13 行孤立注释，实际 diff 移除 15 行（计数偏差） | 无需修复 |

---

## 构建矩阵备注（macOS arm64）

| 检查项 | 结果 | 备注 |
|--------|------|------|
| `cargo fmt --all -- --check` | ✅ | 44 文件已修复 |
| `cargo clippy --workspace --all-targets` | ✅ 0 warnings | 重复函数/变体大小/命名/bench API 已修复 |
| `cargo test --workspace` | ✅ 全绿 | macOS APFS skip 已加容错 |
| `cargo check -p edit-plus-app` | ✅ | |
| `cargo check -p edit-plus-app --release` | ✅ | |
| `cargo check -p edit-plus-app --profile profiling` | ✅ | |
