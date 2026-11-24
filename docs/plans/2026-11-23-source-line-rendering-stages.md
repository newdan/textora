# 纯文本可见行渲染阶段

本记录对应《中长期架构演进计划》的专题 B。

- `resolve_visible_line_range` 从滚动锚点、文档长度和光标位置计算候选行范围及排版停止条件。
- `lookup_or_prepare_cached_line` 计算包含文本、排版宽度和源语义的缓存键，并失效旧条目。缓存命中保留长行子集的完整视觉行数。预编辑光标行使用当帧排版。
- `shape_and_wrap_line` 利用已有换行记录或按当前视口重新排版。长行先确定覆盖可见行和光标的字节区间。单次高亮查询提供字形样式和顶点颜色。
- 正文和行号由顶点输出阶段处理。缓存、即时排版与空行占位共用行号绘制。命中测试保留视觉行字节偏移、字素偏移和光标横坐标。
- `EditorFrame::drain_into` 生成最终顶点，`RenderResources::submit_surface_frame` 提交 GPU。已移除无实际提交行为的 `EditorFrame::present`。

`./scripts/verify.sh` 通过，共享壳层有 530 个测试通过，1 个忽略。
