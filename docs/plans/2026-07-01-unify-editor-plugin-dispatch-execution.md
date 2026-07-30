# 执行计划 —— 统一编辑视图 Plugin 分派

- 日期:2026-07-01
- 配套 spec:[`2026-07-01-unify-editor-plugin-dispatch.md`](./2026-07-01-unify-editor-plugin-dispatch.md)
- 范围:把 spec 中的四阶段主线 + 附录 A 三项拆成可执行、可验收的最小步骤。
- 目的:任何人拿到本文档,不用回看 spec 也能一步步推进;每步都有验收命令和回滚点。

---

## 全局约束

- **一步一提交**:下文每个"步骤 N"对应 **一个 commit**;每个"阶段 X"对应 **一个 PR**(合并到 main 之前 rebase 保持步骤颗粒)。
- **验证门槛**:每个步骤完成后,至少通过 `cargo build -p <目标 crate>` + `cargo test -p <目标 crate>`。阶段结束时跑 `./scripts/verify.sh`。
- **每阶段独立回归窗口**:阶段合入 main 后,观察 3-5 天再启动下一阶段,避免多线交叉引入的问题被误归因。
- **修改 `PluginQuery` / `PluginMessage` 枚举**:全局 grep 校验旧名,禁止残留。
- **不改数据模型**:全程 `DocumentView` 是编辑真值,`MarkdownEditorView` 是渲染叠加层。

---

## 主线:四阶段

### 阶段 1 —— 通用化 query,让 `EditorPlugin` 承接几何映射

#### 步骤 1.1  重命名 `VisualMoveWysiwyg → VisualMove`
- 修改文件:`crates/ui/src/plugin.rs` 定义处 + 所有 use 处
- 校验:`rg -n "VisualMoveWysiwyg" crates/` 结果为空
- 验收:`cargo build --workspace`

#### 步骤 1.2  为 `EditorPlugin` 补 `HitTestByte` query
- 修改文件:`crates/app/src/plugins/editor.rs`
- 逻辑:委托 `DocumentView::display` 现有 hit-test。需要在 `plugins/editor.rs` 拿到 `DocumentView` 引用,先评估:
  - 方案 A:扩展 `DocView` trait 暴露 `as_editor_host(&self) -> Option<&dyn EditorHost>`。
  - 方案 B:给 query 入参加 `EditorHostState`,由 App 端算好传入。
- 首选 A(接口清晰),先在 `crates/app/src/editor_host.rs` 起 `EditorHost` trait 骨架。
- 验收:新增单元测试 `plugin_editor_hit_test_matches_doc`(基础编辑器 plugin 与 `DocumentView` 结果一致)。

#### 步骤 1.3  为 `EditorPlugin` 补 `CursorScreenPos` query
- 委托 `cursor_motion` + 视口。
- 验收:`plugin_editor_cursor_screen_pos_matches_doc`。

#### 步骤 1.4  为 `EditorPlugin` 补 `VisualMove` query
- 委托 `DocumentView::cursor_move_*`。
- 验收:`plugin_editor_visual_move_matches_doc`。

#### 步骤 1.5  取消 App 层几何查询的 `is_wysiwyg` 分支
- 修改点:`app_renderer.rs`、`app_window.rs`
- 全部改为无条件 `plugin.query(…)`。
- 校验:`rg -n "is_wysiwyg" crates/app/src/app_renderer.rs crates/app/src/app_window.rs`
- 验收:`cargo test -p textora-app`,重点看 markdown WYSIWYG 编辑测试和基础编辑器鼠标/键盘测试。

#### 阶段 1 出口
- **PR 标题**:`refactor(app): unify geometry queries across editor and markdown plugins`
- **验收**:
  - `./scripts/verify.sh` 全绿。
  - 手工验证:markdown 文档鼠标点击、上下键、IME 位置行为不变;基础编辑器同上。
  - `rg -c "is_wysiwyg\(\)" crates/app/src` 相比阶段 1 前 **减少 ≥ 4 处**。

---

### 阶段 2 —— 通用化 sync,取消 `sync_wysiwyg_plugin_state` 特化

#### 步骤 2.1  给 `EditorPlugin` 加 no-op `handle_message` 实现
- 覆盖:`UpdateSource / SetSelAnchorByte / SetSelCursorByte / SetCursorByte / SetPreedit / SetCursorVisible`
- 全部返回 `false`(不消费)。
- 验收:`cargo test -p textora-app`。

#### 步骤 2.2  重命名 `sync_wysiwyg_plugin_state → sync_plugin_state`
- 位置:`crates/app/src/app_renderer.rs:987`
- 校验:`rg -n "sync_wysiwyg_plugin_state" crates/` 结果为空。

#### 步骤 2.3  移除 sync 中的 `if !plugin.is_wysiwyg() return`
- 位置同上。
- 验收:markdown WYSIWYG 测试全绿(消息还是被消费);基础编辑器 no-op(消息不影响状态)。

#### 步骤 2.4  移除 `app_renderer.rs:479, 521, 994` 三处 `is_wysiwyg()` 分支
- 保留 `handles_own_rendering()` 分支(渲染入口不在本阶段动)。
- 校验:`rg -n "is_wysiwyg" crates/app/src/app_renderer.rs` 结果为 0 处或仅剩注释。

#### 阶段 2 出口
- **PR 标题**:`refactor(app): unify plugin state sync across editor and markdown plugins`
- **验收**:
  - `./scripts/verify.sh` 全绿。
  - 每帧调用一次 `sync_plugin_state` 无明显性能回退(基础编辑器 6 条 no-op 消息)。
  - `rg -c "is_wysiwyg\(\)" crates/app/src` 相比阶段 2 前继续下降。

---

### 阶段 3 —— `EditAugmenter`:统一编辑前置协商

#### 步骤 3.1  在 `ui::plugin` 定义 `EditAugmenter` trait
- 输入:`AugmentContext { current_byte, kind: AugmentKind }`
- 输出:`Option<EditAugmentation>`
- `ViewPlugin` 提供默认方法 `fn augmenter(&self) -> &dyn EditAugmenter { &NoopAugmenter }`

#### 步骤 3.2  `MarkdownEditorView` 实现自定义 augmenter
- 逻辑复用现有 `PluginQuery::AugmentEdit` 分支中的实现。
- 验收:markdown 列表续行、括号配对测试全绿。

#### 步骤 3.3  `EditorPlugin` 使用默认 no-op augmenter
- 无需改动(默认实现即可)。

#### 步骤 3.4  改造 App 层的 Enter/Backspace/Tab 分派
- 位置:`crates/app/src/events.rs:73, 235-236`、`app_dispatch.rs:113-117`
- 从 `PluginInterceptKey action → PluginMessage::InterceptKey` 改为 `plugin.augmenter().augment(ctx)`。
- 取消 `PluginInterceptKey` action、`PluginMessage::InterceptKey`、`PluginQuery::AugmentEdit`(如仍未删除)。

#### 步骤 3.5  清理 `is_wysiwyg()` 剩余引用
- 目标:`events.rs`、`app_dispatch.rs` 内 `is_wysiwyg()` 引用降为 0。
- 允许保留 `ViewPlugin::is_wysiwyg()` 方法本身,供极少数强绑定处使用(例如渲染入口)。

#### 阶段 3 出口
- **PR 标题**:`refactor(app): introduce EditAugmenter trait to unify edit key augmentation`
- **验收**:
  - `./scripts/verify.sh` 全绿。
  - Enter/Backspace/Tab 在 markdown 与基础编辑器中行为不变。
  - `rg -n "is_wysiwyg\(\)" crates/app/src | wc -l` **≤ 1**(理想为 0)。

---

### 阶段 4(可选)—— 光标 / 选区绘制抽公共函数

#### 步骤 4.1  提取 `ui::decorations::draw_caret`
- 签名:`fn draw_caret(dl: &mut DrawList, rect: Rect, theme: &Theme, blink_visible: bool);`
- 两侧调用点接入。
- 验收:视觉像素级一致(可以用现有 UI 快照测试或手工对比)。

#### 步骤 4.2 (更可选)  抽 `LineHighlighter` trait 用于选区高亮
- 若在 4.1 完成后觉得抽象过于臃肿,**直接放弃**这一步 —— 收益<成本。

#### 阶段 4 出口
- **PR 标题**:`refactor(ui): extract shared caret drawing`
- **验收**:markdown / 基础编辑器 caret 与选区高亮视觉一致,单元测试全绿。

---

## 附录 A:并行清理三项

以下三项**任意顺序、任意时机**推进,与主线四阶段互不阻塞。

### A.1 `SearchState` 命名切分与下沉

#### 步骤 A1.1  重命名 `markdown::search::SearchState → SearchHighlightCache`
- 位置:`crates/markdown/src/search.rs`
- 更新调用点:`crates/markdown/src/view.rs` 及测试。
- 验收:`cargo build -p textora-markdown` + 单元测试。

#### 步骤 A1.2  评估 `app::search_state::SearchState` 是否需要下沉到 `core`
- 若 markdown crate 没有真正引用它 → **不下沉,保留原位置**,只调整 rustdoc 明确职责为"搜索会话状态"。
- 若 markdown 后续需要复用匹配逻辑 → 建立 `crates/core/src/search/session.rs`,搬家 + 更新 use。
- 验收:`cargo build --workspace`。

#### 步骤 A1.3  在 `crates/core/README.md` 或 `docs/specs/` 记录职责边界
- 一句话:"搜索**匹配**逻辑归 [session 所在 crate];搜索**高亮 rect 缓存**归各视图。"

#### PR
- **标题**:`refactor(markdown): rename SearchState to SearchHighlightCache to clarify role`
- **验收**:命名清晰,`cargo test -p textora-markdown` + `cargo test -p textora-app` 全绿。

---

### A.2 `word_class` / grapheme 纯函数下沉

#### 步骤 A2.1  新建 `crates/core/src/text/word_class.rs`
- 定义 `pub enum CharClass { Word, Whitespace, Punctuation }` + `pub fn classify(ch: char) -> CharClass`
- 加基础单元测试(ASCII、CJK、标点)。

#### 步骤 A2.2  `markdown/src/selection.rs` 改用 `core::text::word_class`
- 删除本地 `CharClass` / `char_class`
- 调用点全部走 `core::text::word_class::classify`。
- 验收:`cargo test -p textora-markdown`,重点看词选测试。

#### 步骤 A2.3  评估 `core::buffer::navigation::word_select` 是否要复用同一 `CharClass`
- 若语义一致 → 内部改用统一实现。
- 若语义不同(字节/grapheme 粒度差异)→ 保留两套实现,但在 `word_class` 头部 rustdoc 明确"当前 char 粒度,待升级 UAX#29"。

#### PR
- **标题**:`refactor(core): extract shared word_class helper`
- **验收**:`./scripts/verify.sh` 全绿;词选行为无回归。

---

### A.3 `arboard` 剪贴板访问收敛

#### 步骤 A3.1  新建 `crates/app/src/clipboard.rs`
- 提供 `pub fn read_text() -> Option<String>` 与 `pub fn write_text(text: &str) -> bool`
- 内部一次 `arboard::Clipboard::new()` 的错误处理集中化(warn 一次而非 silent-fail)。

#### 步骤 A3.2  改造 `document_view/selection.rs` 复用新模块
- `copy_selection_to_clipboard` / `cut_selection_to_clipboard` / `paste_from_clipboard` 内部改调 `clipboard::write_text` / `read_text`。
- 验收:剪贴板复制/粘贴功能不变。

#### 步骤 A3.3  改造 `workspace.rs:952`、`ui_shell.rs:762/768`
- 全部改走 `crate::clipboard`。
- 校验:`rg -n "arboard::Clipboard::new" crates/app/src` 只在 `clipboard.rs` 命中一处。

#### PR
- **标题**:`refactor(app): centralize clipboard access into single module`
- **验收**:`./scripts/verify.sh` 全绿;复制/剪切/粘贴手工验证正常。

---

## 里程碑与时间估算

| 里程碑 | 内容 | 估算 |
|---|---|---|
| M1 | 阶段 1 完成 | 2-3 天 |
| M2 | 阶段 2 完成 | 1 天 |
| M3 | 阶段 3 完成 | 1-2 天 |
| M4 | 阶段 4(可选) | 0.5-1 天 |
| M-A | 附录 A.1 + A.2 + A.3 全部完成 | 合计 2 天(可并行插入 M1~M3 之间) |

**关键路径**:M1 → M2 → M3 依次串行(下一阶段需要上一阶段的协议基座)。M4 独立。M-A 完全独立,可以随时穿插。

---

## 退出/回滚策略

- **每阶段单 PR**:出问题 revert 单个 PR 即可回到上一里程碑。
- **每步骤单 commit**:PR 内部若某一步骤引入问题,`git rebase -i` 移除即可。
- **协议改动的兼容窗口**:阶段 1 重命名 `VisualMoveWysiwyg → VisualMove` **不留 alias**,一次改到位;因为整个符号只在 workspace 内使用,`rg` 校验足以避免遗漏。

---

## 成功指标(全部完成后)

1. `rg -n "is_wysiwyg\(\)" crates/app/src` ≤ 1 处(仅保留在渲染入口的必要检查)。
2. `crates/app/src/plugins/editor.rs` 从 stub(38 行)升级为承接几何/协商能力的实现(预计 200-300 行)。
3. `sync_wysiwyg_plugin_state` 更名为 `sync_plugin_state`,不再有 `is_wysiwyg` 短路。
4. `PluginInterceptKey` action 与 `PluginMessage::InterceptKey` 已删除(被 `EditAugmenter` 取代)。
5. `SearchState` / `word_class` / `clipboard` 三项工具分层清晰,搜索命名不再歧义,新 View 接入门槛降低。
6. `./scripts/verify.sh` 全绿;markdown 与基础编辑器手工验证行为无回归。
