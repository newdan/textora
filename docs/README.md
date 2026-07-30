# docs 索引

## 文件命名约定

- 新计划文件统一使用 `docs/plans-` 前缀，主题以小写连字符命名，`.md` 后缀。
- 存量文件可能不完全符合此约定（如缺少 `code-quality-` 前缀或带编号后缀），**新文件必须遵循**。
- 每份计划文件首部必须包含以下元数据（YAML-front-matter 或 blockquote 均可）：

  ```markdown
  > Status: draft | active | done | superseded
  > Owner: <责任人>
  > Supersedes: <被取代的文件，若无则省略>
  ```

## Code Quality Remediation 计划执行顺序

本次共 8 份计划，按以下顺序依次推进（每份只依赖前序计划的公开验收结果）：

| # | 文件 | 阶段 | 说明 |
|---|------|------|------|
| 0 | [`plans-code-quality-remediation-overview.md`](plans-code-quality-remediation-overview.md) | 总览 | 架构约束与总体执行策略 |
| 1 | [`plans-code-quality-phase0-trusted-baseline.md`](plans-code-quality-phase0-trusted-baseline.md) | Phase 0 | 恢复测试与 all-targets 编译可信基线 |
| 2 | [`plans-code-quality-phase1-quality-gates.md`](plans-code-quality-phase1-quality-gates.md) | Phase 1 | 建立自动质量门禁 |
| 3 | [`plans-code-quality-phase2a-soundness.md`](plans-code-quality-phase2a-soundness.md) | Phase 2a | unsafe / soundness 收口 |
| 4 | [`plans-code-quality-phase2b-persistence.md`](plans-code-quality-phase2b-persistence.md) | Phase 2b | 持久化层收口 |
| 5 | [`plans-code-quality-phase3-app-boundaries.md`](plans-code-quality-phase3-app-boundaries.md) | Phase 3 | app 层边界收口 |
| 6 | [`plans-code-quality-phase4-ui-boundaries.md`](plans-code-quality-phase4-ui-boundaries.md) | Phase 4 | UI 层边界收口 |
| 7 | [`plans-code-quality-phase5-maintenance.md`](plans-code-quality-phase5-maintenance.md) | Phase 5 | 构建与仓库维护 |

另有 [`plans-code-quality-remediation.md`](plans-code-quality-remediation.md) 为历史版本，已被 `overview` 取代，保留作参考。

## 历史文档

已废弃的计划和审计文档最终将移至 `docs/archive/`，但本轮任务**不批量移动**（涉及 123+ 个旧文件）。后续如有需要再单独处理。

## 遗留问题

Phase 5 执行后的遗留问题与建议见 [`plans-code-quality-phase5-findings.md`](plans-code-quality-phase5-findings.md)。
