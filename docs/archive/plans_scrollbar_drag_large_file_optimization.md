# 超大文件滚动条拖拽优化计划

> 基于 `docs/superpowers/reviews/2026-06-04-scrollbar-drag-large-file-review.md`

## 范围

本轮先做第一阶段优化，目标是修复拖拽快速路径中最容易造成错误显示和性能退化的两个问题：

1. 连续小步拖动不能无限复用并累积平移旧顶点。
2. 拖动/debounce 帧必须明确进入 rapid 渲染模式，不能依赖不可达的 `max_miss_shapes > 15` 判断。

暂不在本阶段接入 `DisplayLineMap` Snapshot/Patch、`ReshapeWorker`、RenderCache 字节预算和 atlas 反向索引。这些会涉及更多模块，应拆到后续阶段。

## 阶段 1：拖拽快速路径正确性

文件：

- `crates/app/src/app.rs`
- `crates/app/src/render_pipeline.rs`

步骤：

1. 先写回归测试：
   - 平移缓存基准必须保持在最后一次完整 render 的 `scroll_top`。
   - 当累计滚动超过阈值时，不再允许继续平移旧顶点。
   - debounce/drag 帧传入 render pipeline 的 rapid mode 必须为 true。
2. 实现：
   - 抽出 `can_translate_cached_shape()`。
   - 抽出 `update_cached_shape_vertices()`，translated frame 不更新 full-render cache。
   - 抽出 `shape_options_for_debounce()`，用显式 `rapid_mode` 替代 `max_miss_shapes > 15`。
3. 验证：
   - 跑相关单元测试。
   - 跑 `cargo test -p edit-plus-app --lib -- scrollbar_drag` 或相关过滤。
   - 跑 `cargo check -p edit-plus-app`。

## 完成标准

- 连续拖动不会把已平移的旧顶点写回完整渲染缓存。
- 大于半屏的累计拖动会强制走完整 shape 路径。
- render pipeline 的 rapid 判断由调用方显式传入。
- 测试覆盖上述行为。

## 阶段 1 后复核

完成后需要检查：

- 需求是否覆盖：本阶段只覆盖文档优先级 1 的一部分。
- 测试是否完整：覆盖缓存平移和 rapid mode，不覆盖真实 GPU 渲染截图。
- 残留漏洞：滚动条 total rows 仍依赖懒更新 `WrapIndex`，需要后续阶段处理。
- 性能缺陷：顶点 clone/GPU upload 仍存在，后续阶段继续优化。
