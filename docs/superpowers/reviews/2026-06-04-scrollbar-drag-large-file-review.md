# 超大文件快速拖动滚动条实现审查

> 审查日期：2026-06-04
> 范围：当前实现中“超大文件快速拖动滚动条”的交互、渲染、缓存、DisplayRow/WrapIndex 映射路径。
> 主要文件：`crates/app/src/app.rs`、`crates/app/src/scrollbar.rs`、`crates/app/src/render_pipeline.rs`、`crates/app/src/render_cache.rs`、`crates/app/src/wrap_index.rs`、`crates/app/src/display_line_map.rs`。

## 总结

当前实现已经具备滚动条拖拽交互，也有“拖动/滚轮期间复用上一帧顶点并做 NDC 平移”的快速路径。但这个方案更像临时视觉补丁，还没有形成稳定的大文件滚动架构。

核心问题是：拖动中 `scroll_top` 会实时跳到目标 DisplayRow，但文本区域可能仍复用旧屏幕的顶点；滚动条比例依赖的 `WrapIndex::total_display_rows()` 又是懒更新的，未访问过的长 wrap 行不会被准确计入总高度。结果是拖动时可能出现内容滞后、空白、滑块比例跳变，以及大跳跃时重新进入同步 shape 的卡顿。

## 关键问题

### P0：小幅连续拖动会无限平移旧顶点，不补新内容

位置：

- `crates/app/src/app.rs:1433`
- `crates/app/src/app.rs:1446`
- `crates/app/src/app.rs:1484`

当前逻辑在 debounce/drag 期间，如果本帧 `scroll_delta_rows < visible_rows * 0.5`，就 clone `cached_shape_vertices` 并整体平移。随后又把这批“已经平移过的旧顶点”写回 `cached_shape_vertices`。

这意味着连续慢拖或中速拖动时，每一帧看起来都是小位移，但累计已经跨过很多屏，文本仍然来自起始区域，只是不断被移走。用户可见效果可能是：

- 屏幕上半部或下半部出现空白。
- 文本内容明显滞后于滚动条位置。
- 松手或 debounce 结束后内容突然跳到正确位置。

### P0：快速拖动的大跳跃仍会回到完整渲染路径

位置：

- `crates/app/src/app.rs:1443`
- `crates/app/src/app.rs:1299`
- `crates/app/src/render_pipeline.rs:498`

大幅跳跃时当前代码放弃顶点平移，走 `shape_visible_lines()`。但 `shape_visible_lines()` 固定传 `max_miss_shapes = 15`，而 render pipeline 里用 `max_miss_shapes > 15` 判断 `is_rapid`。

因此这些快速滚动分支实际上不会启用：

- rapid scroll 下使用单视觉行 wrap。
- rapid scroll 下跳过高亮 span 收集。
- rapid scroll 下跳过 glyph rasterization。
- rapid scroll 下避免污染 RenderCache。

实际生效的只有“miss 超过 15 行后跳过 shape”。这会造成目标区域内容不完整，而且不能真正保证拖动大跳跃帧稳定低成本。

### P0：滚动条比例依赖的总 DisplayRow 数不可靠

位置：

- `crates/app/src/app.rs:237`
- `crates/app/src/scrollbar.rs:83`
- `crates/app/src/render_pipeline.rs:872`
- `crates/app/src/wrap_index.rs:75`

打开文件时 `WrapIndex::new(line_count)` 默认每个 doc line 只有 1 个 visual line。真实 wrap 数只有在行进入可见区域、经过 shape 后，才通过 `wrap_index.update_batch()` 增量写入。

超大 JSON 或包含大量长行的文件里，大部分文件内容在首次拖动时还没有被 shape。此时 `wrap_index.total_display_rows()` 偏小，导致：

- 滑块高度偏大。
- `max_scroll` 偏小。
- 拖动到某个位置映射出的 `scroll_top` 偏离真实内容位置。
- 随着更多行被 shape，滑块比例和位置会跳变。

### P1：同一次拖动过程中 max_scroll 可能变化

位置：

- `crates/app/src/app.rs:2047`
- `crates/app/src/scrollbar.rs:275`

拖动过程中每次 `CursorMoved` 都重新用当前 `WrapIndex::total_display_rows()` 计算 layout。若拖动过程中渲染了新区并更新了 wrap 计数，`max_scroll` 会变化。

这会让同一次拖拽中的鼠标位移映射不稳定：用户手指没有明显变化，内容位置却可能因为总高度修正而跳动。

### P1：拖拽末尾 NDC 修正当前基本无效

位置：

- `crates/app/src/app.rs:1615`
- `crates/app/src/app.rs:1624`
- `crates/app/src/app.rs:1629`

“拖拽末尾修正”在 `queue.write_buffer()` 和 draw pass 编码之后修改本地 `vertices`，不会影响当前帧已经提交给 GPU 的数据。

即便调整到上传前，这段代码也会平移所有顶点。注释里也提到会影响 selection、cursor、overlay 等元素。正确做法应该是将文本/gutter 顶点与 tab/status/scrollbar/cursor/selection 分层或分 buffer 处理。

### P1：快速路径仍有大量 CPU 拷贝和 GPU 上传

位置：

- `crates/app/src/app.rs:1446`
- `crates/app/src/app.rs:1484`
- `crates/app/src/app.rs:1538`

即使不 shape，当前快速路径仍然会：

- clone 全量 `cached_shape_vertices`。
- 平移 clone 出来的 Vec。
- 再 clone 一份存回缓存。
- 将整批顶点重新写入 GPU vertex buffer。

对于超长可见行或大量 glyph 的屏幕，这部分 CPU 内存带宽和 GPU upload 仍然可能成为卡顿来源。

### P1：RenderCache 命中路径仍分配很多临时 Vec

位置：

- `crates/app/src/render_cache.rs:91`
- `crates/app/src/render_pipeline.rs:333`

`CachedLine::emit_vertices_for_visual_line()` 每个 visual line 都创建一个新的 `Vec<GlyphVertex>`，调用方再 `extend()`。这减少了 shape 成本，但没有做到低分配渲染。

更适合大文件滚动的接口是把目标 `Vec<GlyphVertex>` 传进去，让 cache hit 直接 append，避免每视觉行一份临时 Vec。

### P1：DisplayLineMap 还没有成为滚动主路径

位置：

- `crates/app/src/display_line_map.rs:153`
- `crates/app/src/display_line_map.rs:162`
- `crates/app/src/render_pipeline.rs:838`

render pipeline 会用 `display_map.update_entry_in_place()` 写入真实 wrap breaks，但该方法明确不重建 tree。当前滚动条比例和 `scroll_top` 映射仍然依赖 `WrapIndex`。

这导致 `DisplayLineMap` 目前更像旁路缓存，尚未承担设计文档中 Snapshot/Patch 模型的主职责。

### P1：ReshapeWorker 是悬空能力，没有接入拖动路径

位置：

- `crates/app/src/reshape_worker.rs:47`

worker 模块存在，但 `App` 没有持有、spawn、submit、poll 它。大文件拖动时无法后台预计算目标区域 wrap，也无法在主线程预算用尽时异步补齐 DisplayLineMap。

### P2：超长行 shaping 限制没有使用设置项

位置：

- `crates/app/src/settings.rs:28`
- `crates/app/src/render_pipeline.rs:462`

`Settings::max_line_bytes_for_shaping` 字段存在，但 render pipeline 中实际使用的是写死的 `MAX_WRAP_BYTES`。这会让超长行策略难以配置，也不利于针对不同文件类型调优。

### P2：ScrollAnchor 字段存在但没有用于拖动/编辑稳定性

位置：

- `crates/app/src/viewport.rs:122`
- `crates/app/src/viewport.rs:222`
- `crates/app/src/viewport.rs:231`

`Viewport` 里有 `scroll_anchor`，但滚动条拖动仍直接读写 `scroll_top`。编辑和 resize 路径也没有系统性用 anchor 保持视口内容稳定。`restore_scroll_from_anchor()` 里还写死 `line_height = 14`，单位语义不可靠。

## 效果优化建议

1. 拖动中不要无限平移单屏顶点。至少维护 `viewport +/- overscan` 的缓存窗口，平移超过 overscan 后立即补齐目标区域。
2. 对拖动中的目标区域允许画低精度占位行，idle 后再替换为精确 shape 结果。
3. drag start 时固定一份 scrollbar model：`total_rows`、`max_scroll`、thumb geometry。拖动过程中用这份快照映射，松手或 idle 后再校准。
4. 把文本/gutter、selection/cursor、tab/status/scrollbar 分层渲染，避免用一个 Vec 平移所有 UI。
5. 滚动条比例需要区分 estimated total rows 和 exact total rows。未完成精确 wrap 时，可以稳定使用估算值，避免一边拖一边跳。

## 性能优化建议

1. 将 `RenderCache::emit_vertices_for_visual_line()` 改为 append 到调用方提供的 `Vec<GlyphVertex>`，减少临时 Vec。
2. 避免每帧 clone `cached_shape_vertices`。可以保留基础顶点 buffer + 单独 scroll uniform，或者至少复用一个 scratch Vec。
3. 拖动期间每帧设置明确预算：最多 shape N 行，最多 rasterize M 个 glyph，最多 GPU upload K bytes。
4. 接入 `ReshapeWorker`，拖动到新区域时后台预计算 wrap breaks，主线程只消费完成的结果。
5. 行号 glyph/顶点应单独缓存；当前 cache hit 路径仍可能 shape 行号。
6. 快速路径的 `is_rapid` 条件需要修正，否则 rapid 分支实际不可达。

## 内存优化建议

1. RenderCache 使用字节预算，而不是固定 1000 行。超长行可能让 1000 行远超预期内存。
2. `estimated_bytes` 应计入 `visual_lines`、`visual_line_instance_starts`、`cluster_data` 和 Vec capacity，而不仅是 glyph instances。
3. 对单行缓存设置上限：超长行只缓存可见切片或分段缓存，避免一行占据大量 glyph/cluster 内存。
4. 建立 atlas 驱逐反向索引。否则 atlas slot 失效后，RenderCache 里的 UV 可能继续被复用。
5. `DisplayLineMap::snapshot()` 当前额外包了一层 `Arc`，可以简化，减少快照开销。

## 建议测试用例

1. **连续慢拖**：打开 4MB+ JSON，从顶部慢慢拖到 80%，观察拖动中是否出现旧内容漂移和空白。
2. **一次性大跳跃**：打开大量长 wrap 行文件，直接把滑块从顶部拖到底部，检查首帧是否卡顿、内容是否完整。
3. **wrap 总高度修正**：文件前半短行、后半超长行，首次拖到后半部分，检查滚动条滑块是否明显跳变。
4. **拖动中 resize**：拖动滑块时改变窗口高度，检查 thumb、scroll_top、文本位置是否一致。
5. **cache hit 分配统计**：统计拖动 5 秒期间每帧 allocation 次数、顶点 clone 字节数、GPU upload 字节数。
6. **松手恢复**：松开滚动条后 50ms 内必须完成目标区域完整 render，且 `advance_cache`、cursor hit-test、selection 均与可见文本一致。
7. **超长单行**：单行超过 `MAX_WRAP_BYTES`，拖动经过该行时不能卡死，也不能让总 DisplayRow 明显失真。

## 优先级建议

1. 先修快速路径正确性：禁止连续无限平移旧顶点；修正不可达的 rapid 分支。
2. 再修滚动条模型稳定性：drag start 固定 `total_rows/max_scroll` 快照，idle 后校准。
3. 接着降低拖动帧成本：去掉全量顶点 clone，减少临时 Vec，分层上传。
4. 最后推进架构收敛：接入 `DisplayLineMap` Snapshot/Patch 和 `ReshapeWorker`，逐步让 `WrapIndex` 退出主路径。
