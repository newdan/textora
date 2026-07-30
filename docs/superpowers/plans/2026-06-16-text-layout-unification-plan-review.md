# 执行计划评审：TextLayout 统一文本渲染管线

这份执行计划非常详尽，很好地将之前讨论的架构设计落实到了具体的代码步骤中。通过 `DrawCmd::TextLayout` 携带 `Arc<UiTextLayout>` 并在 `drain()` 中查询缓存，完美兼顾了性能和架构边界。

但在通读代码设计后，我发现 **Task 1 与 Task 4 之间存在一个逻辑冲突（关于 Cache Key）**，需要稍作修正，否则会导致 Preview 管线的缓存完全失效。

## 关键修正建议 (Blocker for Performance)

### 1. `UiTextLayout` 的自增 ID 与 Preview 管线相冲突
**现象**：
- 在 **Task 1** 中，`UiTextLayout` 定义了一个全局自增的 `NEXT_LAYOUT_ID`，并在 `new()` 和 `from_shaped()` 时递增分配。
- 在 **Task 4 (Step 4)** 中，Preview 每帧遍历 `LaidOutLine` 时，都会调用 `UiTextLayout::from_shaped`。
- **冲突**：这意味着 Preview 中同一行文字，每一帧都会产生一个新的、具有**不同自增 ID** 的 `UiTextLayout`。如果缓存 Key 使用这个 ID，那么 Preview 将永远无法命中缓存（Cache Miss 率 100%），并且会迅速占满 `RenderCache`。

### 2. 解决方案：用 `content_hash` 彻底替代自增 ID
执行计划的作者似乎也意识到了这个问题，所以在 **Task 4 (Step 5)** 的 `drain()` 里，代码并没有使用 `layout.id`，而是临时计算了一个 `content_hash`。
为了让逻辑更严密且减少每帧哈希的开销，建议如下调整：

**在 Task 1 中：**
- 移除 `NEXT_LAYOUT_ID` 和 `AtomicU64`。
- `UiTextLayout` 新增字段 `pub cache_key: u64`。
- 在 `UiTextLayout::new` 和 `from_shaped` 的构造函数内部，直接计算并保存这个 hash 值。
- **注意哈希因子的完整性**：计划中的 hash 只算了 text、size、weight。请务必将 `font_style` (Normal/Italic) 和 `font_family` 也加入哈希计算，否则斜体/不同字体的同一段文字会发生渲染串键碰撞。

**在 Task 4 中：**
- `drain()` 函数里不再需要现场算 hash，直接使用 `layout.cache_key` 查询缓存即可。

### 3. 缓存命名优化
- 在 **Task 4** 中，缓存被命名为 `preview_cache` 和 `PreviewRenderCache`。
- 由于在 **Task 6** 中，UI Widgets 也使用了 `DrawCmd::TextLayout`，所以 UI 文本实际上也会存入这个 Cache。
- 建议将其重命名为 `text_layout_cache` 或 `shared_render_cache`，因为它现在统一服务于 UI 和 Preview 管线了。

## 其他小细节（非致命）
- **Task 4 (Step 5)** 中发射顶点时调用的 `emit_from_instances(..., sw, sh, ...)` 中的 `sw, sh` 变量名似乎没有在上下文中定义，实际应该传入外层的 `screen_w, screen_h`。
- **Task 6 (Widget 迁移)**：非常棒！引入 `label_layout: Option<Arc<UiTextLayout>>` 是完美的按需更新机制，UI 性能将得到质的飞跃。

## 总结
除开 Cache Key 逻辑上的小冲突之外，这份计划极度专业，拆解得非常合理。请参考上述修改意见调整 `UiTextLayout` 的 ID / Hash 生成逻辑，之后即可放心安排 Agent 按照 Checklist 执行代码迁移了！
