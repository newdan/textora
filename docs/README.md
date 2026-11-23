# docs 索引

## 唯一架构入口

[新人接手指南](onboarding.md)是当前架构的唯一阅读入口：从双产品启动点、状态归属和编辑、保存、点击三条链路开始。依赖与分层约束以 [AGENTS.md](../AGENTS.md) 为准；下面的计划和归档用于查设计背景，不代表当前实现。

根目录迁出的历史草稿：[补丁一](archive/patch.diff)、[补丁二](archive/patch2.diff)、[旧修复计划](archive/plans_bug_fix_2026_06_13.md)、[旧问题清单](archive/issues.md)、[旧写作计划](archive/new_writing_plans.md)。

## 文件命名约定

- 新计划放在 `docs/plans/`，文件名以日期开头，主题以小写连字符命名。
- 每份计划文件首部必须包含以下元数据（YAML-front-matter 或 blockquote 均可）：

  ```markdown
  > Status: draft | active | done | superseded
  > Owner: <责任人>
  > Supersedes: <被取代的文件，若无则省略>
  ```

## 历史代码质量整改计划

以下 8 份归档按原执行顺序排列。

| # | 文件 | 阶段 | 说明 |
|---|------|------|------|
| 0 | [`plans-code-quality-remediation-overview.md`](archive/plans-code-quality-remediation-overview.md) | 总览 | 架构约束与总体执行策略 |
| 1 | [`plans-code-quality-phase0-trusted-baseline.md`](archive/plans-code-quality-phase0-trusted-baseline.md) | Phase 0 | 恢复测试与 all-targets 编译可信基线 |
| 2 | [`plans-code-quality-phase1-quality-gates.md`](archive/plans-code-quality-phase1-quality-gates.md) | Phase 1 | 建立自动质量门禁 |
| 3 | [`plans-code-quality-phase2a-soundness.md`](archive/plans-code-quality-phase2a-soundness.md) | Phase 2a | unsafe / soundness 收口 |
| 4 | [`plans-code-quality-phase2b-persistence.md`](archive/plans-code-quality-phase2b-persistence.md) | Phase 2b | 持久化层收口 |
| 5 | [`plans-code-quality-phase3-app-boundaries.md`](archive/plans-code-quality-phase3-app-boundaries.md) | Phase 3 | app 层边界收口 |
| 6 | [`plans-code-quality-phase4-ui-boundaries.md`](archive/plans-code-quality-phase4-ui-boundaries.md) | Phase 4 | UI 层边界收口 |
| 7 | [`plans-code-quality-phase5-maintenance.md`](archive/plans-code-quality-phase5-maintenance.md) | Phase 5 | 构建与仓库维护 |

另有 [`plans-code-quality-remediation.md`](archive/plans-code-quality-remediation.md) 为历史版本，已被 `overview` 取代，保留作参考。

## 历史文档

根目录的旧计划已移入 [archive/](archive/)。

## 遗留问题

Phase 5 执行后的遗留问题与建议见 [`plans-code-quality-phase5-findings.md`](archive/plans-code-quality-phase5-findings.md)。
