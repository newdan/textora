# WYSIWYG 编辑过程对抗性审查报告（回车 / 退格 / 光标上下移动）

- 日期：2026-08-16
- 范围：Markdown WYSIWYG 模式下的 Enter（回车）、Backspace（退格）、ArrowUp/ArrowDown（垂直光标移动）。**明确排除表格**。
- 方法：三个独立审查线分别完整追踪调用链后做对抗性分析；关键发现已由主审查人对照源码二次核实（文中标注"已核实"）。静态推理但依赖外部行为（pulldown-cmark range、winit IME 事件流、真机绘制时序）的结论标注"存疑/待复现测试"。

## 一、调用链概述（已核实）

生产路径（crates/app，winit 前端）：

1. 键盘事件 → `appkit-shell/src/input_mapper.rs` → `EditCommand::{InsertNewline, Backspace, ...}`。
2. `crates/app/src/dispatch/editor.rs:309-314`：`edit_intent_for_command` 把 InsertNewline/Backspace/InsertChar/InsertText 一律映射为 `EditIntent` 并**提前 return** 进入 `dispatch_transactional_edit`（editor.rs:690）。
3. 事务管线：`sync_plugin_state()` → `build_edit_request` → `MarkdownEditorView::plan_edit`（`crates/markdown/src/view.rs:2607`）→ `augment_edit`（`augmenter.rs`）分类并产出 `EditAugmentation` → `execute_edit_plan`（`edit_transaction.rs:333`，校验 generation / char / grapheme 边界）→ 单条 `EditTransaction` 应用 → 光标落点 → 缓存失效与重投影。
4. 垂直移动：`dispatch_wysiwyg_navigation`（`dispatch/wysiwyg.rs:93-175`）→ `PreviewEngine::visual_move`（view.rs:1676-1789）→ 投影行 ±1 → `grapheme_at_x` 像素换算 → `source_anchor_at` 回映字节；goal column 经 `preferred_x` 记忆。

另有一条 appkit-shell 运行时路径（`editor_runtime/mod.rs`，服务 notora-app），共用同一个 `plan_edit` 分类器，但有自己的 `default_edit_plan`（`model_session.rs:954`）——两条路径存在实现漂移（见发现 5、6、11）。

## 二、发现清单

### 高严重度

#### H1. 代码块 / metadata 块内部空行被误分类为 HiddenBlockSeparator，ArrowDown 卡死、显示光标与插入点分离（已核实分类逻辑，建议补复现测试）

- 证据：`crates/markdown/src/layout/source_line_map.rs:168-179` —— 任何"空 run 第一行 + 前后都有渲染行"的源码空行即被分类为 `HiddenBlockSeparator`，**不判断前后渲染行是否属于同一块**。代码块内部空行（active 状态下每行都有渲染行）必然满足该条件。随后 `add_hidden_separator_collapsed_ranges`（`layout/types.rs:351-376`）把该空行的投影 boundary 改写为上一行末尾字节。
- 触发场景：光标在含空行的代码块内（如 ` ```rust \nfn a() {}\n\nfn b() {}\n``` `），从 `fn a() {}` 行按 ArrowDown：
  1. 第一次 Down 落到被改写的 boundary（上一行行尾字节），视觉上光标没有下移；
  2. 第二次 Down 解析回同一字节，`visual_move` 返回当前字节——**光标永久卡死**，只能用 ArrowRight 逃逸；
  3. 若光标经 Enter 落在该空行上再按 ArrowUp，文档字节落到上一行行尾，此时直接打字会把字符插到上一行末尾而非看到的空行——**显示光标与插入点分离**。
- 预期：代码块是单一块，内部空行是普通可编辑行，垂直移动应逐行经过。
- 根因：`empty_source_line_role` 的分类缺少"前后渲染行分属不同块"这一隐藏分隔的真正判据（`docs/specs/2026-08-02-markdown-wysiwyg-enter-backspace-behavior.md` 的空行不变量一节明确该概念只针对块间空行）。
- 测试缺口：代码块/metadata 块内含空行的垂直移动**零测试覆盖**。

### 中严重度

#### M1. 带选区回车退化为"软换行"，与规范核心承诺冲突（已核实）

- 证据：`crates/markdown/src/view.rs:2608-2610` —— `plan_edit` 在 `request.selection.is_some()` 时直接 `UseDefault`；落到 `edit_transaction.rs:141-142`，默认计划用**单个 `\n`** 替换选区。
- 触发场景：选中段落中一段文字按 Enter → 得到段内软换行，视觉上几乎无变化；跨段落选区按 Enter → 两段并成一段加软换行；列表项内带选区 Enter → 新行无 marker，变 lazy continuation。
- 预期 vs 实际：规范（spec 第 5 行）说"WYSIWYG 把 Enter 解释为新段落"；直觉行为应为"先删选区，再在删除点做块级智能回车"。规范的行为矩阵完全没有定义选区场景，属规范与实现的双重空白。无数据丢失、可撤销，故不定为高。

#### M2. 段首连续退格可破坏代码围栏 / 分隔线 / 缩进代码块等叶块结构（已核实白名单逻辑）

- 证据：`crates/markdown/src/augmenter.rs:313-320` —— `backspace_paragraph_boundary` 的前块白名单只接受 `TopLevelParagraphEnd | ParagraphInterior | Heading`；前块为代码块/分隔线时返回 `None`，回退到 `edit_transaction.rs:160-188` 的单 grapheme 删除，每次只删一个 `\n`，完全没有块边界意识。测试 `augmenter.rs:1209-1220` 只断言返回 `None`，未验证回退后果。
- 触发场景：```` ```\ncode\n```\n\npara ````，光标在 `para` 段首：
  1. 第 1 次退格 → 删一个 `\n`（尚可）；
  2. 第 2 次退格 → ```` ```\ncode\n```para ````——闭合围栏不允许带 info string，` ```para ` 不再是合法闭合围栏 → 代码块延伸到文档末尾，**段落文本被吞进代码块，渲染上"段落消失"**。
  同类：`---\n\npara` 两次退格 → `---para`（分隔线退化）；`    code\n\npara` → 段落被吸进缩进代码块；`> quote\n\npara` → 段落以 lazy continuation 形式静默并入引用（见 M4）。
- 预期：段首退格应跨过块边界安全合并或保持不动；实际是发生在第二次按键时的静默结构破坏。
- 根因：跨块合并是"逐 `\n` 删除"的隐式多步过程，白名单拒绝与 fallback 单字节删除之间没有针对叶块的护栏。

#### M3. Setext 标题被按 ATX 逻辑处理，回车/退格均会静默破坏标题（存疑，依赖 pulldown-cmark range 行为，需先写复现测试）

- 证据：`augmenter.rs:507-523` `classify_heading_hit` 用 `hash_prefix = level + 1` 无条件假设 `# ` 前缀，但 pulldown-cmark 对 Setext 标题（`Title\n===`）同样发 `Tag::Heading` 事件。
- 触发场景（回车）：光标在 `Title` 末尾按 Enter → `at_end` 判定基于整个 range（含 `===` 行）恒为 false → 插 `\n` 得 `Title\n\n===`，**标题样式整个丢失**。中部回车 → 后半变成新 Setext 标题。
- 触发场景（退格）：`Title\n===\n\npara` 段首退格 → M2 的白名单接受"Heading"前块 → 删除换行 run → `Title\n===para`，`===para` 不是合法下划线 → 标题静默消失。
- 说明：规范（spec 第 65 行）声明 Setext 不在范围内，但实现并未**排除** Setext，行为是"静默做错"而非"不支持"。正确做法是在 `classify_heading_hit` 中验证 `source[start..]` 确实以 `#` 开头，否则归 `Other`。
- 测试缺口：全 crate 无 Setext 测试。

#### M4. 段首退格前块为引用 / 列表时，段落被"懒延续"静默并入前块

- 证据：同 M2 的白名单拒绝路径。`> quote\n\npara` 段首退格 → fallback 删一个 `\n` → `> quote\npara`，按 CommonMark 懒延续规则 `para` 成为引用块延续行；再按一次 → `> quotepara`。列表同理（松散列表变紧凑且段落并入列表项）。
- 预期 vs 实际：合并方向接近 Typora，但 Typora 会写入显式 marker（`> `），这里是隐式懒延续——后续 reflow/再解析行为不同，属脆弱中间态。反向（光标在引用/列表内容开头退格，先删 marker 降级为段落）已正确处理，无问题。

#### M5. appkit-shell 运行时的默认回车计划硬编码 `"\n"`，CRLF 文档产生混合行尾（已核实）

- 证据：`crates/appkit-shell/src/editor_runtime/model_session.rs:962` —— `EditIntent::InsertParagraphBreak => Some("\n".to_owned())`。对比 app 侧 `default_newline_text`（`edit_transaction.rs:194-196`）会按 `doc.tb.is_crlf()` 返回 `\r\n`。
- 触发场景：notora-app 打开 CRLF 文档，在代码块内 / `Other` 上下文（HR、HTML 块）/ 带选区时按 Enter → 插入孤立 `\n`，文件行尾混杂。
- 根因：两份 `default_edit_plan` 实现漂移。

#### M6. appkit-shell 路径在 undo/redo 后不刷新插件源码，紧接着回车会基于过期源码计算替换范围（存疑时序窗口）

- 证据：`model_session.rs:424-454` `undo_or_redo_active_document` 只调 `refresh_presentation_after_edit`，**不发送 `PluginMessage::UpdateSource`**；源码同步只在绘制时按需发生（`editor_painter.rs:184`）。而 `edit_active_document` 在 `resolve_edit_plan` 前也不做同步；`plan_edit` 直接用引擎缓存的源码分类，不校验 generation 语义一致性。
- 触发场景：WYSIWYG 下 Cmd+Z 后、下一次重绘前按 Enter → augmenter 用撤销前的旧源码算出 `replace_range`/`cursor_byte_after` 套用到撤销后的文档 → **在错误偏移处插入换行**（文本错乱，可撤销但用户未必察觉）。
- 对比：app 侧 `dispatch_transactional_edit` 规划前先 `sync_plugin_state()`（editor.rs:705），无此问题。
- 存疑点：撤销后正常会触发重绘同步，时序窗口窄；若事件循环保证先绘后键则仅为潜在隐患。根因明确：规划前未建立"插件源码与文档同代"的不变量。

#### M7. 垂直移动后没有任何"保证光标可见"的滚动逻辑，光标可走出视口（存疑，建议真机验证）

- 证据：`dispatch_wysiwyg_navigation`（wysiwyg.rs:93-175）移动光标后只返回 `AppEffect::REDRAW`，不调整滚动；markdown 视图只在收到显式 `PluginMessage::Scroll` 时改 `scroll_y`（view.rs:775-785）；插件绘制路径（`editor_painter.rs:116-177`）无 ensure-visible——对比纯文本路径有 `ensure_cursor_visual_row_visible`（editor_painter.rs:311）。`plugin_scroll_by_command`（app_scroll.rs:130）注释称用于 Arrow keys，但生产代码中无调用方。
- 触发场景：长文档 WYSIWYG 编辑，光标在视口最后一行按 ArrowDown → 光标移到屏外，视图不滚动。
- 存疑说明：此问题若在真机存在应极易被发现，不排除有未追踪到的滚动路径；建议人工真机验证一次。

#### M8. 生产代码中存在整段不可达的 Enter/Backspace 分发路径，且 app 层 Enter 测试全部测的是死路径（已核实）

- 证据：`editor.rs:312-314` 对 InsertNewline/Backspace/InsertChar/InsertText 恒先走事务路径 return（`edit_intent_for_command`，edit_transaction.rs:104-116 已核实全覆盖）；因此 editor.rs:321-336 的 `WysiwygCommandRoute::AugmentedEnter/AugmentedBackspace/AugmentedInsertText` 分支及其实现（`dispatch/wysiwyg.rs:356-476`）、`wysiwyg_recursing` guard（editor.rs:309）在生产全部不可达。`command_should_replace_selection`（editor.rs:48）是无人调用的死函数。`execute_edit_plan` 的 `_advance_cache` 参数（edit_transaction.rs:336）未使用——`dispatch/wysiwyg.rs:1-10` 头注释承诺的 advance_cache 失效机制实际不存在。
- 测试保真度问题：`app_tests.rs:3780/3812/3844` 三个 app 级 Enter 测试调用 `dispatch_wysiwyg_augmented_enter_for_test`，走死路径，**不经过**生产用的 generation 校验、grapheme 校验、选区折叠逻辑。测试变绿不能证明生产行为正确。`dispatch_wysiwyg_augmented_backspace_for_test` 则完全无调用方。
- 附带风险：死路径若被重新启用，它在 augmentation 前不做 `delete_selection()`，会把选区场景 bug 带回来。
- 文档过期：`docs/plans_wysiwyg_enter_fix.md` 仍以旧路径描述行为。

#### M9. WYSIWYG 事务路径 undo 粒度为"每次按键一条目"，与源码模式不一致

- 证据：事务路径所有文本编辑经 `appkit-core/edit.rs:48-50` → `tb.replace_range`（`core/src/buffer/edit.rs:818-830`）固定 `HistoryType::Other`；`edit_begin`（edit.rs:518-519）的合并条件只对 `Write | Delete` 生效。源码模式退格走 `tb.delete(Grapheme, -1)`（`HistoryType::Delete`，连续退格合并为一条）。
- 触发场景：WYSIWYG 下按住退格删 10 个字符，Ctrl+Z 需按 10 次逐字符恢复；源码模式通常 1 次恢复整段连删。
- 影响范围：整个事务路径（含普通输入），不限于退格。

### 低严重度

- **L1. 零宽选区（anchor == cursor）在 app 路径被当作"有选区"，回车退化为软换行；两个运行时行为不一致。** `appkit-core/src/document/model.rs:391` `has_selection()` 只看 `selection_anchor.is_some()`；app 侧 `build_edit_request`（edit_transaction.rs:120-127）因此对 `anchor == cursor` 也产生 `Some(x..x)` → M1 路径。appkit-shell 侧 `edit_request`（model_session.rs:665）有 `.filter(start < end)`，不受影响。
- **L2. IME preedit 期间 Enter/Backspace 被放行，存在双插入/双删除风险（存疑，依赖 winit/macOS 事件流）。** `window_input.rs:77-79` `command_allowed_during_preedit` 只拦 `InsertChar`；`events.rs:82-92` 在 preedit 非空时仍放行 InsertNewline/Backspace，而该文件注释（events.rs:59-62）明确说 macOS + winit 0.30 会同时派发 IME 事件和 KeyboardInput。拼音候选窗激活时按 Enter 确认上屏，可能"提交拼音 + 拆段"同时发生；Backspace 同理可能双重删除。建议用集成测试钉住。
- **L3. ExtendUp/ExtendDown 在文档首/末视觉行不扩展到文档首/尾。** `visual_move_in_projection_sequence` 在首行 Up / 末行 Down 返回 `Some(current_byte)`（view.rs:1762-1773）；Shift+Up 在第一行时光标不动、选区坍缩为空。各平台惯例是扩展到文档起点。
- **L4. `projection_screen_x` 的 `expect` 存在理论 panic 点（view.rs:1756-1759）。** 预览模式懒布局下 `flat_line_idx_for_projection` 可为 None，任何宿主在预览模式查询 `PluginQuery::VisualMove` 且 `target_x=None` 即 panic；当前 app 层只在 WYSIWYG 路由下发该查询，故为低。
- **L5. 空行作为当前行时 goal-x 回退为绝对 0.0，忽略引用/列表缩进上下文**（view.rs:1821）。`empty_source_line_typography` 已有取周边行 `rect.x` 的正确做法可参照。
- **L6. 无空格 checkbox 的任务项续 marker 缺尾随空格**（augmenter.rs:803-806）：`- [x]done` 回车产出 `\n- [ ]`（无尾随空格），不是合法任务项，渲染退化为普通 bullet。正常带空格情形已有测试且正确。
- **L7. 文档开头/空文档退格是无操作，但仍上报 `executed: true` 并推进 content_revision**（edit_transaction.rs:174-186 + appkit-core/edit.rs:53,62-67）。底层空替换不产生垃圾 undo 条目（正确），但每次无效退格触发 REDRAW 与缓存失效，属浪费。
- **L8. 未精确 shaping 行的像素→grapheme 换算用 `font_size * 0.55` 启发式宽度**（layout/context.rs:63,131）。编辑模式仅视口范围 reshape，目标行刚超出 precise range 时落点列有误差，比例字体下偏差明显。
- **L9. 首次垂直移动时 cursor rect 查询失败则 goal column 不播种**（wysiwyg.rs:138-142,169），下一次连续移动从短行落点重新取 x，原始列记忆丢失一次。仅查询失败路径。
- **L10. 列表项之间空行上的 Enter 可能插入多余空 item（存疑）。** augmenter.rs:570-587 的 `End(TagEnd::Item)` 命中条件可能覆盖松散列表的中间空行 → 分类为 ListItem 而非 EmptyBlockSeparatorLine → Enter 插入 `\n- `。需先运行 pulldown 验证 range 行为。
- **L11. preedit 期间垂直移动的当前行判定取 preedit 首 grapheme 位置（存疑/低）。** projection.rs:294-297 剥离虚拟 boundary 后 dedup，preedit 软折行跨两行时行归属可能偏一行。水平方向已有测试，垂直方向未覆盖。
- **L12. 标题行首（`#` 字节处）退格会把字面 `#` 并入上一段落（存疑）。** 该字节位置在 WYSIWYG 投影下可能不可达；若未来开放源码级光标定位则成为真缺陷。

### 信息性（非缺陷，但需处理）

- `dispatch/wysiwyg.rs` 头注释与 `docs/plans_wysiwyg_enter_fix.md` 描述的都是已不可达的旧派发路径，误导维护者（见 M8）。
- `docs/plans/2026-07-03-wysiwyg-cursor-path-convergence.md:480` 要求"移动后查询 snapped_byte 更新 preferred_x"（会导致短行漂移），实现改为保留移动前锚点 x——实现更正确且有测试锁定，但文档未同步。
- `app_lifecycle.rs:627-628` 有重复的 `set_preferred_x(None)` 语句（疑似合并残留）。

## 三、已检查且未发现问题的方面

- **规范行为矩阵逐行核对**（spec 的 8 个 Enter 场景）：段落中部/末尾、软换行前后、ATX 标题中部/末尾等 `emit_block_break` 的 `insertion_count` 逻辑在 LF 与 CRLF 下均正确，光标落点正确，有单测覆盖。
- **空块/文档末尾/空文档回车**：分类与光标落点正确，尾随空行 run 有测试。
- **代码块内回车**：正确回落为普通换行（语义正确）。
- **空列表项/空引用行退出**（`emit_remove_current_line`）：CRLF 场景有测试。
- **退格优先级分派**（空行→段首→单 grapheme 段→marker）：与 spec 一致；marker 行不会被误并入上一块。
- **CRLF 边界删除**：以完整 `\r\n` 为单位，不留孤立 `\r`。
- **字节 vs grapheme**：augmenter 只在 ASCII 边界切分；`execute_edit_plan` 双层校验 char+grapheme 边界，非法 offset 被拒绝而非 panic；未发现现实可 panic 路径。
- **增强操作 undo 原子性**：单条 `EditTransaction` 单次 grouping，一次 undo 完整还原；enter↔backspace 可逆性有参数化测试。
- **app 侧投影/源码时效**：编辑前 `sync_plugin_state`，无陈旧分类窗口（有回归测试 view.rs:3853）。
- **goal column 主路径**：锚点 x 跨移动保留，长→短→长行不漂移；文本编辑/鼠标点击/IME commit/焦点丢失均正确清除，有测试锁定。
- **投影 stale 防护**：generation 变化即作废旧索引，`source_anchor_at` 双重校验，失败静默不动而非错跳。
- **隐藏块分隔（段落间真空行）的上下/左右跳过**：行为与测试一致（H1 的问题在于该逻辑被误用到代码块内部空行）。

## 四、测试覆盖缺口汇总

- 根 `tests/` 只有 golden 图片，对回车/退格/垂直导航**零集成覆盖**；`crates/app/src/dispatch/wysiwyg_test.rs` 是 0 行空文件。
- 回车：带选区 Enter（M1）、Setext（M3）、代码块内 Enter 回落断言、列表项间空行（L10）、无空格 checkbox（L6）、undo 后紧接着 Enter 的源码同步（M6）、appkit-shell CRLF 回车（M5）。
- 退格：段首退格前块为代码围栏/HR/缩进代码块的 fallback 后果（M2）、前块为引用/列表的中间态（M4）、WYSIWYG 带选区退格端到端、undo 粒度合并（M9）、空文档退格 no-op 断言（L7）。
- 垂直移动：代码块内空行（H1，零覆盖）、ExtendUp/Down 文档首尾（L3）、跨字号块落点、引用块缩进 goal-x、穿越 `---`、CRLF/空文档/preedit 场景。
- 端到端断层：app 层用 mock plugin，markdown 层直调 engine；真实 `MarkdownEditorView` + dispatch 的垂直移动/选区同步无集成测试。app 层现有三个 Enter 测试测的是死路径（M8）。

## 六、修复状态（2026-08-17，分支 wysiwyg-edit-fixes）

| 发现 | 修复 commit | 说明 |
|------|-------------|------|
| H1 代码块内空行垂直移动卡死 | `802c789` | 空行自带渲染投影即非隐藏分隔；分类/投影两侧同步修，含 CRLF/metadata 测试 |
| M2 段首退格破坏叶块 | `69ea736` | 不可合并叶块边界退格变为消费型无操作（EditPlan::Consume） |
| M3 Setext 被当 ATX | `32e108f` | `classify_heading_hit` 校验 ATX marker，否则归 Other；含 pulldown 行为探针测试 |
| M4 引用/列表懒延续并入 | `468278e` | 并入行补显式 `> ` / 列表延续缩进，光标落在内容前 |
| M1 带选区回车 | `27ec52d` | 删选区 + 删除点块级增强，单条原子替换 |
| L1 零宽选区 | `27ec52d` | `build_edit_request` 过滤空选区，与 appkit-shell 对齐 |
| M5 appkit-shell CRLF 回车 | `2bd3786` | 默认计划按 `is_crlf()` 返回 `\r\n`/`\n` |
| M6 undo 后源码同步 | `47ab335` | 规划前按需同步 + undo/redo 后即时同步 |
| M7 垂直移动不滚动 | `9c5c6d0` | 复用 `PluginMessage::Scroll`，最小位移、无抖动 |
| M8 死派发路径 | `f12cfb8` | 删除不可达链路（−429 行），3 个 Enter 测试迁到生产事务路径 |
| M9 undo 粒度 | `ea323d8` | `EditPlan::ApplyDefault` 携带 history kind，连续输入/退格合并为单条 undo |
| L2 IME preedit 输入穿透 | 2026-08-19 工作区修复 | 文档修改命令全部阻断，且守卫前移到插件 EditIntent 映射之前 |
| L3 垂直选区首尾 | 2026-08-19 工作区修复 | 首/末视觉行分别返回文档起点/终点，覆盖非边界列 |
| L4 投影坐标 panic | 2026-08-19 工作区修复 | 移除 `expect`，缺失几何时安全返回 `None` |
| L5 空行 goal-x | 2026-08-19 工作区修复 | 复用相邻渲染行 typography x，保留引用/列表缩进 |
| L6 无分隔 task marker | 2026-08-19 工作区修复 | 续项统一补内容空格，保留已有空格或 tab |
| L7 边界删除空事务 | 2026-08-19 工作区修复 | BOF Backspace / EOF Delete 直接 `Consume` |
| L8 未 shaping 行列误差 | 2026-08-19 工作区修复 | 缓存真实字体 advance，未 shaping 行不再把比例字体按统一 `0.55em` 处理 |
| L9 首次 goal-x 未播种 | 2026-08-19 工作区修复 | 移动前无 rect 时，从实际落点几何补播种 |
| L10 松散列表空行 Enter | 2026-08-19 测试排除 | pulldown range 归一化后分类为普通空行，回归测试锁定不插空 item |
| L11 多行 preedit 行归属 | 2026-08-19 测试排除 | 真实视图测试确认 caret 落在虚拟第二行，现有 ordinal 投影正确 |
| L12 ATX marker 起点退格 | 2026-08-19 工作区修复 | marker 起点增加消费型护栏，阻止与上一段拼接 |

至此，本报告列出的 H1、M1–M9、L1–L12 均已修复，或通过针对性回归测试排除旧审查中的存疑场景。

合并前锁定测试：`4659c65`（T2 range 归一化带来的 Enter 分类变化回归锁定）。最终全分支审查结论：With fixes → 该修复已完成。

## 修复优先级建议（原文，已执行完毕）

1. **H1**（代码块内空行垂直移动卡死）——常见内容 + 功能卡死，先写复现测试再修分类判据。
2. **M2 / M3 / M4**（段首退格破坏叶块 / Setext / 懒延续）——同源：为 fallback 加叶块护栏，`classify_heading_hit` 排除 Setext。
3. **M1 / L1**（带选区回车）——规范先行：定义"删选区后走块级智能回车"。
4. **M8**（死路径与假测试）——拆除或重新启用前需明确决策；把 app 层测试迁到事务路径。
5. **M7**（垂直移动不滚动）——真机验证后修复。
6. **M5 / M6**（appkit-shell 路径漂移）——统一两份 default_edit_plan；规划前建立源码同代不变量。
7. **M9**（undo 粒度）——事务路径按 intent 选择 HistoryType。
