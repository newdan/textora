# Code Quality Remediation Program Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 2026-06-19 代码质量审计中的 P0/P1/P2 整改项拆成可独立交付、验证和回滚的执行计划，并最终恢复“一条命令验证整个 workspace”的工程约束。

**Architecture:** 先恢复功能和 all-targets 基线，再建立自动门禁；随后分别收口 unsafe、持久化、app 和 UI 边界，最后处理构建与仓库卫生。每份子计划只依赖前序计划公开的验收结果，每个原子任务最多修改 3 个文件。

**Tech Stack:** Rust 1.93 stable、Cargo workspace、rustfmt、Clippy、Criterion、GitHub Actions、macOS arm64。

---

## 基线与约束

- 审计报告：`/Users/dan/.codex/worktrees/40f2/edit+/docs/code-quality-audit-2026-06-19.md`
- 审计基线：`a18aee64`
- 计划编写基线：`5263555a`
- 当前工具链实测：`rustc 1.95.0`、`cargo 1.95.0`；计划要求仓库锁定审计声明的最低 stable `1.93.0`。
- 每份计划开始前执行 `git status --short`；非空时不得覆盖或顺手整理用户改动。
- 每次提交前至少执行该任务列出的定向测试和 `cargo check`；逻辑整改与全仓格式化必须分开提交。

## 子计划与执行顺序

| 顺序 | 子计划 | 覆盖审计项 | 独立验收结果 |
|---:|---|---|---|
| 1 | `docs/plans-code-quality-phase0-trusted-baseline.md` | P0-1、P0-2、P2-1 的阻断项 | ✅ 8 个替换测试通过，bench 可编译，重复/遗漏测试标记修正 |
| 2 | `docs/plans-code-quality-phase1-quality-gates.md` | P1-1、P2-1 其余项 | ✅ f4f8e6c3 `./scripts/verify.sh` 与 CI 四项门禁全绿 |
| 3 | `docs/plans-code-quality-phase2a-soundness.md` | P1-2 | ✅ 2e32d692 安全 public API 不再通过 `static mut` 懒初始化/分派 |
| 4 | `docs/plans-code-quality-phase2b-persistence.md` | P1-5、P1-3 的 persistence 部分 | ✅ 852533d9 所有 app 用户状态通过统一原子写入并传播错误 |
| 5 | `docs/plans-code-quality-phase3-app-boundaries.md` | P1-3、P2-4 app 部分 | app 不再穿透 active view/doc，动作副作用统一 |
| 6 | `docs/plans-code-quality-phase4-ui-boundaries.md` | P1-4、P2-4 UI 部分 | UI 主题解析不做 I/O，widget 输入不依赖 thread-local Settings |
| 7 | `docs/plans-code-quality-phase5-maintenance.md` | P2-2、P2-3、P2-4 其余项 | 平台/profile/依赖策略和仓库文档约定明确 |

不得并行执行 Phase 0 与 Phase 1，也不得在 Phase 1 门禁稳定前开始 Phase 3/4。Phase 2A 与 2B 可在 Phase 1 完成后并行；Phase 5 可在 Phase 3/4 后执行，避免文档描述再次过期。

### Task 1: 记录整改状态与证据

**Files:**
- Modify: `docs/plans-code-quality-remediation-overview.md`

- [ ] **Step 1: 在每份子计划完成后更新状态表**

在对应行的“独立验收结果”开头添加 `✅` 和实际的 7–40 位十六进制提交 SHA；不得仅写“完成”。

- [ ] **Step 2: 执行最终验收**

Run:

```bash
./scripts/verify.sh
git status --short
```

Expected: verify 四阶段全部退出码为 0；`git status --short` 无输出。

- [ ] **Step 3: 提交总状态更新**

```bash
git add docs/plans-code-quality-remediation-overview.md
git commit -m "docs: complete code quality remediation program"
```

## 审计覆盖矩阵

- P0-1 → Phase 0 Tasks 1–2。
- P0-2 → Phase 0 Task 3。
- P1-1 → Phase 1 Tasks 1–5。
- P1-2 → Phase 2A Tasks 1–4。
- P1-3 → Phase 2B Tasks 4–6；Phase 3 Tasks 1–9。
- P1-4 → Phase 4 Tasks 1–7。
- P1-5 → Phase 2B Tasks 1–6。
- P2-1 → Phase 0 Task 4；Phase 1 Tasks 3–4。
- P2-2 → Phase 5 Tasks 1–3。
- P2-3 → Phase 5 Tasks 4–5。
- P2-4 → Phase 3 Task 9；Phase 4 Tasks 6–7；Phase 5 Task 6。
