# WYSIWYG 双向源码—视觉投影层设计

## 状态

- 日期：2026-07-11
- 范围：`textora-markdown` WYSIWYG 编辑视图
- 决策：采用独立的双向源码—视觉投影层，替代布局末端的源码 byte 猜测
- 关系：本设计扩展并取代 `2026-07-11-wysiwyg-source-grapheme-mapping-design.md` 的映射架构部分；样式化文字 advance 仍由 `2026-07-11-wysiwyg-styled-advance-fix.md` 独立约束

## 背景

Markdown WYSIWYG 同时存在三个坐标空间：

1. 源码 byte：文档、事务、Undo、selection 的持久坐标。
2. 逻辑视觉 grapheme：Markdown 标记折叠或展开后，用户可以停靠的文本边界。
3. 布局坐标：逻辑视觉文本经过软折行后形成的 visual line、grapheme 和像素位置。

当前实现没有一个组件完整表达三者之间的关系。布局阶段有时携带精确的
`source_bytes_by_visual_grapheme`，有时在 `build_flat_lines()` 中使用“源码行起点 +
视觉文本长度 + marker 长度”重新推导。两种路径会出现在同一个逻辑行的不同软折行
分段中。

`promotion.md` 的连续引用行稳定暴露了该问题：第一视觉分段因活动 `> ` marker 获得
显式映射，后续分段走回退映射；显式分段提前 `continue`，没有推进回退累计偏移，导致
后续分段再次从前一源码位置开始。重复的 source byte 随后被反向查询的视觉亲和性规则
解析到错误视觉行，鼠标点击、方向键和光标绘制共同表现为跳行与循环。

即使只修复累计偏移，连续引用源码中的 `\n> `、嵌套结构标记、表格单元格分隔符和
折叠内联标记仍然不是连续的一对一文本，线性推导无法成为长期正确的基础。

## 已确认和同类风险

| 场景 | 风险 | 原因 |
|---|---|---|
| 连续 blockquote 物理行 | 已确认缺陷 | softbreak 折叠 `\n> `，且显式/回退映射混用 |
| 单个长 blockquote 软折行 | 已确认同构风险 | 首段展开 marker，后续分段仍可能回退 |
| 嵌套 blockquote | 高风险 | 每层 continuation marker 都形成隐藏源码区间 |
| 长 heading 激活后软折行 | 高风险 | 首段 marker 显式映射与后续回退映射同构 |
| 跨物理行且含内联样式的引用 | 高风险 | style span 精确，但 span 间普通文本仍可能线性推导 |
| 多行或嵌套 list item | 中风险 | 活动路径已有部分逐行映射，但不同布局状态来源不统一 |
| Markdown table | 高风险 | 多个 cell flatten 后可能共享父 block 的逻辑行身份和回退起点 |
| code block | 低风险 | 每个源码内容行已有独立起点映射，不参与普通软折行 |
| metadata / horizontal rule | 低到中风险 | 当前结构简单，但仍应服从统一投影不变量 |
| IME preedit | 中风险 | 视觉字符没有源码宽度，需要显式的虚拟投影语义 |

风险等级表示当前架构下产生错误映射的可能性，不代表所有场景都已由用户复现。

## 目标

- 为每个可编辑视觉 grapheme 边界保存可验证的绝对源码 byte 投影。
- 为每个可停靠源码 byte 提供确定的视觉位置和明确的 cursor affinity。
- 正确表达直接文本、隐藏标记、折叠 softbreak、展开标记和 IME 虚拟文本。
- wrapping 只切分已有投影，不再重建或猜测源码位置。
- 点击、光标绘制、水平/垂直导航、selection、IME 和拖选共用一个投影索引。
- 全量布局、视口布局和 cursor-only 增量布局对同一 generation 产生等价投影。
- 在不改变 Markdown 渲染样式和编辑语义的前提下，消除同类 cursor mapping 缺陷。

## 非目标

- 不重写 `pulldown-cmark` 或 Markdown AST。
- 不改变 Enter、Backspace、Indent、Outdent 等结构编辑策略。
- 不改变普通文本编辑器、Novel View 或 Mindmap View 的坐标模型。
- 不在本阶段实现多光标、协同编辑或跨 generation 的位置变换。
- 不以增加点击后第三次重试、nearest-neighbor 扫描或特殊结构补丁作为修复手段。

## 核心不变量

### 1. 坐标单位

- 源码持久坐标始终是 UTF-8 byte boundary。
- 视觉停靠单位始终是 Unicode extended grapheme cluster boundary。
- 像素 advance 只负责 grapheme 与 x 的转换，不参与源码 byte 推导。

### 2. 投影完备性

- 每个可编辑 `LaidOutLine` 必须携带完整投影。
- 视觉边界数量必须等于该行 grapheme 数量加一。
- 每个投影到源码的 byte 必须处于文档范围内并位于 UTF-8 char boundary。
- 直接文本映射必须落在源码 grapheme boundary；结构 marker 可落在合法 char boundary。
- 缺失投影是布局错误，不允许静默回退到文本长度计算。

### 3. 单调性与跳跃

- 单个 visual line 内的源码 byte 投影必须单调不减。
- 投影允许向前跳跃，用于跨过隐藏的换行、blockquote marker、list continuation marker
  和内联 marker。
- 投影不得回退；跨 visual line 的阅读顺序也不得产生倒序源码位置。

### 4. 边界唯一性与亲和性

- soft-wrap 交界处允许“上一行末尾”和“下一行开头”共享同一个 source byte。
- 合法重复必须显式携带 `Upstream` 或 `Downstream` affinity，不能通过
  `grapheme_pos == 0` 猜测。
- 普通源码 byte 的反向投影必须返回唯一的 canonical visual position。
- 位于折叠源码区间内部的 byte 按操作方向和 affinity 吸附到区间入口或出口。

### 5. generation 一致性

- 投影必须绑定 document source generation 和 layout revision。
- query 不得混用旧 generation 的反向索引与新 generation 的 flat lines。
- cursor-only 重排只能替换受影响 block 的投影片段，并在发布前重新验证全局顺序。

## 数据模型

### SourceAnchor

```rust
pub(crate) struct SourceAnchor {
    pub byte: usize,
    pub affinity: CursorAffinity,
}

pub(crate) enum CursorAffinity {
    Upstream,
    Downstream,
}
```

`SourceAnchor` 表示一个视觉边界应落到哪个源码插入点，以及同一 byte 同时位于两个视觉
分段边界时选择哪一侧。

### ProjectionSpan

```rust
pub(crate) struct ProjectionSpan {
    pub source_range: std::ops::Range<usize>,
    pub visual_range: std::ops::Range<usize>,
    pub kind: ProjectionSpanKind,
}

pub(crate) enum ProjectionSpanKind {
    Direct,
    Collapsed,
    Virtual { anchor_byte: usize },
}
```

- `Direct`：源码内容或当前已展开 marker 与视觉 grapheme 直接对应。
- `Collapsed`：一个源码区间折叠为零个或一个视觉 grapheme，例如隐藏 marker 或
  softbreak 空格。span 必须同时定义入口和出口 anchor。
- `Virtual`：IME preedit 等只存在于当前帧的视觉文本，所有边界锚定同一 source byte。

`ProjectionSpan` 是构建和验证用的语义表示，不要求最终热路径逐 span 查询。

### ProjectedText

```rust
pub(crate) struct ProjectedText {
    pub text: String,
    pub spans: Vec<ProjectionSpan>,
    pub boundaries: Vec<SourceAnchor>,
}
```

`boundaries` 是按 grapheme 压缩后的热路径表。`spans` 保留折叠语义，用于反向索引、
debug 验证和增量更新。构建完成后必须满足核心不变量。

### VisualLineProjection

```rust
pub(crate) struct VisualLineProjection {
    pub flat_line_idx: usize,
    pub boundaries: Vec<SourceAnchor>,
    pub source_extent: std::ops::Range<usize>,
}
```

wrapping 只能按 `ProjectedText.text` 的 grapheme boundary 切片，并同步切片
`boundaries`。`source_extent` 用于视口查找、空行邻接和增量失效，不假设其中每个 byte
都可见。

### SourceProjectionIndex

```rust
pub(crate) struct SourceProjectionIndex {
    pub generation: u64,
    pub layout_revision: u64,
    pub visual_lines: Vec<VisualLineProjection>,
    pub reverse: Vec<SourceVisualAnchor>,
    pub collapsed: Vec<CollapsedSourceRange>,
}
```

`reverse` 按 `(source_byte, affinity)` 排序，提供 `O(log n)` 的 source byte → visual
position 查询；`visual_lines` 提供 `O(1)` 的 flat line + grapheme → source anchor 查询；
`collapsed` 为隐藏区间提供方向敏感的吸附规则。

## 架构边界

建议新增 `crates/markdown/src/projection.rs`，只依赖 Markdown parser/builder 的纯数据和
Unicode grapheme 工具，不依赖 `app::DocumentView`、窗口事件或渲染状态。

职责划分：

- `parser.rs`：继续提供事件及其精确源码 range。
- `builder.rs`：按事件顺序构造逻辑文本及投影语义，不能丢弃 softbreak 的源码 range。
- `projection.rs`：定义投影类型、构建器、验证器、wrapping 切片和双向索引。
- `edit.rs`：把 marker 展开/折叠和 preedit 表达为投影变换，不再判断 map 是相对还是绝对。
- `layout/block.rs`：使用 `ProjectedText` 做 shaping/wrapping，将投影切片写入每个
  `LaidOutLine`。
- `layout/types.rs`：flatten 已完成的视觉行投影并构建统一索引；删除可编辑文本的
  fallback source-map 推导。
- `view.rs`：点击、cursor rect、导航、selection 和 IME 只调用投影索引 API。
- `app`：继续只传递绝对源码 byte 和像素坐标，不理解 Markdown 投影内部结构。

这保持了现有分层红线：纯映射数据和算法位于 `textora-markdown`，不会让 UI 层依赖
app 状态。

## 数据流

### 1. Parser 到逻辑投影

builder 消费每个 `MarkdownEvent` 及其 source range：

- `Text` / `Code`：产生 `Direct` span。
- `SoftBreak`：视觉上产生一个空格；source range 从前一内容末尾跨到下一内容起点，
  产生具有不同入口、出口 anchor 的 `Collapsed` span。
- 隐藏 inline marker：产生零视觉宽度的 `Collapsed` span。
- 当前活动 marker：产生 `Direct` span。
- 图片等替代视觉内容：必须显式选择 `Collapsed` 或 `Virtual` 语义，不能遗漏映射。

连续 blockquote 因此会把视觉空格前的 anchor 指向 line3 末尾，把空格后的 anchor 指向
line4 内容起点，明确跳过 `\n> `。嵌套引用同理跳过所有 continuation markers。

### 2. 活动结构与内联 materialization

marker 展开不再在已经布局的第一行上临时拼接字符串和猜测相对偏移。materialization
在 wrapping 之前操作 `ProjectedText`：

- 展开 marker：用 marker 的真实 source range 插入 `Direct` span。
- 折叠 marker：把同一 source range 改为 `Collapsed` span。
- preedit：在 cursor anchor 处插入 `Virtual` span。

同一逻辑文本无论是否含样式，都必须经过同一投影构造路径。

### 3. Wrapping

wrapping 输出逻辑文本的 grapheme 边界区间，而不是只输出字符串 byte 区间。每个视觉
分段从 `ProjectedText.boundaries` 切出 `grapheme_count + 1` 个 anchor。

soft-wrap 共享边界在上一分段记为 `Upstream`，在下一分段记为 `Downstream`。反向索引
默认选择 `Downstream`，Home/End 和显式向左移动可请求 `Upstream`。

### 4. Flatten 与索引发布

`build_flat_lines()` 只执行：

1. 按视觉阅读顺序收集 `LaidOutLine`。
2. 校验每行 projection。
3. 构建 `SourceProjectionIndex`。
4. 校验全局 source 顺序、generation 和合法重复边界。
5. 原子替换引擎当前索引。

旧的 `line_byte_offsets`、`fallback_line_byte_offsets`、marker overhead 累加和
nearest-neighbor source-line 扫描不能再为可编辑文本生成映射。空源码行继续由
`SourceLineMap` 提供几何，但最终也转换为零 grapheme 的 `VisualLineProjection`，使消费
端不再维护第二套导航规则。

### 5. 消费端

- Hit-test：pixel x → visual grapheme → `SourceAnchor`。
- Cursor rect：source byte + affinity → canonical visual position → pixel x/y。
- Left/Right：在投影边界序列移动，不对 source byte 做算术。
- Up/Down：选择相邻 visual line，再按 sticky x 命中其投影边界。
- Selection：两个 source anchor 分别投影，按视觉顺序生成高亮区间。
- IME：使用 cursor anchor 的 `Virtual` span 计算 preedit 光标，不制造伪 source byte。

## 表格处理

表格不能继续依靠父 `TableWrapper` 的单一 `block_line_base` 推导所有 cell。每个 cell
必须拥有稳定的 `ProjectionOwnerId`，至少包含 table block、row、column 和 cell 内逻辑行
索引。

布局 cell 时直接使用该 cell 的 `ProjectedText`；flatten 只保留视觉顺序，反向索引仍按
真实 source byte 排序。分隔符 `|`、对齐行和单元格 padding 分别表达为 collapsed source
range 或纯布局几何，不能混入 cell 内容的线性偏移。

## 空行和结构间距

空源码行没有文本 grapheme，但仍是合法插入点。`SourceLineMap` 继续负责判定
`EditableEmpty` 与 `HiddenBlockSeparator`，随后生成：

- 可编辑空行：带唯一 source anchor 的零 grapheme visual line。
- 隐藏块间隔：collapsed source range，不产生可点击 visual line。

Left/Right/Up/Down 通过投影索引跨越空行，不再在 `view.rs` 中分别调用
`previous_non_empty_source_line`、`next_editable_empty_source_line` 等旁路函数。迁移完成前
这些函数保留为兼容层，最终删除。

## 错误处理

新增内部 `ProjectionError`，至少区分：

- `BoundaryCountMismatch`
- `InvalidSourceBoundary`
- `NonMonotonicSourceOrder`
- `UnclassifiedDuplicateBoundary`
- `StaleGeneration`
- `MissingEditableProjection`

开发和测试构建必须让投影错误直接导致测试失败。生产构建不得 panic 或返回猜测 byte：

- 当前 query 返回 `None`，App 保持原 cursor/selection 不变。
- 渲染可以继续使用已经生成的文字几何，但该行暂时不可点击编辑。
- 保留结构化诊断信息，包含 generation、block owner、flat line 和错误种类。
- 不得回退到 buffer 末尾或 byte 0，避免一次映射错误演变为破坏性编辑。

## 增量布局与缓存

- `ProjectedText` 按 source generation 和 block owner 缓存。
- cursor 进入或离开 marker 只重建含该 marker 的逻辑投影及其视觉分段。
- viewport culling 可以丢弃 shape 和像素几何，但不能让同一 generation 的全局反向索引
  指向已不存在的 visual line。
- 两种允许策略：保留轻量 projection、只逐出 shape；或重建视口 projection 后发布带
  viewport scope 的索引。实现计划必须选择前者，避免导航到视口外时失去源码归属。
- 投影构建复杂度为可见/变更文本 grapheme 数量的 `O(n)`；反向查询为 `O(log n)`；
  visual → source 查询为 `O(1)`。

## 测试策略

### 1. 纯投影单元测试

覆盖：

- ASCII、CJK、组合字符和 ZWJ emoji。
- 普通 softbreak、hardbreak 和 CRLF 归一化输入。
- `\n> `、嵌套 `\n> > `、list continuation marker。
- 隐藏/展开粗体、链接、行内代码 marker。
- soft-wrap 共享边界的 upstream/downstream affinity。
- preedit virtual span。
- projection validation 的每一种错误。

### 2. 布局集成测试

对 heading、paragraph、blockquote、nested blockquote、list、nested list、table cell、code、
metadata 和 empty line，至少验证：

- 每个 `LaidOutLine` 的边界数量正确。
- 所有 source byte 合法且行内单调。
- cursor 激活前后，未受影响文本的 source 投影不变。
- 全量布局和 viewport/cursor-only 布局产生相同投影。

### 3. 非自指向交互测试

测试不能先从被测 source map 获取预期点击位置。应从真实视觉内容构造 oracle：

- 在 flat line 中按显示文本找到目标词，使用 shaping advance 计算点击 x，使用该行真实 rect
  计算 y，再断言返回预先写明的 source byte range。
- 断言 line3 文本所在 rect 的点击不会落入 line4 source range。
- 从每个视觉 grapheme 边界执行 byte → rect → hit-test roundtrip。
- 对整行连续执行 Left/Right，断言 source anchor 按 affinity 有序且最终可终止，不循环。
- 对相邻视觉行执行 Up/Down，断言保持 sticky x 且 source line 归属正确。

### 4. 真实回归夹具

使用 `promotion.md` 的最小匿名化片段作为固定 fixture，并覆盖至少两种 viewport 宽度。
另外增加：

- 超长 H1/H2 激活 marker 后折为三行。
- 两层 blockquote，第二物理行含粗体和 `—`。
- 多行 list item 与嵌套 list。
- 两行三列表格，每个 cell 都会软折行。
- 空行位于引用、列表和普通段落之间。

### 5. 确定性遍历测试

不新增 fuzz 依赖。对固定 Markdown corpus、多个 viewport 宽度和所有合法 source grapheme
boundary 遍历：

1. 设置 cursor byte。
2. 构建投影与布局。
3. 查询 cursor rect。
4. 在 rect 中心执行 hit-test。
5. 验证 roundtrip、合法边界和无循环导航。

## 分阶段迁移

### 阶段 1：投影类型与验证器

新增纯数据投影模块和不变量测试，不接入生产消费端。建立结构化错误类型和确定性 corpus。

### 阶段 2：Parser/Builder 保留源码关系

让逻辑文本在构建时保存 text event、softbreak 和隐藏 marker 的真实 source ranges。先覆盖
普通段落、heading 和 blockquote。

### 阶段 3：统一 materialization 与 wrapping

marker 展开、内联样式和 preedit 改为投影变换；所有 wrapped lines 携带显式投影。完成后
禁止 blockquote/heading 使用回退映射。

### 阶段 4：双向索引与消费端切换

构建 `SourceProjectionIndex`，依次切换 cursor rect、hit-test、Left/Right、Up/Down、
selection 和 IME。每切换一个消费者就删除对应的局部 byte 推导。

### 阶段 5：列表、表格与空行统一

为 list continuation、table cell owner 和 empty visual line 接入投影层，移除结构专用的
导航旁路。

### 阶段 6：删除 legacy fallback

删除 `fallback_line_byte_offsets`、可编辑文本的 `line_byte_offsets` 推导、marker overhead
猜测、relative/absolute map 启发式和重复 byte 的 `grapheme_pos == 0` 选择规则。

### 阶段 7：完整验证与性能检查

执行定向测试、crate 全测、工作区编译和 `./scripts/verify.sh`。记录长文档首次布局、
cursor-only 更新、hit-test 和反向查询的基线，确认没有引入全篇逐光标重建。

每个阶段必须限制在最多三个生产文件；超过三个文件时拆成独立子任务。每个子任务先写
失败测试，再实现单一变更，并在提交前通过编译。

## 验收标准

- `promotion.md` line3 可通过鼠标、Up/Down 和 Left/Right 到达，不跳到 line4/line5，且
  水平遍历不会循环。
- 长 heading、单层/嵌套 blockquote、list 和 table cell 在窄/宽 viewport 下均可准确
  点击和导航。
- 所有 cursor query 只返回合法 UTF-8/grapheme 边界。
- source → visual → source 在 canonical affinity 下精确 roundtrip。
- 任何可编辑 flat line 都不存在缺失 projection 或 legacy fallback。
- cursor 激活 marker 前后，未受影响 visual lines 的几何与投影保持稳定。
- 全量布局与增量布局的投影结果一致。
- Markdown crate 全测、App 相关测试、`cargo fmt --all -- --check`、工作区编译和
  `./scripts/verify.sh` 全部通过。

## 被否决方案

### 仅修正累计偏移

只能修复显式分段后没有推进 fallback offset 的直接缺陷，无法表达 `\n> `、嵌套 marker
或 table cell 等不连续源码关系。

### 为每种 Markdown 结构增加 marker overhead

会把 parser 语义复制到 layout fallback 中。结构组合、内联标记和未来 Markdown 扩展会
持续产生新的遗漏。

### 保留 nearest-neighbor 并增加搜索范围

它只能让错误映射更容易得到某个结果，不能证明结果属于正确视觉行，并可能把一次点击
吸附到相邻空行或结构边界。

## 后续文档

本设计经审阅确认后，单独编写 implementation plan。计划必须给出精确文件、接口、失败
测试、验证命令和阶段提交点，不得在同一任务中同时迁移超过三个生产文件。
