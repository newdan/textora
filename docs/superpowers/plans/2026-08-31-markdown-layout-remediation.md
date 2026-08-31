# Markdown Layout Remediation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 每个嵌套源空行 run 只补偿一次，并让多输出块精排只传播各槽位自身高度变化一次。

**Architecture:** LayoutCtx 维护本次布局已补偿的源行边界集合，父子容器共享同一去重状态。LazyLayout 多输出整组重排先把每个新块归一到对应 estimated_positions，再以 rect.h 与旧槽位高度之差作为独立 delta；y_delta 继续是唯一的累计位移来源。

**Tech Stack:** Rust、textora-markdown layout engine、LazyLayout、BlockSource 测试桩。

## Global Constraints

- 保持 displayed_y = block.rect.y + y_delta[slot] 的既有不变量。
- total_height 对每个槽位自身高度 delta 只累计一次。
- 空行补偿只在 source_text 存在的编辑布局生效；预览布局行为不变。
- 不在公开 LaidOutBlock 或 BlockSource 上增加状态字段。
- 两项布局修复分别以独立测试和提交完成。

---

## Task 1: 锁定嵌套空行 run 的重复补偿

**Files:**
- Modify: crates/markdown/src/layout/block.rs

- [ ] 增加 nested_list_blockquote_blank_run_is_reserved_once。以 - a\n\n\n  > b 构造编辑布局，记录 b 所在 LaidOutLine 的 y；再以 - a\n\n  > b 构造对照。断言多出的一个可编辑空行只增加 style.line_height，而不是两倍。

- [ ] 同一测试再覆盖 > a\n>\n>\n> - b 的容器顺序，确保去重键不依赖父容器类型。

- [ ] 运行测试并确认 RED，实际差值应为 2 * style.line_height：

    cargo test -p textora-markdown --lib layout::block::tests::nested_list_blockquote_blank_run_is_reserved_once -- --exact

- [ ] 提交测试：

    git add crates/markdown/src/layout/block.rs
    git commit -m "test(markdown): cover nested blank run compensation"

## Task 2: 在 LayoutCtx 中按源行边界去重

**Files:**
- Modify: crates/markdown/src/layout/context.rs
- Modify: crates/markdown/src/layout/block.rs

- [ ] 在 LayoutCtx 增加 compensated_blank_run_line_starts: HashSet<usize>，并在 new 中初始化为空集合。

- [ ] 将 count_preceding_blank_source_lines 改为返回语义化值：

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct PrecedingBlankSourceRun {
        following_line_start: usize,
        blank_line_count: usize,
    }

  following_line_start 是 byte 所在物理行的稳定起点；父层 list child 与内层 blockquote child 即使 block_range.start 不同，也映射到同一键。

- [ ] 给 LayoutCtx 增加 reserve_blank_source_run(following_line_start, blank_line_count) -> f32。首次插入键时返回 blank_line_count.saturating_sub(1) * style.line_height，重复键返回 0.0。

- [ ] reserve_nested_blank_source_lines 只负责提取 PrecedingBlankSourceRun 并把返回高度累加到 ctx.y；移除父子层各自无条件补偿的行为。

- [ ] 运行回归和相关布局测试：

    cargo test -p textora-markdown --lib layout::block::tests::nested_list_blockquote_blank_run_is_reserved_once -- --exact
    cargo test -p textora-markdown --lib layout::block::tests

- [ ] 格式化并检查包：

    cargo fmt --all -- --check
    cargo check -p textora-markdown

- [ ] 提交空行修复：

    git add crates/markdown/src/layout/context.rs crates/markdown/src/layout/block.rs
    git commit -m "fix(markdown): deduplicate nested blank runs"

## Task 3: 锁定多输出槽位 delta 的重复传播

**Files:**
- Modify: crates/markdown/src/layout/types.rs

- [ ] 在现有 multi_output_source 测试附近新增 precise_multi_output_group_propagates_each_height_delta_once：

    - 使用根 BlockKind::Container，第一段为足够长的文本，第二段为 omega；
    - 用窄 viewport 创建 LazyLayout，记录精排前第二槽位 displayed y；
    - 调用 precise_block_at(0, ...)；
    - 用同样 shaper/style 对相同 source 做一次完整精确布局作为基准；
    - 断言第二槽位 rect.y + y_delta[1] 等于基准第二块 rect.y；
    - 断言 lazy.total_height 等于基准 total_height；
    - 再次精排同组，断言位置和总高不继续漂移。

- [ ] 运行测试并确认 RED；旧实现中第二槽位会额外叠加第一槽位的高度变化：

    cargo test -p textora-markdown --lib layout::types::tests::precise_multi_output_group_propagates_each_height_delta_once -- --exact

- [ ] 提交测试：

    git add crates/markdown/src/layout/types.rs
    git commit -m "test(markdown): cover multi-output delta propagation"

## Task 4: 将多输出块归一到估计坐标系

**Files:**
- Modify: crates/markdown/src/layout/types.rs

- [ ] 在 relayout_multi_output_group 的输出循环中，populate_style_segments 后先计算：

    let normalized_y_delta = self.estimated_positions[slot] - new_block.rect.y;
    shift_laid_out_block(&mut new_block, 0, normalized_y_delta);
    let old_height = self.estimated_heights[slot];
    let new_height = new_block.rect.h;
    let delta = new_height - old_height;

- [ ] estimated_heights[slot] 写入 new_height；retain_block_projections 与 laid_out 写入归一后的 new_block；total_height 只加该槽位 delta；只有 abs(delta) > 0.5 时加入 group_deltas。

- [ ] 删除 old_bottom/new_bottom 推导，确保后续槽位 rect.y 中的前序增长不会再被解释为自身高度。

- [ ] 运行新回归、现有 multi-output 测试和所有 layout tests：

    cargo test -p textora-markdown --lib layout::types::tests::precise_multi_output_group_propagates_each_height_delta_once -- --exact
    cargo test -p textora-markdown --lib multi_output
    cargo test -p textora-markdown --lib layout::

- [ ] 格式化并检查包：

    cargo fmt --all -- --check
    cargo check -p textora-markdown

- [ ] 自审：逐槽检查 rect.y 固定在 estimated_positions、y_delta 承担累计位移、total_height 等于估计总高加所有独立 delta。

- [ ] 提交多输出修复：

    git add crates/markdown/src/layout/types.rs
    git commit -m "fix(markdown): normalize multi-output layout deltas"

## Task 5: 跨模块最终验证

**Files:**
- Verify only

- [ ] 运行三个计划中的全部新增回归测试。

- [ ] 运行项目重大修改验证脚本：

    ./scripts/verify.sh

- [ ] 检查工作树，确认只包含计划内改动与用户原有未跟踪文件：

    git status --short
    git diff --check

- [ ] 对照设计文档逐项核验六个审查问题均有 RED/GREEN 证据，没有跨层依赖、魔法值、未使用引入或废弃 helper。
