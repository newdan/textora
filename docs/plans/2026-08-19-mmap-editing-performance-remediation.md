# 思维导图（mmap）编辑性能与能耗修复方案

## 目标

消除 `MindmapView` 编辑思维导图节点标题时的主线程开销。当前每次按键都会丢弃整张导图的布局与命中
几何并全量重建，其中的文本测量（shaping）成本随节点数线性增长，约 200 节点起单键成本就超过一帧
预算。性能与能耗在这里是同一个根因：每键的全量 shaping 既是输入延迟的来源，也是电量消耗的主要
来源。

本方案不涉及 Markdown WYSIWYG 路径。那条路径由
`docs/plans/2026-08-19-markdown-editing-performance-remediation.md` 覆盖，两者根因不同：Markdown
侧 `parse` 与 `layout` 都是显著项且存在 O(n²) 的投影切片；mmap 侧 `parse` 可忽略，瓶颈是**无缓存
的 shaping 被全树重复调用**。

## 实测基线

release 构建，临时探针直接调用 `mmf::parser::parse` + `mmf::layout::compute_layout` +
`mmf::layout::build_hit_map`，每项 20 次取均值。样本为 root → 一级 → 二级的三层树，标题为中文短
句（典型形态）。

| 节点数 | 源码体积 | parse | compute_layout | build_hit_map | 单键合计 |
|---|---|---|---|---|---|
| 50 | 2.1 KB | 7.4 µs | 1.70 ms | 1.64 ms | 3.3 ms |
| 200 | 8.7 KB | 23 µs | 6.51 ms | 6.63 ms | 13.2 ms |
| 500 | 21.8 KB | 56 µs | 16.56 ms | 16.86 ms | 33.5 ms |
| 1000 | 44.2 KB | 112 µs | 33.69 ms | 34.20 ms | 68.0 ms |
| 2000 | 89.0 KB | 219 µs | 68.39 ms | 69.32 ms | 137.9 ms |
| 5000 | 223.4 KB | 547 µs | 171.55 ms | 172.46 ms | 344.6 ms |

`parse` 在 1000 节点时仅占 0.16%，**不是优化对象**。全部成本集中在两处 shaping：`compute_layout`
测卡宽、`build_hit_map` 算 grapheme 边界，各占约一半。

`Shaper::shape` 的单次成本（同一 shaper，2000 次取均值）：

| 场景 | 耗时 |
|---|---|
| 重复 shape 同一 CJK 标题 | 38.20 µs |
| shape 各不相同的 CJK 标题 | 35.87 µs |
| 重复 shape 纯 ASCII 标题 | 17.05 µs |

重复与不重复同价，证明**不存在 run 级缓存**。现有 `GraphemeAdvanceCache`（`shaping/src/lib.rs:67`，
LRU 4096）只覆盖 `grapheme_advance()` 的单 grapheme 回退路径，mmap 主路径走 `shape()`，命不中。

由此可推出单键 shape 次数为 **2 × N**（N 为展开节点数）：1000 节点 → 2000 次 × 34 µs ≈ 68 ms，与
上表吻合。**优化目标就是把这个 2N 降下来。**

阶段划分按「收益 ÷ 风险」排序：阶段 0 是零风险的死代码清理；阶段 1、2 是消除全树 shaping 的核心修
复，各自独立可验证；阶段 3 是跨视图受益的通用缓存；阶段 4、5 是能耗与 IME 体验。

---

## 阶段 0：删除 `HitMap` 的死代码字段

### 根因

`HitMap` 有两个标注为「过渡期只读兼容」的字段：

```443:446:crates/markdown/src/mmf/layout.rs
    /// 过渡期只读兼容字段；Task7 会改为消费 `nodes` 后移除。
    pub node_rects: Vec<Rect>,
    /// 过渡期只读兼容字段；其值按 grapheme 边缘生成，不能用作新的编辑命中数据。
    pub title_char_edges: Vec<Vec<f32>>,
```

全仓库检索确认：这两个字段**只有写入方，没有任何生产代码读取**——写入在 `build_hit_map`
（`layout.rs:458-459`、`488-489`、`507`），其余出现全部是 `canvas.rs` 测试里构造 `HitMap` 时填空
`Vec::new()`。它们是 Task7 遗留的未清理产物。

代价并非只是内存。`legacy_title_char_edges`（`layout.rs:621`）对每个节点遍历标题的
`char_indices()`，并对每个 char 做一次 `binary_search`，即每次按键额外付出 O(全树标题字符数) 的纯
浪费工作。

### 方案

删除 `node_rects`、`title_char_edges` 两个字段与 `legacy_title_char_edges` 函数，同步清理
`build_hit_map` 内的构造代码与 `canvas.rs` 测试中的初始化。

这是纯删除，无行为变更，也符合 `AGENTS.md`「提交前必删死代码」的要求。放在最前面做，可以让后续阶
段的基准数据不受这部分噪声干扰。

### 涉及文件

`crates/markdown/src/mmf/layout.rs`、`crates/markdown/src/mmf/canvas.rs`。

### 验收

- `cargo test -p textora-markdown --lib` 全绿，且 `cargo clippy -- -D warnings` 无未使用告警。
- 删除后 `build_hit_map` 的耗时应有小幅下降（预期个位数百分比），作为阶段 1、2 的干净基线。

---

## 阶段 1：标题测量缓存，消除 `compute_layout` 的全树 shaping

### 根因

`compute_layout`（`layout.rs:364`）中唯一的 shaping 来自 `collect_card_widths_by_depth` 对每个节点
调用一次卡宽测量：

```296:301:crates/markdown/src/mmf/layout.rs
    let title = projected_title
        .filter(|projected| projected.node_index == source_node_index)
        .map(|projected| projected.text)
        .unwrap_or_else(|| title_or_placeholder(&node.title));
    let card_w = measured_card_width_for_depth(title, constants, shaper, depth);
    out[depth_idx] = out[depth_idx].max(card_w);
```

`assign_positions` 与 `subtree_height` 都是纯几何计算，不含 shaping。因此**只要卡宽可以复用，
`compute_layout` 的 shaping 成本就能从 N 降到 1**（只有被编辑的那个节点标题真的变了）。

当前无法复用的原因是每次 `UpdateSource` 都 `mmf::parser::parse` 出一棵全新的 `Tree`
（`mindmap_view.rs:1274`），旧树整体丢弃，节点没有跨代身份。

### 方案

引入按内容寻址的标题宽度缓存，跨 `Tree` 重建存活。

- 新增模块 `crates/markdown/src/mmf/title_width_cache.rs`，提供 `TitleWidthCache`：
  - 缓存的是 `measure_text(title)` 在**深度缩放后字号**下的裸文本宽度，不含 padding 与
    `MIN_CARD_WIDTH`。padding 与下限是纯算术，留在 `measured_card_width` 里，避免常量变化时缓存
    失效面扩大。
  - key 为 `(depth, title)`。depth 必须进 key，因为字号按 `font_scale_for_depth(depth)` 缩放，同
    一标题在不同层级宽度不同。
  - 用 LRU 有界容量。打字过程中每个中间态标题都会产生一个条目，无界会持续增长。
- `compute_layout` 与 `collect_card_widths_by_depth` 增加 `&mut TitleWidthCache` 参数，
  `measured_card_width_for_depth` 内部先查缓存，miss 时 shape 并回填。
- 缓存归属 `MindmapView`（而非 `MindmapDocumentState::Ready`），因为它必须在 `clear_layout()` 与
  `Tree` 重建后继续存活——这正是它的全部价值所在。

### 必须一并修正的失效缺口

缓存的正确性依赖字号，而当前字号变化**不会**触发布局失效：`ensure_layout` 只是记录
`self.cached_font_size = shaper.font_size()`（`mindmap_view.rs:133`），从未参与比较；
`update_layout_constants`（`mindmap_view.rs:179`）只比较 dpi 与 `LayoutConstants`，而字号不属于
`LayoutConstants`。

这意味着当前存在一个先天缺陷：**仅改变编辑器字号时，mmap 会继续使用旧字号算出的布局**。实施前
先写一个复现测试确认（改 `shaper` 字号后 `ensure_layout` 是否重算），再按结果处理：

- 若确认是 bug：在 `update_layout_constants` 或 `ensure_layout` 中把字号纳入失效条件，并同时清空
  `TitleWidthCache`。
- 若已有其他路径覆盖：仍需把字号纳入 `TitleWidthCache` 的失效条件，不能只依赖外部路径。

无论哪种，`TitleWidthCache` 都必须在字号或 `LayoutConstants.depth_font_scales` 变化时整体清空。

### 涉及文件

`crates/markdown/src/mmf/title_width_cache.rs`（新增）、`crates/markdown/src/mmf/layout.rs`、
`crates/markdown/src/mindmap_view.rs`、`crates/markdown/src/mmf/mod.rs`（声明模块）。

超过 3 个文件，拆为两个子任务：

1. 新增 `title_width_cache.rs` 与 `mmf/mod.rs` 声明，含独立单元测试（命中/未命中/字号失效/LRU
   淘汰），不接入主路径。
2. 接入 `layout.rs` 与 `mindmap_view.rs`，含字号失效缺口的修正。

### 验收

- **等价性优先**：对同一棵树，带缓存的 `compute_layout` 与不带缓存的实现必须产出逐字段相等的
  `LayoutTree`。覆盖空标题（走 `EMPTY_TITLE_PLACEHOLDER`）、CJK、emoji（ZWJ 序列）、多层深度、
  以及 `ProjectedTitle` 生效时被投影的节点。
- **调用计数防护**：用计数器包裹 `shape()` 入口，断言「1000 节点、只改一个标题」的第二次
  `compute_layout` 中 shape 次数为 1（被编辑节点），而非 1000。计数断言比耗时断言在 CI 上稳定，
  作为首选防护。
- 1000 节点单键的 `compute_layout` 从 33.7 ms 降至亚毫秒量级。

---

## 阶段 2：命中几何惰性化，消除 `build_hit_map` 的全树 shaping

### 根因

`build_hit_map`（`layout.rs:449`）对 `layout.nodes` 中**每个**节点算一次 grapheme 边界，每次都是一
个完整 `shape()`：

```472:477:crates/markdown/src/mmf/layout.rs
        let title = projected_title
            .filter(|projected| projected.node_index == ln.source_node_index)
            .map(|projected| projected.text)
            .unwrap_or_else(|| title_or_placeholder(&node.title));
        let grapheme_byte_offsets = grapheme_byte_boundaries(title);
        let grapheme_edges = grapheme_edges(title, &grapheme_byte_offsets, text_x, shaper);
```

而实际消费者每次只需要**极少数节点**的几何：

| 消费者 | 位置 | 需要的节点 |
|---|---|---|
| `semantic_hit_target` | `mindmap_view.rs:593` | 指针命中的那一个 |
| `drag_request_hits_title` | `mindmap_view.rs:470` | 指针命中的那一个 |
| `cursor_screen_pos` | `mindmap_view.rs:651` | caret 所在节点 |
| `title_caret_navigation` | `mindmap_view.rs:799` | caret 所在节点 |
| `render_title_selection` / `render_preedit_underline` / `render_caret` | `canvas.rs:1281`、`1285`、`1286` | 可见节点 + caret 节点 |

### 为什么不能简单按视口裁剪

`title_caret_navigation`（键盘左右移动光标）与 `cursor_screen_pos`（IME 候选窗定位）按
`source_node_index` 查询几何，**不保证该节点在视口内**——用户可以滚动到别处后继续按方向键。若把
`build_hit_map` 裁剪到 `visible_node_indices`，这两条路径会静默失效（光标不动、候选窗错位）。

因此方案是**惰性求值 + 缓存**，而不是视口裁剪。惰性化在收益上等价（实际只算被查询的节点，通常就
是可见集加 caret 节点），但语义严格不变。

### 方案

把 `HitMap` 从「预计算的数组」改为「按需求值的缓存」。

- `HitMap` 保留 `controls` 为预计算。`build_control_hit_geometries`（`layout.rs:510`）是纯几何、
  不含 shaping，成本可忽略，且 `render_controls` 会遍历全部 controls。
- 节点几何改为惰性：`nodes: RefCell<HashMap<usize, NodeHitGeometry>>`（或按 `layout.nodes` 长度预
  分配的 `Vec<Option<_>>`），入口统一为
  `fn node_geometry(&self, source_node_index, shaper, ...) -> Option<&NodeHitGeometry>`。
- 惰性求值需要 `&mut Shaper`，但多数消费者当前在 `&self` 上下文。两种落地方式，实施时二选一并在
  子任务 1 中先确定：
  - **A（推荐）**：把「需要哪些节点的几何」显式化。渲染前在 `prepare_canvas` / `render_canvas`
    里为「可见集 ∪ caret 节点」预热几何（此处已有 `&mut Shaper`）；指针命中路径改为先用
    `card_rect`（纯几何，来自 `LayoutTree`，无需 shaping）筛出候选节点，再只对该节点求几何。
  - **B**：`HitMap` 内部持有 `Shaper` 的共享句柄。侵入性更大，仅在 A 无法覆盖某条路径时采用。
- 前置依赖阶段 0：`node_rects` 与 `title_char_edges` 若不先删除，会强制对全树求值、完全抵消本阶段
  收益。

### 涉及文件

`crates/markdown/src/mmf/layout.rs`、`crates/markdown/src/mindmap_view.rs`、
`crates/markdown/src/mmf/canvas.rs`。

拆为三个子任务：

1. 确定落地方式（A/B），`HitMap` 结构改造 + 单元测试，消费者暂时全部预热（行为等价、性能不变），
   保证可独立验证与回滚。
2. 指针命中路径改为 `card_rect` 预筛 + 单节点求值。
3. 渲染路径改为只预热「可见集 ∪ caret 节点」。

### 验收

- **等价性优先**：对构造的一批查询（每个节点的每个 grapheme 边界处点击、键盘逐字符移动穿过整个
  标题、IME 组合中查询 caret 位置），惰性实现与全量实现返回完全相同的 `EditHitTarget` 与
  `cursor_screen_pos`。
- **必须包含离屏节点用例**：滚动使 caret 节点离开视口后，`title_caret_navigation` 与
  `cursor_screen_pos` 仍返回与全量实现一致的结果。这是本阶段最容易回归的点。
- **调用计数防护**：1000 节点、视口内 20 个节点时，单键渲染路径的 shape 次数为 O(可见节点数) 而非
  1000。
- 1000 节点单键的 `build_hit_map` 从 34.2 ms 降至与可见节点数成正比。

---

## 阶段 3：`Shaper` run 级缓存，消除每帧重复 shaping

### 根因

阶段 1、2 解决的是「每键」，本阶段解决「每帧」。`render_text` 对每个可见节点重新 shape：

```829:841:crates/markdown/src/mmf/canvas.rs
        if !title.is_empty() {
            dl.text_shaped_with_font(
                text_origin.x,
                baseline_y,
                font_size,
                with_alpha(color, opacity),
                title,
                font_family.clone(),
                font_weight,
                font_style,
                false,
                shaper,
            );
        }
```

`text_shaped_with_font`（`ui/core/paint.rs:137`）内部无条件 `UiTextLayout::new(...)`，而
`UiTextLayout::new` 既 `shape()` 又 `text.to_string()`，并分配一个全新的自增 `id`
（`ui/core/text_layout.rs:159`）。

这带来两个后果：一是每帧对可见节点重复 shaping（30 个可见节点约 1.1 ms/帧）；二是
`PreviewRenderCache` 以 `layout.id` 为 key（`appkit-shell/paint_backend.rs:106`），mmap 每帧换新
id，**永不命中还持续挤占 1000 条 LRU 容量**，反过来污染其他视图的缓存。

### 方案

在 `Shaper` 内部增加 run 级缓存，与已有的 `GraphemeAdvanceCache` 并列：

- key 为 `(text, font_size_bits, weight, style, family)`。字号用 `to_bits()` 或量化整数，避免浮点
  作 key。
- value 为 `ShapedRun`（含 `clusters: Vec<GlyphCluster>`）。内存占用明显高于宽度缓存，容量需要实
  测标定，且必须在 `set_font_family` 等状态变更时保持 key 完备而非清空整表。
- 这一层同时兜住阶段 1、2 未覆盖的所有重复 shaping（含 `render_text` 每帧、`draw_invalid_canvas`、
  其他视图），是通用收益。

先做阶段 1、2 再做本阶段的理由：前两者把「不该发生的调用」直接消除，本阶段只是把「重复调用」变
便宜。次序颠倒会掩盖真实的调用次数问题，也会让阶段 1、2 的计数断言难以建立。

**PreviewRenderCache 的污染问题本阶段不解决**——它需要 mmap 按节点复用
`Arc<UiTextLayout>` 才能命中，涉及标题变化时的失效联动，单独列为阶段 6（见下）。

### 涉及文件

`crates/shaping/src/lib.rs`。

### 验收

- 缓存命中与未命中路径返回的 `ShapedRun` 逐字段相等（含 `clusters` 的 `byte_range`、`glyph_id`、
  `font_id`、`advance`、偏移）。
- 字号、weight、style、family 任一变化时不得错误命中——每个维度一个用例。
- 重复 shape 同一 CJK 标题从 38.2 µs 降至亚微秒量级。
- `cargo test -p shaping` 与 `-p textora-markdown` 全绿，确认无渲染回归。

---

## 阶段 4：收敛光标闪烁唤醒（能耗）

### 根因

```1494:1496:crates/markdown/src/mindmap_view.rs
    fn needs_cursor_blink_wakeup(&self) -> bool {
        true
    }
```

该返回值驱动 app 层的唤醒调度（`app/src/app_window.rs:351` 计算下一次 deadline、
`app/src/app_lifecycle.rs:1164` 检测相位变化并置 `needs_redraw`）。返回恒 `true` 意味着只要窗口获
焦且 mmap 是活动标签，就以 2 Hz 唤醒并整帧重绘 + 全屏 Clear。

但 mmap 只在存在 caret 时才画光标——`render_caret` 在没有 caret 时提前返回：

```1161:1166:crates/markdown/src/mmf/canvas.rs
    if !projection.cursor_visible {
        return;
    }
    let Some((node_index, byte_offset)) = projection.caret() else {
        return;
    };
```

因此在「未进入标题编辑态」时，这些帧前后**像素完全相同**，是纯浪费。

### 方案

`needs_cursor_blink_wakeup()` 改为反映真实需求：仅当存在 caret（即 `render_focus()` 为
`TitleEditing`，或存在 IME 组合 caret）时返回 `true`。

判定必须与 `render_caret` 实际绘制条件**同源**，避免两处条件漂移导致光标不闪。建议提取一个私有
方法（如 `fn has_paintable_caret(&self) -> bool`）供两处共用。

注意 `shows_cursor()` 应保持 `false` 不变——mmap 自绘光标，不需要外壳叠加。

### 涉及文件

`crates/markdown/src/mindmap_view.rs`。

### 验收

- 未进入编辑态时 `needs_cursor_blink_wakeup()` 为 `false`；进入标题编辑态后为 `true`；IME 组合期
  间为 `true`。
- 进入编辑态后光标仍正常闪烁（回归防护：这是本阶段唯一的用户可感知风险）。
- `cargo test -p textora-app --lib` 全绿（该返回值被 app 层唤醒逻辑消费）。

---

## 阶段 5：IME 预编辑改为局部失效

### 根因

```1323:1329:crates/markdown/src/mindmap_view.rs
            PluginMessage::SetPreedit { text, cursor } => {
                let next_preedit = (!text.is_empty()).then_some((text, cursor));
                if self.preedit != next_preedit {
                    self.preedit = next_preedit;
                    self.clear_layout();
                }
                true
            }
```

而 `clear_layout()` 清空 layout、hit_map、connector mesh 三者（`mindmap_view.rs:155`）。IME 组合期
间每次按键都触发一轮全量重建：拼音输入「你好」（n-i-h-a-o 加提交）约 6 轮，500 节点下约 200 ms。
中文是本产品的主要使用场景，此项实际影响大于英文路径。

`clear_layout_for_preedit`（`mindmap_view.rs:253`）在选区/光标变化时也会走同一条全清路径。

### 方案

本阶段**依赖阶段 1、2 先落地**。两者完成后，全量重建的单次成本已降到亚毫秒量级，本阶段的收益随
之大幅缩小，因此需要在阶段 2 完成后重新实测，再决定是否值得做。

若仍需优化，方向是让预编辑只影响被编辑节点：预编辑仅改变一个节点的投影标题，唯一的跨节点影响是
该节点所在深度的列宽 max 可能变化。可先判断「新宽度是否改变了该 depth 的 max」，未改变时只更新该
节点的几何、保留其余布局与连接线网格。

connector mesh 是否必须一并失效需单独确认：若列宽 max 未变，节点位置不变，网格理论上可完整保留。

### 涉及文件

`crates/markdown/src/mindmap_view.rs`、`crates/markdown/src/mmf/layout.rs`。

### 验收

- 先出实测数据：阶段 2 完成后测量 500 / 1000 节点下 IME 组合每次按键的实际耗时，据此决定取舍。
- 若实施：局部失效后的 `LayoutTree`、`HitMap` 与「同一状态全量重建」的结果逐字段相等，覆盖列宽
  max 变化与不变两种分支。
- 现有 IME 用例（预编辑投影、候选窗定位、提交）全部保持通过。

---

## 阶段 6：mmap 复用 `Arc<UiTextLayout>`（可选）

阶段 3 完成后，每帧重复 shaping 已经变便宜，但 `PreviewRenderCache` 仍然永不命中且持续污染 LRU。
彻底修复需要 mmap 按节点缓存 `Arc<UiTextLayout>` 并在标题变化时失效，使 `layout.id` 跨帧稳定，
从而复用已缓存的 `GlyphInstance`。

收益是每帧省掉可见节点的顶点重建，并停止挤占其他视图的缓存容量；代价是又一处需要正确联动失效的
缓存。**建议在阶段 3 完成后实测该缓存的实际命中率与 LRU 压力，再决定是否实施**，不要提前投入。

### 涉及文件

`crates/markdown/src/mmf/canvas.rs`、`crates/markdown/src/mindmap_view.rs`。

---

## 基准与回归防护

仓库目前没有 mmap 编辑路径的基准，本次数据来自已删除的临时探针。若不沉淀，这些优化会在后续迭代
中静默回退。

- 新增 `crates/markdown/benches/mmap_editing_perf.rs`，覆盖节点数扫描（50 / 200 / 1000 / 5000）下
  的单键成本，分别记录 `compute_layout` 与 `build_hit_map`。
- `crates/markdown/Cargo.toml` 增加 `criterion` dev-dependency 与 `[[bench]]` 段，
  `harness = false`。
- 复杂度类断言优先用 **shape 调用计数**而非墙钟耗时。本方案的每个阶段都能表述为「shape 次数从 N
  降到 K」，计数断言直接锁住复杂度，且在 CI 上稳定。耗时断言只用于量级差异极大的场景并留足噪声
  余量。

## 验证

每个阶段执行：

```bash
cargo fmt --all
cargo test -p textora-markdown --lib
```

阶段 3 追加 `cargo test -p shaping`；阶段 4 追加 `cargo test -p textora-app --lib`。

全部完成后执行：

```bash
cargo clippy --workspace --all-targets -- -D warnings
./scripts/verify.sh
```

## 附录：本轮未纳入的项

审计中确认存在、但不属于阶段 1 至 6 范围的开销，记录备查：

- **`parse` 全量重建**：每次 `UpdateSource` 都全量 `mmf::parser::parse`
  （`mindmap_view.rs:1274`），并持有一份完整源码副本。实测 1000 节点仅 112 µs，占比 0.16%，**当前
  不值得优化**。仅当文档规模再上一个量级时重新评估。
- **`collect_nodes_dfs` 重复调用**：`mmf/utils.rs:4` 每次分配新 `Vec`，在单次编辑请求内被调用多次
  （编辑规划 2 次、`preedit_projection` 多次、渲染 1 次）。是常数因子放大而非复杂度问题，收益远
  小于阶段 1、2，可在阶段 2 顺手合并调用点。
- **连接线全表扫描**：`render_connectors`（`canvas.rs:608`）遍历全部 layout 节点做 Y 轴相交过滤，
  而非按可见卡片索引。因为长连接线可跨视口，不能直接改用 `visible_node_indices`。mesh 已按 zoom
  缓存，每帧只是遍历加包围盒判断，成本远低于 shaping。
- **GPU 无脏矩形**：每帧对整个 swapchain `LoadOp::Clear` 并全量上传顶点。pipeline、atlas 纹理、
  bind group 均复用，vertex buffer 仅在容量不足时扩容，单帧 GPU 成本本身不高。真正的问题是帧数，
  已由阶段 4 处理。
- **`PowerPreference::HighPerformance`**（`appkit-shell/src/gpu.rs:190`）：双 GPU Mac 上可能选中独
  显。与 Markdown 方案的附录重复记录，需实测后统一决定，不在本方案内单独处理。
