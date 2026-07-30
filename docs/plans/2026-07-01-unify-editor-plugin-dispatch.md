# 统一编辑视图 Plugin 分派 —— 消除 WYSIWYG 双路径

- 日期:2026-07-01
- 范围:`crates/app`(dispatch/renderer/window)、`crates/ui::plugin`、`crates/app/src/plugins/editor.rs`、`crates/markdown/src/view.rs`
- 目标:让"基础编辑视图"和"Markdown 编辑视图"在 App 层走同一套 `ViewPlugin` 协议,取消现存基于 `is_wysiwyg()` 的双路径分支。

---

## 一、背景与现状

### 编辑权威已经统一
两种编辑视图的**文本真值都在 `DocumentView`(`TextBuffer`)** ——
- 基础编辑视图:App 直接调用 `DocumentView::{insert_at_cursor, delete_*, undo/redo, extend_selection_*}` 等方法修改 rope。
- Markdown 编辑视图 (`MarkdownEditorView`):**不拥有 `TextBuffer`**,只做渲染 + 命中测试 + 视觉光标 + 编辑增强建议。App 修改 `DocumentView` 之后,通过 `PluginMessage::UpdateSource` 回灌一份 `String` 快照给 plugin。

因此本次改造 **不涉及数据模型合并**,只针对 App 层与 plugin 之间的分派协议。

### 现存双路径(需要消除)

以下位置存在 "if plugin.is_wysiwyg() {…} else {…}" 或"WYSIWYG 走 plugin,基础编辑器直读 `DocumentView`"两条路径:

| 位置 | 现状 |
|---|---|
| `app_renderer.rs:479, 521, 994` | 帧渲染时按 `is_wysiwyg()` 走不同 sync 流程 |
| `app_renderer.rs:987-1060` `sync_wysiwyg_plugin_state` | 仅服务 WYSIWYG,基础编辑器不进 |
| `app_window.rs:138, 168` | IME 光标定位:WYSIWYG 走 `CursorScreenPos` query,基础编辑器读 `DocumentView` |
| `events.rs:73, 235-236` + `app_dispatch.rs:113-117` | Enter/Backspace/Tab 前置协商:WYSIWYG 走 `PluginInterceptKey` / `AugmentEdit`,基础编辑器直接进 `DocumentView::insert_at_cursor` |
| `app_renderer.rs` 鼠标 hit-test | WYSIWYG 走 `HitTestByte` query,基础编辑器走 `render_pipeline`/`display` |
| `app_renderer.rs` 上下键视觉运动 | WYSIWYG 走 `VisualMoveWysiwyg` query,基础编辑器走 `cursor_move_*` |

### 基础编辑器 plugin 现状
`crates/app/src/plugins/editor.rs` 目前是空 stub,`render()` 返回空 DrawList。所有基础视图的实际渲染仍走 `app_renderer` + `render_pipeline` 的老路径。

---

## 二、总体思路

**关键判断**:两个 View 的差异表面上是"markdown 是 WYSIWYG,基础编辑器不是",实际上是"markdown 的**几何和视觉映射**由 plugin 内部的 `LazyLayout` 决定,而基础编辑器的几何映射一直是 App 内部的 `render_pipeline` / `display` 拥有,没暴露成协议"。

因此本次不是"把基础编辑器塞进 WYSIWYG 模型",而是**把 WYSIWYG 特化的 plugin 协议泛化,让基础编辑器也去实现**。做完之后 App 只面向 `ViewPlugin` 协议编程,不再问"你是谁"。

设计原则:
- **不改数据模型**:`DocumentView` 仍是真值,`MarkdownEditorView` 仍是渲染叠加层。
- **协议名去 "WYSIWYG"化**:`VisualMoveWysiwyg`、`HitTestByte`、`CursorScreenPos`、`AugmentEdit` 均是"视觉↔源码字节映射"的通用能力,与 WYSIWYG 无强绑定。
- **默认 no-op**:基础编辑器 plugin 对新协议提供**从 `DocumentView`/`display` 派生的默认实现**;markdown 保持现有实现。
- **`is_wysiwyg()` 逐步降级**:阶段末尾该方法仅剩少数强绑定"块结构编辑"的地方保留;分支消除后可考虑彻底删除。

---

## 三、分阶段方案

每一阶段为独立 PR,自成闭环,可独立回归。

### 阶段 1 —— 通用化 query,让 `EditorPlugin` 承接几何映射

**目标**:让 `plugins/editor.rs` 实现三个 query,取消 App 层"走 plugin 还是走 doc"分支。

**改动**
1. `ui::plugin`:重命名 `PluginQuery::VisualMoveWysiwyg` → `VisualMove`。字段保持不变(名字上去掉 WYSIWYG 内涵)。
2. `crates/app/src/plugins/editor.rs`:实现
   - `PluginQuery::HitTestByte { x, y, offset_x, offset_y }` → 委托 `DocumentView::display` + `render_pipeline` 现有 hit-test。
   - `PluginQuery::CursorScreenPos(byte)` → 委托 `cursor_motion` + 视口计算。
   - `PluginQuery::VisualMove { current_byte, direction, target_x }` → 委托 `DocumentView::cursor_move_*` 系列。
   > 因为 `DocumentView` 状态在 `doc: &dyn DocView` 里,plugin 通过 downcast 或者由 App 端在调用前用一个新增的辅助 trait `EditorHost` 暴露必要信息。参考 `crates/app/src/editor_host.rs` 现有做法。
3. `crates/app/src/app_renderer.rs`、`app_window.rs`:所有原先 `if is_wysiwyg { plugin.query(…) } else { doc.compute_…() }` 分支改为**无条件** `plugin.query(…)`。

**验收**
- `cargo test -p textora-app` 全绿。
- 基础编辑器视图鼠标点击 / 上下键 / IME 光标定位行为像素级一致(以现有 `app_tests.rs` 中的相关用例为准)。
- `grep -n "is_wysiwyg" crates/app/src` 计数下降至少 4 处。

**风险**
- 基础编辑器 plugin 需要读到 `DocumentView` 的 `display` / `render_cache` — 现有 `DocView` trait 无此能力。方案:新增 `EditorHost::view_geometry(&self) -> &ViewGeometry`,由 `DocumentView` 实现;plugin 通过 `doc.as_editor_host()` 拿到。
- 若 downcast/host 路径不接受,可退化为:在 App 一侧算好几何数据,通过 `PluginQuery` 的入参传进去(query 变胖但避免 trait 扩展)。

---

### 阶段 2 —— 通用化状态同步,取消 `sync_wysiwyg_plugin_state` 特化

**目标**:让所有 plugin 每帧接收同一组 sync 消息;不消费的 plugin 直接 no-op 返回。

**改动**
1. `app_renderer.rs`:把 `sync_wysiwyg_plugin_state` 改名为 `sync_plugin_state`,**移除** `if !plugin.is_wysiwyg() return`。
2. `plugins/editor.rs`:对 `UpdateSource / SetSelAnchorByte / SetSelCursorByte / SetCursorByte / SetPreedit / SetCursorVisible` 六个 `PluginMessage` 全部 no-op 返回 false(基础编辑器不需要外部灌数据 — doc 就是它的数据)。
3. `MarkdownEditorView`:行为不变,继续消费全部消息。
4. `app_renderer.rs` 的帧渲染入口:`if plugin.handles_own_rendering() { plugin.render(…) } else { legacy_path(…) }` 分支保留 —— 阶段 2 不动渲染。仅 sync 通道统一。

**验收**
- 现有 markdown 编辑器所有测试通过。
- 基础编辑器不受影响(消息全 no-op)。
- `sync_wysiwyg_plugin_state` 函数名不再包含 wysiwyg,调用点无条件调用。

**风险**
- 消息数量增多 → 潜在性能开销。评估:六个 `handle_message` 调用每帧一次,分支中最多几次 `if let` + no-op return,数量级 ns,可忽略。

---

### 阶段 3 —— `EditAugmenter`:统一编辑前置协商

**目标**:消除 `PluginInterceptKey` / `AugmentEdit` 的双路径分派逻辑。

**改动**
1. `ui::plugin`:提取 trait
   ```rust
   pub trait EditAugmenter {
       fn augment(&self, ctx: &AugmentContext) -> Option<EditAugmentation>;
   }
   ```
   `ViewPlugin` 提供默认实现 `fn augmenter(&self) -> &dyn EditAugmenter { &NoopAugmenter }`。
2. `MarkdownEditorView`:实现自定义 augmenter,内部逻辑同现在 `PluginQuery::AugmentEdit` 分支。
3. `EditorPlugin`(基础编辑器):使用默认 no-op augmenter。
4. `app_dispatch.rs` / `events.rs`:所有"按下 Enter/Backspace/Tab"路径统一调 `plugin.augmenter().augment(ctx)` 拿到 `EditAugmentation`,再套用到 `DocumentView`。取消 `PluginInterceptKey` action 及其 fallback 分支。

**验收**
- `is_wysiwyg()` 在 `events.rs`、`app_dispatch.rs` 内不再被读取。
- markdown 列表续行、括号配对回归用例通过。
- 基础编辑器 Enter/Backspace/Tab 行为不变。

**风险**
- `PluginMessage::InterceptKey` 从消息通道降级为 trait 方法,需要处理 `KeyCode`、`Modifiers` 类型跨 crate 引用(现在 `ui::core::widget` 已提供)。

---

### 阶段 4(可选)—— 光标 / 选区绘制抽公共函数

**目标**:统一 caret 绘制与选区高亮矩形生成,减少两处重复但**允许 layout 输入不同**。

**改动**
1. `ui::decorations::caret`:新增 `draw_caret(dl: &mut DrawList, rect: Rect, theme, blink_visible: bool)`,markdown/基础编辑器都用它。rect 由各自 layout 计算。
2. `ui::render_geom::line_highlights`:泛化为接受 `LineHighlighter` trait(方法:`line_rect(idx)`、`x_at_col(idx, col)`),基础视图用 `AdvanceCacheEntry` 适配,markdown 用 `FlatLine` 适配。

**验收**
- 视觉像素级一致(可用 `scripts/verify.sh` + 手工 UI 对比)。
- markdown/基础编辑器选区高亮各自的单元测试保持通过。

**风险**
- trait 抽象成本高于收益的可能性 — 若实施时发现 trait 定义臃肿,回退方案:仅共用 caret 绘制,保留两套 highlight 逻辑。

---

## 四、不做的事

明确排除,避免范围蔓延:

- **不合并 `TextBuffer` 与 `LazyLayout`**:数据形态本质不同,合并会写出上帝抽象。
- **不合并"搜索匹配"和"搜索高亮渲染缓存"两个数据结构**:两者职责不同,方案是**命名切分**(见附录 A.1),不是塞进同一结构体。
- **不改 `SelectionState`(markdown)/`selection_anchor`(基础)**:各自数据结构保留,只通过 App 层协议桥接。
- **不动 `HighlighterCache`**:两侧已经共用,无需再抽。

---

## 五、成功标准

阶段 1-3 完成后:

1. `rg -n "is_wysiwyg\(\)" crates/app/src | wc -l` 从当前 8+ 处下降至 0-1 处(仅在 plugin 自身构造/工厂内保留信息性用途)。
2. App 层 dispatch/renderer/window 三大模块中不再有"两个视图分别处理"的分支。
3. 新增第三种视图(如 code editor with LSP、diff view)只需实现 `ViewPlugin` + 可选 `EditAugmenter`,不需要动 App。
4. `./scripts/verify.sh` 全绿。
5. `crates/app/src/plugins/editor.rs` 从 stub 升级为承接几何/协商能力的实体,并被文档标注为 "reference implementation"。

---

## 六、开发建议

- **单阶段单 PR**,严禁把阶段 1-3 塞进同一次改动;每个阶段 diff 控制在 ~500 行以内。
- 阶段 1 开始前,先补一批 App 层的**协议级测试**(mock plugin 只实现最小接口,断言 App 调用顺序/次数),避免重构过程中隐式回归。
- 修改 `PluginQuery` 枚举时(阶段 1 的重命名),同步搜索 `VisualMoveWysiwyg` 全项目,替换 grep 校验。
- `is_wysiwyg()` 每减少一处调用,commit message 里列出具体 file:line,便于回顾。

---

## 附录 A —— 编辑无关的顺手清理(并行推进)

以下三项与本文档的主线(消除 dispatch 双路径)**互不依赖**,可以由任何人在任何时候独立推进,每项一个 PR。收益点是"减少 crate 间重复代码,让未来添加 View 时少踩坑"。

### A.1 `SearchState` 下沉到共享 crate

**问题**
- `crates/app/src/search_state.rs::SearchState`:功能全,含 regex/replace/active_idx/is_stale/next_prev/counter_text,是**逻辑权威**;字段为源码字节 `Range<usize>`,支持 SIMD 匹配。
- `crates/markdown/src/search.rs::SearchState`:结构较小,内部 `RefCell` 缓存高亮 rect,做 char 比较。**只服务于渲染层的高亮**,不是真正的搜索会话。

两者在概念上是"搜索会话"和"搜索渲染缓存",目前名字重合造成误解。

**方案**
1. 把 `app::search_state::SearchState` 升级为**共享搜索会话状态**,搬到 `crates/core/src/search/session.rs`(或保留在 app,视依赖方向而定 —— 若 markdown 需要引用,则必须下沉)。
2. `markdown::search::SearchState` 改名为 `SearchHighlightCache`,只保留"query + rect vec"两个字段,明确职责是"渲染缓存"。
3. `ViewPlugin::query(SearchHighlights{…})` 的入参保持不变(query/options 由 App 传入)。
4. `docs/specs/` 目录下用一句话记录:"搜索的**匹配**归 core,搜索的**高亮渲染缓存**归 View 各自"。

**收益**:命名清晰,未来第三种 View 只需实现"高亮缓存",匹配逻辑不用再抄一遍。

**代价**:约 200 行搬家 + 少量重命名 + 更新 3-5 处 use 路径。

---

### A.2 `word_class` / grapheme 纯函数下沉到 `core::text`

**问题**
- `markdown/src/selection.rs:22-37` 有一个 `CharClass` + `char_class(ch)`,用于视觉侧的双击选词(VS Code 风格,char 粒度)。
- `core/src/buffer/navigation.rs::word_select` 是基础编辑器用的,基于 `ReadableDocument`,字节粒度。
- 两侧词分类的语义**接近但不完全一致**(markdown 侧还没做 UAX#29 grapheme 感知)。

**方案**
1. 在 `crates/core/src/text/word_class.rs`(新建)提供纯函数:
   ```rust
   pub enum CharClass { Word, Whitespace, Punctuation }
   pub fn classify(ch: char) -> CharClass;
   ```
2. `markdown/src/selection.rs` 的 `CharClass` / `char_class` 删除,改用 `core::text::word_class`。
3. `core::buffer::navigation` 内部若使用类似分类,统一到同一实现。
4. Grapheme 相关的**纯函数**(如 `markdown/src/grapheme_map.rs::byte_at_grapheme_index`、`app/src/line_index.rs::count_graphemes_before` 中不依赖具体索引的部分)可以逐步收敛到 `core::text::grapheme`。具体拆迁清单在实施时再列。

**收益**:未来引入完整 UAX#29 word 边界,只改一处。

**代价**:半天,极低风险(纯函数下沉,无 API 变化)。

---

### A.3 `arboard` 剪贴板访问收敛到通用模块

**问题**
- `document_view/selection.rs::copy/cut/paste_from_clipboard` 每个方法都 `arboard::Clipboard::new()` 一次。
- `workspace.rs:952`、`ui_shell.rs:762/768` 又各自 `arboard::Clipboard::new()`。
- 至少 5 处独立调用,错误处理各写各的(全部 silent-fail)。

**方案**
1. 新建 `crates/app/src/clipboard.rs`,提供:
   ```rust
   pub fn read_text() -> Option<String>;
   pub fn write_text(text: &str) -> bool;
   ```
   隐藏 `arboard::Clipboard::new()` 细节,统一错误策略(warn 一次而非 silent-fail)。
2. `DocumentView::copy_selection_to_clipboard` 等方法内部改调 `clipboard::write_text`。
3. `ui_shell.rs`、`workspace.rs` 的直接 `arboard` 引用统一走新模块。
4. **不放到 `core`** —— arboard 是 GUI 应用依赖,`core` 保持无 GUI 依赖。

**收益**:markdown 未来若要独立复制某段渲染文本(不经过 DocumentView),复用同一入口;剪贴板故障可以集中日志。

**代价**:半天,极低风险。

---

### 附录 A 的推进策略

- 三项**互相独立**、也**独立于主线四阶段**,可以任意顺序甚至并行。
- 优先级建议:A.1(收益最大,消除概念混淆)> A.2(收益中等)> A.3(收益最小但最省事)。
- 每一项完成后,主线阶段 1-3 不受影响 —— 因为它们改的是 App 层协议,不是工具函数位置。
