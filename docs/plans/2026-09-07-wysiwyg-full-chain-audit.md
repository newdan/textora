# Markdown WYSIWYG 编辑与显示链路审查

日期：2026-09-07。审查版本：`550785d9cf47c6947145c6c92d8640a4ba350ad4`。

后续修复状态：本文 F1–F8 已全部修复，独立复审发现也已关闭；最终 `./scripts/verify.sh` 通过（4812 passed、0 failed、5 ignored）。详见[修复记录](2026-09-07-wysiwyg-full-chain-repair.md)。下文保留修复前版本的审查与复现证据。

## 结论

确认 8 类问题，其中 4 类建议按 P1 优先处理，4 类为 P2。独立复现程序包含 9 个行为断言，全部在审查版本上失败；代码格式化的两个断言归为同一类问题。现有 Markdown 和应用测试通过，说明目前测试集尚未覆盖这些组合。

审查阶段没有修改 Rust 实现、调整现有测试预期或提交代码。新增此审查报告，并将复现代码存入附录，便于后续修复先建立失败回归。

## 审查范围与方法

核查的主链路：

```text
键盘 / 鼠标 / IME / 剪贴板 / 格式命令
  → app dispatch、光标及选区同步
  → Markdown EditPolicy / augmenter / semantic commands / paste writer
  → EditTransaction、范围与字素边界校验、文档历史
  → UpdateSource、源码版本及缓存失效
  → parser → builder → source projection
  → LazyLayout、字形排版、可见行
  → 光标、命中、导航、选区、搜索高亮、IME 绘制
```

对编辑、显示及应用入口分别审查，再由主审使用当前 `MarkdownEditorView` 公共接口与真实 `Shaper` 复核。没有仅凭旧审查文档、预览模式或独立纯函数的结果推定 WYSIWYG 行为。

静态范围包括 Enter / Shift+Enter、Backspace / Delete、选区替换、格式快捷键、富文本粘贴、事务及 Undo / Redo 路径、源码同步、字素映射、空段、容器、表格、行内内容、图片、光标、点击、搜索与 IME。新增实测聚焦以下已确认问题；不是对所有输入组合的穷举验证。

## 已确认问题

### F1 · P1：图片占位文本破坏整份文档的源码投影索引

位置：`crates/markdown/src/builder.rs:847–849`；相关路径 `projection.rs:83–107, 275–283`、`layout/types.rs:226–236`。

最小场景：`abc ![x](y) tail`。扩大场景：`before\n\nabc ![x](y) tail\n\nafter`。

实际：图片行显示为 `abc [Image: y]x tail`，`CursorScreenPos` 对普通文字位置也返回 `None`。扩大场景中，完全不含图片的第一段 `before` 在 byte 0 也没有光标坐标。这不是单纯的图片尚未实现显示，而是破坏了正常文字编辑需要的全局映射。

根因：Image 起始事件把合成的 `[Image: URL]` 通过 `push_text` 当作真实源码文字加入 `Direct` 投影，后续 alt 文本又加入较早的源码偏移，形成倒退的锚点序列。全局 `SourceProjectionIndex::build` 拒绝非单调序列，随后整个索引被置空。光标和导航依赖该索引。

建议：图片使用有明确源码锚点的合成或原子对象投影，不为占位字符串伪造逐字节源码位置。覆盖图片前后独立段落、空 alt、中文 alt、长 URL、多个图片，并断言整份文档的合法文字位置仍可映射。

证据：`image_preserves_unrelated_paragraph_caret_and_navigation` 失败；日志同时记录图片行各位置的空光标结果。

### F2 · P1：行内代码的源码范围包含分隔符，却被当作内容起点

位置：`crates/markdown/src/builder.rs:924–930`；相关路径 `projection.rs:101–103`、`edit.rs:230–260`。

最小场景：源码 ``abc `中文` tail``，光标放在“中”之前，即 byte 5。

实际：激活后可见行是 ``abc 中文` tail``，缺少起始反引号；`CursorScreenPos(5)` 返回 `None`。“文”之前及内容末端的合法源码位置也无法正确映射。ASCII 场景 ``abc `xyz` tail`` 同样漏起始反引号，位置映射偏移。

根因：解析器的 Code 事件范围包含完整分隔符，事件文字已经去掉分隔符。`push_text(code)` 仍把完整事件范围交给 `push_direct`，于是首个代码字符被映射到反引号。中文内容进一步产生处于 UTF-8 字符内部的错误锚点。展开逻辑又用这些边界推导 prefix，得到空前缀。

建议：分别表达完整语法范围、实际内容范围和规范化后的可见内容映射。不能仅固定加 1；还需处理多个反引号、代码首尾空格和跨源码行的规范化。

证据：`inline_code_expands_both_delimiters_and_maps_chinese_caret` 失败；公共接口日志记录了漏标记与空光标坐标。

### F3 · P1：代码中的字面 `<br>` 被一次删除操作整体吞掉

位置：`crates/markdown/src/augmenter.rs:523–538`，前向对应 `172–189`，入口 `118–124`。

复现：围栏代码内容是 `<br>`，光标置于 `>` 后，按一次 Backspace。也可在行内代码 `` `<br>` `` 中执行。

实际：删除整个 `<br>`，围栏代码变成空内容；行内代码变成两个反引号。预期：代码是字面文本，只删去 `>`。静态检查确认 Delete 分支同样仅匹配 `<br>` / `<br/>` / `<br />` 字符串，没有判断是否真的是换行事件。

根因：HTML 换行删除增强在代码上下文识别之前执行，无条件返回覆盖整个标签字符串的替换；事务范围合法，所以事务校验不能阻止这个语义错误。

建议：只对解析结果明确识别为可见换行的节点执行整体删除。代码块、行内代码及转义文本走字素删除。两种删除方向都需回归。

证据：真实编辑策略测试 `code_html_literal_backspace_removes_one_character` 失败；另独立诊断同时复现围栏与行内代码两种情况。前向 Delete 的同源问题为静态确认，未单独进行原生按键回放。

### F4 · P1：富文本粘贴未转义 HTML 起始字符和实体，改变原文

位置：`crates/markdown/src/paste/writer.rs:1104–1115`。

输入剪贴板 HTML：`<p><strong>&lt;br&gt;</strong></p>`，plain 表示为 `<br>`。

实际生成：`**<br>**`；重新解析后内容成了 `InlineHtml("<br>")`，原本可见的字面标签变成换行。另一个输入 `<p><strong>&amp;copy;</strong></p>`、plain 为 `&copy;`，生成 `**&copy;**`，重解析文字变为 `©`。

根因：普通文字的转义分支没有处理 `<` 和 `&`。转换前的富文本模型与 plain 内容一致，不能证明输出 Markdown 再解析后的文字仍然一致。

建议：写出普通文本时保护 HTML 和实体语法，同时验证“富文本 → Markdown → 解析后可见文本”的内容保真。覆盖字面标签、字符实体、链接标题、列表和表格内容。

证据：`rich_paste_preserves_literal_html_and_entities` 失败；独立诊断同时记录上述两种输入的输出与解析事件。

### F5 · P2：代码格式命令使用固定分隔符，选区包含反引号时结构错误

位置：`crates/markdown/src/commands.rs:36–37, 105–112, 197–206`。

行内复现：选择普通文本 ``a`b``，执行行内代码命令。生成 `` `a`b` ``，解析结果只有 `a` 属于 Code，`b` 和剩余反引号变成普通文本。预期整个选区都成为同一段代码。

块级复现：选择 ````before\n```\nafter````，执行代码块命令。固定三个反引号被选区内的围栏提前闭合，结果解析成两个代码块及中间普通段落。

根因：行内代码沿用普通样式标记拼接；代码块固定 `CODE_MARKER.repeat(3)`，都没有检查内容中的连续分隔符。粘贴 writer 已有根据内容计算反引号长度的路径，而格式命令没有采用同等规则。

建议：代码格式化使用专用序列化规则，按最长反引号串选择安全长度，并处理首尾空格。取消格式也应识别实际分隔符长度。

证据：`inline_code_command_preserves_embedded_backticks`、`code_block_command_preserves_embedded_fences` 均失败。

### F6 · P2：编辑与下一次绘制之间，方向键查询不可用

位置：`crates/markdown/src/layout/types.rs:177–183`，`view.rs:2492–2501`；消费方 `crates/app/src/dispatch/wysiwyg.rs:94–111`。

复现顺序：渲染 `a|bc` → 插入 `X` 并同步为 `aX|bc`、generation 增加 → 暂不 render → 查询 Right。

实际：`VisualMove` 返回 `BytePosition(None)`；相同文档 render 后立即返回正确的 `Some(3)`。应用导航分支在空结果下返回 `AppEffect::NONE`，没有延迟重试。这使得同一轮事件处理期间到达的后续方向键可能无效。

根因：更新 source generation 立即清空投影索引，但只有 render 才重建；应用在导航前同步源码，却不保证可供查询的新投影已经准备好。

建议：明确查询所需的布局准备协议，避免导航正确性依赖下一次绘制。保留版本校验，不能为了让查询成功而继续使用旧源码索引。

证据：`navigation_remains_available_between_edit_and_paint` 失败；诊断记录未绘制/已绘制两阶段分别为 `None` / `Some(3)`。已经验证接口事件顺序，未测量真实设备上该事件组合的发生频率。

### F7 · P2：WYSIWYG 搜索高亮的提供方与消费方脱节

位置：`crates/markdown/src/view.rs:2867–2869`；消费方 `crates/app/src/app_renderer.rs:989–1011`、`crates/appkit-shell/src/editor_runtime/editor_painter.rs:169–184`。

复现：文档 `one two one`，搜索 `one`。

实际：`SearchHighlights` 始终返回空 DrawList，两个可见匹配都没有专用搜索高亮。当前匹配可能因为应用更新了选区而得到普通选区高亮；这不能替代所有匹配的搜索高亮。

根因：插件写明“交给 app”，但 app 的 WYSIWYG 绘制分支仍调用插件 `search_highlights()` 来获取矩形，没有自己构造搜索几何。双方没有实际完成这项职责。

建议：让源码搜索结果与投影几何建立明确接口，覆盖所有匹配及活动匹配，避免直接套用预览模式的可见文本搜索序号，因为它与源码搜索结果可能不同。

证据：`search_highlights_include_visible_matches` 失败，日志 `SEARCH HIGHLIGHT COUNT 0`。搜索跳转另有 `ScrollToSearchMatch` 消息未在 Editor 分支实现的静态风险，但当前光标可见性机制可能产生补偿，本次不把“不能跳转”列为已确认结论。

### F8 · P2：选区内 IME 预编辑的位置与最终替换语义不一致

位置：`crates/markdown/src/edit.rs:207–216`，上下文结构 `edit.rs:8–16`；最终替换路径 `view.rs:2671–2675` 与 app 默认事务。

复现：文档 `one two one`，选择第一个 `one`（0..3，光标在 3），开始预编辑“新”。

实际可见行：`one新 two one`。最终提交按选区替换时应得到 `新 two one`。组合文字显示在选区末尾，提交后将移到选区起点；预编辑所见位置和实际替换结果不一致。

根因：`EditContext` 只有光标和组合文字，没有待替换选区；`materialize_projected_line` 只在 `cursor_byte` 插入虚拟文本，不投影选区替换。当前源码保留本身是正确的，缺失的是临时可见投影对替换范围的表达。

建议：预编辑投影显式携带 replacement range，同时保留原始文档用于取消组合。验证正反向选择、跨行选区、不同组合光标偏移及提交/取消后的几何。

证据：`preedit_previews_replacement_at_selection_start` 失败。这里验证的是插件预编辑消息与布局，不是对某个 macOS 输入法进行原生窗口实操；未发现该案例提交后的文本损坏。

## 基线、复现和边界

本次实际运行：

| 验证 | 结果 |
|---|---|
| `cargo test -p textora-markdown --lib` | 1282 通过，0 失败 |
| `cargo build -p textora-markdown` | 成功，独立程序链接当前编译产物 |
| `cargo test -p textora-app` | 924 单元测试通过、2 个既有忽略；12 个集成测试通过；doctests 通过 |
| 附录中的 9 个新增行为断言 | 成功编译，9 个失败，分别对应上述 8 类缺口 |

日志：`/tmp/textora-md-audit-baseline.log`、`/tmp/textora-app-audit-baseline.log`、`/tmp/textora-root-audit-tests.log`。诊断程序 `/tmp/textora-root-audit.rs`，详细几何输出 `/tmp/textora-root-audit.log`。临时路径可能被系统清理，因此完整复现代码保留在下方附录。

未运行完整 `./scripts/verify.sh`：本次只审查并新增报告，没有修改产品实现。未执行真实应用窗口的键鼠、滚动和 IME 回放；也未将基线测试通过解读为这些路径已经不存在问题。

排除的候选：预览 `selection::word_at_pos` 混用 char 与 grapheme 下标确有局部错误，但实际 WYSIWYG 双击路径使用 `DocumentModel::word_select_at`，所以本报告不把该问题计入 WYSIWYG 的 8 项发现。旧空段落审查的问题也没有直接复用，均以当前代码及本次测试为准。

## 建议修复次序

1. 先处理 F1/F2：修复源码投影的真实性和合法性，恢复基础定位能力。为图片、代码、实体建立共同的不变量，但各自保留最小回归。
2. 处理 F3/F4/F5：先阻止错误的内容转换，再补解析后的保真断言；每个根因独立修改和验证。
3. 处理 F6/F7/F8：明确导航准备、搜索矩形与预编辑替换范围的接口，并补应用事件顺序测试。

修复继续遵守 ui 与 app 的边界：ui 接收纯数据；不要让 UI 直接访问 DocumentView，也不要用局部像素偏移掩盖源码锚点问题。实施修复后再执行完整验证脚本及原生窗口回放。

## 附录：可独立编译的复现程序

将下方 Rust 代码保存到 `/tmp/textora-root-audit.rs`。先在仓库构建 `textora-markdown`，然后使用这次构建匹配的 markdown、ui、core、shaping rlib 链接。以下文件名为本次审查实际使用的产物；后续构建哈希可能变化。

```sh
rustc --edition=2024 --test /tmp/textora-root-audit.rs \
  -L dependency=target/debug/deps \
  --extern textora_markdown=target/debug/deps/libtextora_markdown-7d868ab74d59c12d.rlib \
  --extern ui=target/debug/deps/libui-63391d33a21cb912.rlib \
  --extern core=target/debug/deps/libcore-744d25b638fc8e2a.rlib \
  --extern shaping=target/debug/deps/libshaping-a54221f644e54e18.rlib \
  -o /tmp/textora-root-audit-tests
/tmp/textora-root-audit-tests --test-threads=1
```

下面测试表达预期行为，修复前失败是本次取证结果。它们没有加入默认测试集。

```rust
extern crate core as doc_core;
use std::borrow::Cow;
use doc_core::document::{DocView, DocViewMut, StringDocView};
use textora_markdown::view::MarkdownEditorView;
use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin, MoveDirection};
struct Document(String);
impl DocView for Document {
 fn line_count(&self)->usize{StringDocView::new(&self.0).line_count()}
 fn doc_line_text(&self,line:usize)->Cow<'_,str>{Cow::Owned(StringDocView::new(&self.0).doc_line_text(line).into_owned())}
 fn doc_text_in_range(&self,range:std::ops::Range<usize>)->Cow<'_,str>{Cow::Borrowed(&self.0[range])}
 fn line_byte_offset(&self,line:usize)->usize{StringDocView::new(&self.0).line_byte_offset(line)}
 fn line_byte_length(&self,line:usize)->usize{StringDocView::new(&self.0).line_byte_length(line)}
 fn scroll_y(&self)->f32{0.0}
 fn viewport_height(&self)->f32{600.0}
}
impl DocViewMut for Document {
 fn set_scroll_y(&mut self,_:f32){}
 fn replace_range(&mut self,range:std::ops::Range<usize>,text:&str){self.0.replace_range(range,text)}
}
fn render(view:&mut MarkdownEditorView,doc:&Document){
 let mut shaper=shaping::Shaper::new().expect("shaper available");
 view.render(doc,ui::Rect::new(0.0,0.0,800.0,600.0),&ui::theme::test_theme(),&mut shaper,1.0);
}
fn main(){
 for source in ["abc `中文` tail","abc `xyz` tail","a &NotEqualTilde; z","abc ![x](y) tail", "abc ![中文](x) tail", "before\n\nabc ![x](y) tail\n\nafter"] {
  let mut doc=Document(source.to_owned());
  let mut view=MarkdownEditorView::new();view.set_source(source.to_owned(),1);
  view.handle_message(PluginMessage::SetCursorByte(0),&mut doc);
  render(&mut view,&doc);
  println!("SOURCE {source:?} FOLDED {:?}",view.engine().flat_lines().iter().map(|l|l.text.clone()).collect::<Vec<_>>());
  for (byte,_) in source.char_indices(){
   let position=view.query(PluginQuery::CursorScreenPos(byte),&doc);
   println!("CURSOR {byte}: {position:?}");
  }
  view.handle_message(PluginMessage::SetCursorByte(5),&mut doc);
  render(&mut view,&doc);
  println!("ACTIVE {:?} CURSOR {:?}",view.engine().flat_lines().iter().map(|l|l.text.clone()).collect::<Vec<_>>(),view.query(PluginQuery::CursorScreenPos(5),&doc));
 }
 let mut doc=Document("abc".into());let mut view=MarkdownEditorView::new();view.set_source(doc.0.clone(),1);
 view.handle_message(PluginMessage::SetCursorByte(1),&mut doc);render(&mut view,&doc);
 doc.0="aXbc".into();view.set_source(doc.0.clone(),2);view.handle_message(PluginMessage::SetCursorByte(2),&mut doc);
 println!("STALE MOVE {:?}",view.query(PluginQuery::VisualMove{current_byte:2,direction:MoveDirection::Right,target_x:None},&doc));
 render(&mut view,&doc);
 println!("FRESH MOVE {:?}",view.query(PluginQuery::VisualMove{current_byte:2,direction:MoveDirection::Right,target_x:None},&doc));
 let mut doc=Document("one two one".into());let mut view=MarkdownEditorView::new();view.set_source(doc.0.clone(),1);
 view.handle_message(PluginMessage::SetCursorByte(0),&mut doc);render(&mut view,&doc);
 let highlight=view.query(PluginQuery::SearchHighlights{query:"one".into(),match_case:true,use_regex:false,active_idx:0,match_color:ui::theme::test_theme().palette.highlight,inactive_color:ui::theme::test_theme().palette.inactive_highlight},&doc);
 if let PluginResponse::DrawList(dl)=highlight{println!("SEARCH HIGHLIGHT COUNT {}",dl.cmds.len())}
 view.handle_message(PluginMessage::SetSelAnchorByte(Some(0)),&mut doc);
 view.handle_message(PluginMessage::SetSelCursorByte(Some(3)),&mut doc);
 view.handle_message(PluginMessage::SetCursorByte(3),&mut doc);
 view.handle_message(PluginMessage::SetPreedit{text:"新".into(),cursor:Some((3,3))},&mut doc);
 render(&mut view,&doc);
 println!("PREEDIT REPLACEMENT {:?}",view.engine().flat_lines().iter().map(|l|l.text.clone()).collect::<Vec<_>>());
}

#[cfg(test)]
mod tests {
 use super::*;
 use ui::plugin::{EditIntent, EditPlan, EditRequest};
 fn editor(source:&str,cursor:usize)->(MarkdownEditorView,Document){
  let mut document=Document(source.to_owned());
  let mut view=MarkdownEditorView::new();view.set_source(source.to_owned(),1);
  view.handle_message(PluginMessage::SetCursorByte(cursor),&mut document);
  render(&mut view,&document);
  (view,document)
 }
 fn rendered_text(view:&MarkdownEditorView)->String{
  view.engine().flat_lines().iter().map(|line|line.text.as_str()).collect::<Vec<_>>().join("\n")
 }
 #[test]
 fn image_preserves_unrelated_paragraph_caret_and_navigation(){
  let (view,document)=editor("before\n\nabc ![x](y) tail\n\nafter",0);
  assert!(matches!(view.query(PluginQuery::CursorScreenPos(0),&document),PluginResponse::CursorScreenRect(Some(_))),"image must not remove first paragraph caret");
  assert!(matches!(view.query(PluginQuery::VisualMove{current_byte:0,direction:MoveDirection::Right,target_x:None},&document),PluginResponse::BytePosition(Some(1))));
 }
 #[test]
 fn inline_code_expands_both_delimiters_and_maps_chinese_caret(){
  let (view,document)=editor("abc `中文` tail",5);
  assert_eq!(rendered_text(&view),"abc `中文` tail","active inline code must expose both delimiters");
  assert!(matches!(view.query(PluginQuery::CursorScreenPos(5),&document),PluginResponse::CursorScreenRect(Some(_))));
 }
 #[test]
 fn navigation_remains_available_between_edit_and_paint(){
  let (mut view,mut document)=editor("abc",1);
  document.0="aXbc".into();view.set_source(document.0.clone(),2);
  view.handle_message(PluginMessage::SetCursorByte(2),&mut document);
  assert!(matches!(view.query(PluginQuery::VisualMove{current_byte:2,direction:MoveDirection::Right,target_x:None},&document),PluginResponse::BytePosition(Some(3))),"next input event must see current source without requiring a paint");
 }
 #[test]
 fn search_highlights_include_visible_matches(){
  let (view,document)=editor("one two one",0);
  let response=view.query(PluginQuery::SearchHighlights{query:"one".into(),match_case:true,use_regex:false,active_idx:0,match_color:ui::theme::test_theme().palette.highlight,inactive_color:ui::theme::test_theme().palette.inactive_highlight},&document);
  assert!(matches!(response,PluginResponse::DrawList(ref commands) if !commands.cmds.is_empty()),"two visible matches need search highlight geometry");
 }
 #[test]
 fn preedit_previews_replacement_at_selection_start(){
  let (mut view,mut document)=editor("one two one",3);
  view.handle_message(PluginMessage::SetSelAnchorByte(Some(0)),&mut document);
  view.handle_message(PluginMessage::SetSelCursorByte(Some(3)),&mut document);
  view.handle_message(PluginMessage::SetPreedit{text:"新".into(),cursor:Some((3,3))},&mut document);
  render(&mut view,&document);
  assert_eq!(rendered_text(&view),"新 two one","preedit should show the selection replacement that commit will apply");
 }
 #[test]
 fn code_html_literal_backspace_removes_one_character(){
  for source in ["```\n<br>\n```","`<br>`"]{
   let cursor=source.find("<br>").expect("fixture has html text")+4;
   let (view,_document)=editor(source,cursor);
   let plan=view.edit_policy().plan_edit(&EditRequest{source_generation:1,cursor_byte:cursor,selection:None,intent:EditIntent::DeleteBackward});
   let actual=match plan{
    EditPlan::UseDefault=>{let mut text=source.to_owned();text.replace_range(cursor-1..cursor,"");text}
    EditPlan::Apply(transaction)=>{let mut text=source.to_owned();for replacement in transaction.replacements.iter().rev(){text.replace_range(replacement.range.clone(),&replacement.text)}text}
    other=>panic!("unexpected plan {other:?}")
   };
   let mut expected=source.to_owned();expected.remove(cursor-1);
   assert_eq!(actual,expected,"code contents must remain literal");
  }
 }
 #[test]
 fn rich_paste_preserves_literal_html_and_entities(){
  use textora_markdown::{paste,parser};
  for (html,plain) in [("<p><strong>&lt;br&gt;</strong></p>","<br>"),("<p><strong>&amp;copy;</strong></p>","&copy;")]{
   let prepared=paste::prepare_paste(paste::PasteRepresentations{markdown:None,html:Some(html),rtf:None,plain:Some(plain),source_url:None});
   let markdown=prepared.into_text().expect("converted paste text");
   let parsed=parser::parse_markdown(&markdown);
   let rendered=parsed.events.iter().filter_map(|event|if let parser::MarkdownEvent::Text(text)=event{Some(text.as_str())}else{None}).collect::<String>();
   assert_eq!(rendered,plain,"serialized markdown must preserve clipboard text: {markdown:?}");
  }
 }
 fn formatted_source(source:&str,command:ui::plugin::SemanticEditCommand)->String{
  let plan=textora_markdown::commands::plan_semantic_edit(source,1,source.len(),Some(0..source.len()),command);
  let ui::plugin::SemanticEditPlan::Apply(transaction)=plan else{panic!("format command must apply")};
  let mut text=source.to_owned();
  for replacement in transaction.replacements.iter().rev(){text.replace_range(replacement.range.clone(),&replacement.text)}
  text
 }
 #[test]
 fn inline_code_command_preserves_embedded_backticks(){
  use textora_markdown::parser::{parse_markdown,MarkdownEvent};
  let formatted=formatted_source("a`b",ui::plugin::SemanticEditCommand::ToggleInlineCode);
  let parsed=parse_markdown(&formatted);
  let codes=parsed.events.iter().filter_map(|event|if let MarkdownEvent::Code(text)=event{Some(text.as_str())}else{None}).collect::<Vec<_>>();
  assert_eq!(codes,vec!["a`b"],"formatted source {formatted:?} must preserve all selected code");
 }
 #[test]
 fn code_block_command_preserves_embedded_fences(){
  use textora_markdown::parser::{parse_markdown,MarkdownEvent,MarkdownTag};
  let formatted=formatted_source("before\n```\nafter",ui::plugin::SemanticEditCommand::CodeBlock);
  let parsed=parse_markdown(&formatted);
  let block_count=parsed.events.iter().filter(|event|matches!(event,MarkdownEvent::Start(MarkdownTag::CodeBlock{..}))).count();
  assert_eq!(block_count,1,"formatted source {formatted:?} must form one intact code block");
 }
}
```
