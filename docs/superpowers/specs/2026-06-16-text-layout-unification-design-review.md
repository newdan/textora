# 评审意见：TextLayout 统一文本渲染管线

整体来看，将 Harfbuzz Shaping（CPU）与 Atlas Rasterize（GPU）分离的思路非常正确。这能有效解决 Preview 和 UI 每帧重复进行昂贵文本 shape 计算的性能浪费问题，并且通过复用 `CachedLine` 统一发射顶点的设计也很优雅。

但在 **UI 管线改动** 以及 **架构依赖** 方面，当前的设计草案存在两个致命的缺陷，必须在实施前进行修正。

## 致命问题 (Blockers)

### 1. Z-Order（渲染层级）会被完全破坏
**设计草案指出**：UI 组件直接 `emit_vertices()` 跳过 `DrawList::Text`，并删除 `DrawCmd::Text`，`DrawList` 退化为纯形状绘制。
**问题所在**：在现有的 UI 体系中，`DrawList` 的核心价值之一是**保证渲染的先后层级**。如果背景 Shape 被推入 `DrawList`，而文本在 Widget `paint()` 阶段直接发射到 Vertex Buffer 中，那么所有的文本将会与所有的背景脱离层级关系。
**后果**：文本可能会被底部的 Shape 覆盖（看不见），或者悬浮菜单底下的文本会穿透到菜单上方。这破坏了诸如 Popup、Tooltip、Button 等需要严格“背景 -> 文本 -> 叠加层 -> 叠加文本”顺序的组件。

### 2. 违反依赖与架构分层
**设计草案指出**：UI 的 widget 渲染时直接 `emit_vertices()`。
**问题所在**：根据 `AGENTS.md` 中的架构规范，`crates/ui` 是纯 UI 组件库，它**不依赖也不应知道**底层的 `GpuState`、`TextState` 以及 `RenderCache` 的实现（这些都在 `crates/app` 中）。如果 widget 内部直接调用 `emit_vertices()`，就必须持有 Atlas 引用和 GPU 队列，这将导致 `ui` 与 `app` 强耦合，甚至产生循环依赖。

## 改进方案与建议

为了解决上述问题，同时实现避免 UI 和 Preview 每帧重新 shape 的目标，建议采用以下修订方案：

### 1. 将 DrawCmd::Text 升级为 DrawCmd::TextLayout
**不要删除** `DrawCmd` 中的文本支持，而是升级它。
- 在 `ui` crate 中设计纯数据驱动的 `TextLayout` 结构体，内部持有（或缓存）已经完成 Harfbuzz Shape 的 `ShapedRun` 数据。
- 当 widget 状态改变时，更新并重新 shape 生成新的 `TextLayout`。
- widget 在 `paint()` 时，向 `DrawList` 发射类似 `DrawCmd::TextLayout(Arc<TextLayout>, x, y)` 的命令，而不是字符串。

### 2. 在 app 的 drain 阶段统一 Emit
`paint_backend::drain()` 继续负责消费 `DrawList`：
- 当遍历到 `DrawCmd::TextLayout` 时，因为已经包含了 `ShapedRun`（避免了重新 shape），`app` 层只需拿着这个 `ShapedRun` 去查询 UI 专属的 `RenderCache`。
- 命中缓存直接拿到 `CachedLine` 并合并顶点；未命中则查 Atlas（`resolve_glyph`）生成 `CachedLine` 存入 Cache，然后再合并顶点。
- **这样既利用了统一缓存、省去了 shape，又完美保持了 Z-Order，且遵守了 `ui` 只输出意图，`app` 负责绘制的架构原则。**

### 3. RenderCache Key 的设计
赞同目前的设计。由于各个场景生命周期和命中策略不同：
- **Editor**：按需缓存，保持 `usize` (行号) 作为 Key。
- **Preview**：使用独立的 Cache 实例，Key 采用 `content_hash`。
- **UI**：如果采用 `DrawCmd::TextLayout`，可以给每个 `TextLayout` 分配一个全局唯一的 ID (`u64`) 作为 Cache Key，UI 专属的 `RenderCache` 使用该 ID 进行 Atlas 缓存检索。

### 4. 关于内存增加的风险
Step 1 中提到 `LaidOutLine` 新增字段持有 `ShapedRun`。
- 对于 Markdown Preview，这在常见文档长度下是完全可以接受的（通常占用不大）。
- 建议在设计中补充一点：当 Preview View 被关闭或长时间隐藏时，应清理关联的 `RenderCache` 避免内存泄漏。

## 总结
该方案的“核心设计 (Harfbuzz 与 Atlas 分离)” 极为优秀。只需修正 UI 层的下发方式（保留在 `DrawList` 中传递已 Shape 的数据结构，推迟到 `app` 渲染），该方案即可完美落地。
