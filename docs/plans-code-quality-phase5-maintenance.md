# Build and Repository Maintenance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 明确平台与发布策略、记录依赖重复的处置结论，并建立最小且一致的仓库/文档维护入口。

**Architecture:** 当前发布承诺收口为 macOS（arm64/x86_64），通用 library crate 保持可移植但不宣称 app 已跨平台；release 与本地 profiling 分离。仓库根文档只放稳定入口，历史审计与计划进入统一索引，crash dump 不入 Git。

**Tech Stack:** Cargo manifests/metadata、Git、Markdown。

---

### Task 1: 收口 macOS dependency 与 app 平台契约

**Files:**
- Modify: `crates/app/Cargo.toml`
- Modify: `crates/app/src/main.rs`

- [ ] **Step 1: 将 Objective-C dependencies 移入 macOS target section**

```toml
[target.'cfg(target_os = "macos")'.dependencies]
objc2.workspace = true
objc2-app-kit.workspace = true
objc2-foundation.workspace = true
```

从普通 `[dependencies]` 删除这三项。

- [ ] **Step 2: binary 对非 macOS 给出编译期明确错误**

在 `main.rs` 顶部加入：

```rust
#[cfg(not(target_os = "macos"))]
compile_error!("The NoteR application currently supports macOS only; library crates remain portable.");
```

不要删除 library crate 的非 macOS 实现；未来跨平台 app 是单独项目。

- [ ] **Step 3: 验证 macOS all-targets 并提交**

```bash
cargo check --workspace --all-targets
git add crates/app/Cargo.toml crates/app/src/main.rs
git commit -m "build(app): declare current macOS platform support"
```

### Task 2: 分离真实 release 与本地 profiling profile

**Files:**
- Modify: `Cargo.toml:11-24`

- [ ] **Step 1: 替换冲突的 profile 设置**

```toml
[profile.release]
codegen-units = 1
debug = 0
lto = true
opt-level = "s"
panic = "abort"
strip = "symbols"
incremental = false

[profile.profiling]
inherits = "release"
debug = "full"
split-debuginfo = "packed"
strip = "none"
incremental = true

[profile.bench]
codegen-units = 16
lto = "thin"
```

- [ ] **Step 2: 验证两个 profile**

```bash
cargo check -p edit-plus-app --release
cargo check -p edit-plus-app --profile profiling
```

Expected: 两条 PASS；Cargo 不报告 ignored/conflicting profile key。

- [ ] **Step 3: 提交**

```bash
git add Cargo.toml
git commit -m "build: separate release and profiling profiles"
```

### Task 3: 记录依赖重复基线，不强制错误 dedupe

**Files:**
- Create: `docs/dependency-policy.md`
- Create: `scripts/dependency-report.sh`

- [ ] **Step 1: 创建可复现依赖报告脚本**

```bash
#!/usr/bin/env bash
set -euo pipefail
cargo tree --workspace --duplicates
```

执行 `chmod +x scripts/dependency-report.sh`。

- [ ] **Step 2: 写依赖政策**

文档明确：直接依赖集中到 `[workspace.dependencies]`；同 major 可升级时优先统一；不同 major 的传递重复必须记录上游来源；禁止仅为消除 `cargo tree -d` 输出而使用不兼容 `[patch]`。记录当前 `objc2 0.5/0.6` 与 `ttf-parser 0.20/0.21` 来自哪些 `cargo tree -i` 路径，并注明复查日期 2026-07-19。

- [ ] **Step 3: 验证和提交**

```bash
./scripts/dependency-report.sh
cargo tree -i objc2@0.5.2
cargo tree -i ttf-parser@0.20.0
git add docs/dependency-policy.md scripts/dependency-report.sh
git commit -m "docs: establish dependency duplication policy"
```

### Task 4: 建立根目录项目入口

**Files:**
- Create: `README.md`
- Create: `CONTRIBUTING.md`
- Create: `LICENSE`

- [ ] **Step 1: README 写稳定事实**

必须包含：项目是 Rust/wgpu 桌面文本与 Markdown 编辑器；当前 app 支持 macOS；Rust 版本由 `rust-toolchain.toml` 锁定；构建 `cargo build -p edit-plus-app`；验证 `./scripts/verify.sh`；架构入口链接 `AGENTS.md`；计划索引链接 `docs/README.md`。不写未经实现的跨平台承诺。

- [ ] **Step 2: CONTRIBUTING 写提交流程**

包含：先跑 verify；bug 先写复现测试；逻辑与格式化分提交；单原子任务最多 3 文件；新 UI widget 通过纯输入 struct；禁止 production stub 编译成功后运行时固定失败。

- [ ] **Step 3: LICENSE 使用 workspace 声明的 MIT 文本**

版权行使用 `Copyright (c) 2026 Dan`，其余为标准 MIT license 全文。

- [ ] **Step 4: 验证链接与提交**

```bash
test -f README.md && test -f CONTRIBUTING.md && test -f LICENSE
rg -n "scripts/verify.sh|AGENTS.md|docs/README.md" README.md
git add README.md CONTRIBUTING.md LICENSE
git commit -m "docs: add project and contribution entry points"
```

### Task 5: 统一文档索引与状态约定

**Files:**
- Create: `docs/README.md`
- Modify: `CODE_REVIEW_20250619.md`
- Modify: `Todo.md`

- [ ] **Step 1: docs 索引定义唯一状态头**

`docs/README.md` 定义新计划文件以 `docs/plans-` 开头、主题以小写连字符命名并以 `.md` 结尾；首部必须有 `Status: draft|active|done|superseded`、`Owner`、`Supersedes`；列出本次 8 份 code-quality 计划及执行顺序；历史完成文档移动到 `docs/archive/`，但本任务不批量移动 123 个旧文档。

- [ ] **Step 2: 修正审计文件日期语义**

保留文件以避免断链，在首部添加：

```markdown
> Status: superseded
> Date: 2026-06-19
> Superseded by: `docs/plans-code-quality-remediation-overview.md`
> Note: filename contains the legacy 2025-style date and is retained for link compatibility.
```

- [ ] **Step 3: Todo 只保留产品 backlog**

删除“咒语”和 agent 执行指令；按 Search、Markdown Preview、Replace、Tabs、Ideas 分类；重复的“自动排版/章节识别”合并为单项。每项写可观察问题，不夹带执行方式。

- [ ] **Step 4: 提交**

```bash
git add docs/README.md CODE_REVIEW_20250619.md Todo.md
git commit -m "docs: index plans and separate product backlog"
```

### Task 6: 移除跟踪的 crash dump 并清理迁移残留

**Files:**
- Delete: `crash.log`
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] **Step 1: 从 Git 删除 crash dump**

```bash
git rm crash.log
git check-ignore -v crash.log
```

Expected: 第二条指出 `.gitignore` 的 `*.log` 或 `crash.log` 规则。

- [ ] **Step 2: 校正 App 字段注释**

逐字段检查注释紧邻其实际字段；删除描述已不存在字段的孤立注释。只改注释，不重排字段、不改变类型。

- [ ] **Step 3: 缩小 lib.rs 中 allow 和 re-export**

删除 `#[allow(unused_imports)]`；没有外部使用的 import/re-export 直接删除。保留 allow 时必须使用 `reason = "外部契约 + 删除条件"`，不得留无原因的 `allow(dead_code)`。

- [ ] **Step 4: 验证与提交**

```bash
git ls-files '*.log'
cargo clippy -p edit-plus-app --all-targets -- -D warnings
git add crates/app/src/app.rs crates/app/src/lib.rs
git commit -m "chore: remove crash dump and stale app annotations"
```

Expected: `git ls-files '*.log'` 无输出；Clippy PASS。`crash.log` 的 staged deletion 与两个源码文件合计 3 个文件。

### Task 7: 最终构建矩阵

**Files:**
- No files changed.

- [ ] **Step 1: 运行默认与 release 验证**

```bash
./scripts/verify.sh
cargo check --workspace --all-targets --release
cargo test --workspace --release
```

Expected: 全部 PASS；macOS arm64 为必需矩阵。x86_64 macOS 在 CI 可用 runner/交叉 target 就绪后加入，不在本任务伪造未运行的结果。
