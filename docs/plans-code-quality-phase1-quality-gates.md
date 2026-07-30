# Workspace Quality Gates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 锁定 stable 工具链，清零格式/编译/Clippy/test warning，并让本地与 CI 共用唯一 verify 语义。

**Architecture:** `scripts/verify.sh` 是唯一门禁入口，CI 只负责安装锁定工具链并调用它。先单独提交工具链与格式化，再按 crate 清理 warning，最后启用 `-D warnings`，避免格式噪声与逻辑修改混在一起。

**Tech Stack:** Rustup stable 1.93.0、rustfmt、Clippy、Bash、GitHub Actions。

---

### Task 1: 锁定 stable 工具链并移除无效 rustfmt 选项

**Files:**
- Create: `rust-toolchain.toml`
- Modify: `rustfmt.toml:1-7`

- [ ] **Step 1: 创建工具链清单**

```toml
[toolchain]
channel = "1.93.0"
components = ["clippy", "rustfmt"]
profile = "minimal"
```

- [ ] **Step 2: 将 rustfmt 配置收敛到 stable 支持项**

完整内容改为：

```toml
edition = "2024"
newline_style = "Unix"
use_field_init_shorthand = true
use_small_heuristics = "Max"
```

- [ ] **Step 3: 验证无 nightly-only warning**

Run: `cargo fmt --all -- --check`

Expected: 可能因文件尚未格式化而失败，但输出不得包含 “unstable features are only available in nightly channel”。

- [ ] **Step 4: 提交配置**

```bash
git add rust-toolchain.toml rustfmt.toml
git commit -m "build: pin stable Rust toolchain"
```

### Task 2: 单独完成全仓格式化

**Files:**
- Modify: 每个原子批次仅处理 `git ls-files '*.rs'` 排序后的连续 1–3 个文件。

- [ ] **Step 1: 确认工作树仅含计划内改动后，按最多 3 文件生成批次**

```bash
git status --short
git ls-files '*.rs' | sort | split -l 3 - /tmp/edit-plus-rustfmt-batch-
```

Expected: 每个 `/tmp/edit-plus-rustfmt-batch-*` 文件包含 1–3 个 Rust 路径。

- [ ] **Step 2: 每个批次独立格式化、验证和提交**

对每个批次执行：

```bash
xargs rustfmt --edition 2024 < /tmp/edit-plus-rustfmt-batch-aa
git add --pathspec-from-file=/tmp/edit-plus-rustfmt-batch-aa
test "$(git diff --cached --name-only | wc -l | tr -d ' ')" -le 3
git diff --cached --quiet || git commit -m "style: format Rust workspace batch aa"
```

将 `aa` 依字典序替换为实际 batch 后缀，直至处理完全部 batch；每个提交只能包含该批 Rust 文件。

- [ ] **Step 3: 全仓格式化验收**

```bash
cargo fmt --all -- --check
git status --short
```

Expected: fmt PASS；工作树无未提交格式差异。

### Task 3: 清理测试层 warning 与固定 sleep

**Files:**
- Modify: `crates/app/src/measure_adapter.rs:23-30`
- Modify: `crates/app/src/reshape_worker.rs`
- Modify: `crates/app/src/document_view/boundary_tests.rs:465`

- [ ] **Step 1: 用 Cargo feature 替代未知 cfg**

在 `crates/app/Cargo.toml` 增加 feature 是独立的第 4 文件提交：

```toml
[features]
default = []
ci-no-fonts = []
```

然后将测试标记改为：

```rust
#[cfg(not(feature = "ci-no-fonts"))]
```

- [ ] **Step 2: 将固定 500ms sleep 改为 deadline 轮询**

测试模块加入一个返回单个结果的 deadline helper：

```rust
fn recv_one(worker: &ReshapeWorker, timeout: Duration) -> ReshapeResult {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(result) = worker.drain_completed(1).into_iter().next() {
            return result;
        }
        assert!(Instant::now() < deadline, "condition was not met within {timeout:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
}
```

四个 worker 测试把 `sleep(500ms)` 与随后 `drain_completed(10)` 替换为 `let result = recv_one(&w, Duration::from_secs(2));`；断言分别检查 generation、`entry.byte_length` 和非空结果。deadline 只负责同步，不吞掉失败。

- [ ] **Step 3: 将 deprecated viewport 调用改成当前入口**

在测试中构造 `DisplayLineMap`，调用：

```rust
dv.display.viewport.scroll_doc_lines(20, &DisplayLineMap::new(&dv.display.display_map));
```

- [ ] **Step 4: 验证测试目标零 warning**

```bash
cargo test -p edit-plus-app --lib reshape_worker -- -Z unstable-options --report-time 2>/dev/null || cargo test -p edit-plus-app --lib reshape_worker
cargo check -p edit-plus-app --tests --features ci-no-fonts -- -D warnings
```

Expected: PASS，且不再出现 `ci_no_fonts`、deprecated viewport 或固定 500ms 等待相关 warning。

- [ ] **Step 5: 分两次提交**

```bash
git add crates/app/Cargo.toml crates/app/src/measure_adapter.rs
git commit -m "test(app): model fontless CI as a feature"
git add crates/app/src/reshape_worker.rs crates/app/src/document_view/boundary_tests.rs
git commit -m "test(app): remove timing sleeps and deprecated viewport calls"
```

### Task 4: 按 crate 清零 Clippy

**Files:**
- Modify: 每个原子提交仅限一个 crate 内最多 3 个文件。

- [ ] **Step 1: 按依赖顺序逐 crate 运行严格 Clippy**

```bash
for package in stdext lsh edit-plus-core edit-plus-shaping edit-plus-render edit-plus-ui edit-plus-markdown edit-plus-app; do
  cargo clippy -p "$package" --all-targets -- -D warnings || break
done
```

Expected: 首次在第一个仍有 warning 的 crate 停止；每修完一个 crate 重跑，直到循环退出码为 0。

- [ ] **Step 2: 仅接受语义明确的修复模式**

- 未使用 import/变量：删除；若值是有意保留的测量结果，改成实际断言，不加 `_` 掩盖。
- `unused_mut`：去掉 `mut`。
- `dead_code`：生产代码无调用则删除；确属平台 API 保留时写具体原因，例如 `#[allow(dead_code, reason = "called by the Windows backend once that target is enabled")]`。
- `deprecated`：迁移到替代 API，不加 allow。
- 测试 helper：移入 `#[cfg(test)] mod tests`。

每修复最多 3 个文件后执行该 crate 的 `cargo check` 与 `cargo test`，随后交互式只暂存本批文件并核对数量：

```bash
cargo check -p edit-plus-shaping --all-targets
cargo test -p edit-plus-shaping
git add -p
test "$(git diff --cached --name-only | wc -l | tr -d ' ')" -le 3
git commit -m "chore(shaping): clear strict clippy warnings"
```

- [ ] **Step 3: workspace 严格 Clippy 验收**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

Expected: PASS，无 warning。

### Task 5: 创建本地 verify 与 CI

**Files:**
- Create: `scripts/verify.sh`
- Create: `.github/workflows/verify.yml`

- [ ] **Step 1: 创建唯一验证脚本**

```bash
#!/usr/bin/env bash
set -euo pipefail

run() {
  printf '\n==> %s\n' "$*"
  "$@"
}

run cargo fmt --all -- --check
run cargo check --workspace --all-targets
run cargo clippy --workspace --all-targets -- -D warnings
run cargo test --workspace
```

执行 `chmod +x scripts/verify.sh`。

- [ ] **Step 2: 创建 CI workflow**

```yaml
name: verify

on:
  pull_request:
  push:
    branches: [main]

jobs:
  verify:
    runs-on: macos-15
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@master
        with:
          toolchain: 1.93.0
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - run: ./scripts/verify.sh
```

- [ ] **Step 3: 本地运行与提交**

```bash
./scripts/verify.sh
git add scripts/verify.sh .github/workflows/verify.yml
git commit -m "ci: enforce workspace verification gates"
```

Expected: 本地脚本四项全绿；CI 不重复维护另一套命令。
