# 最小 EditorRuntime 抽取执行计划

日期：2026-07-30

状态：待执行

## 1. 目标

依据
[`docs/specs/2026-07-30-minimal-editor-runtime-design.md`](../specs/2026-07-30-minimal-editor-runtime-design.md)
抽取可被 textora 和未来 notora 共同使用的最小 `EditorRuntime`。

完成后：

- `textora-appkit-shell` 公开稳定、窄接口的 `EditorRuntime`；
- runtime 持有文档会话、编辑输入、窗口/GPU/文本、reshape、文件安全和保存机制；
- `textora-app::App` 只保留产品状态、`UiShell`、产品事件翻译和
  `ApplicationHandler<ShellEvent>` 薄入口；
- 产品先处理 chrome、modal 和产品焦点，再把剩余窗口事件交给 runtime；
- 产品和编辑器通过同一个 `EditorFrame` 向同一个 surface 绘制；
- 保存时机仍由产品决定，runtime 只提供不可变保存快照和完成结果应用；
- 一个不依赖 `textora-app` 的假产品测试可在非零偏移矩形中完成完整编辑生命周期；
- 不创建 notora crate，不改变 textora 的用户行为和持久化格式。

## 2. 当前基线

本计划以 2026-07-30 的代码状态为起点：

- `Workspace`、`PreparedTab`、`TabRuntimeStore`、`TabSession`、
  `UiShell`、GPU/Text、reshape 和渲染基础模块已经位于
  `crates/appkit-shell`；
- `DocumentModel`、`WorkspaceModel`、文件安全、持久化和快照基础能力已经位于
  `crates/appkit-core`；
- `textora-app::App` 仍直接持有 `Workspace`、`TabRuntimeStore`、
  window/GPU/Text、输入状态、reshape 状态和文件安全状态；
- `textora-app` 仍负责编辑命令、鼠标/IME 分发、帧组合、同步保存和关闭提示；
- `UiShell` 是 textora 产品适配器，继续留在 `App`，不进入 runtime；
- `DocumentModel` 已有 `content_revision` 和 `disk_revision`，但保存仍通过
  UI 线程上的 `save`/`save_as` 同步完成。

## 3. 实施约束

- 每个实现任务最多修改 3 个逻辑文件；源文件与目标文件的纯移动算一个逻辑文件。
- 每次提交前至少运行：

  ```bash
  cargo fmt --all -- --check
  cargo check -p textora-app
  bash scripts/check_architecture.sh
  ```

- 行为变更必须先增加失败测试；纯移动必须在移动前后运行相同测试。
- 同一个失败若连续修改两次仍未解决，停止叠加补丁，回到所有权和借用设计重新审查。
- 不新增 `workspace_mut()`、`document_mut()`、`gpu_mut()`、
  `tab_runtime_store_mut()` 或 `Deref<Target = Workspace>` 等逃生接口。
- 不用字符串 action、`Box<dyn Any>` 产品状态袋、全局回调表或裸指针绕过借用。
- `EditorRuntime`、`appkit-shell` 和 `appkit-core` 不出现 `TextoraProduct`、
  `NoteId`、自动保存策略、`.edit+`、`.notora` 或具体 Markdown/Mindmap
  注册逻辑。
- `UiShell` 不移入 runtime；runtime 只接收产品计算出的 `editor_rect`。
- `ProductWake` 继续无 payload；编辑器后台结果使用现有类型化 `ShellEvent` 路由。
- textora 原 workspace、settings、snapshot、history 和 pinned 格式不变。
- 本计划不升级 `winit`、`wgpu` 或其他基础依赖。
- 重大阶段 ER3、ER4、ER5 结束时运行 `./scripts/verify.sh`。

## 4. 目标模块布局

最终建议布局：

```text
crates/appkit-shell/src/editor_runtime/
├── mod.rs                 # EditorRuntime、生命周期和窄 query
├── contract.rs            # config/input/outcome/notification/error 公共类型
├── input_session.rs       # modifier/mouse/IME/cursor/WYSIWYG 输入会话
├── render_session.rs      # window/GPU/Text/frame timing/resize
├── reshape_session.rs     # font system/worker/generation/pending results
├── editor_frame.rs        # 同 surface 的产品 + 编辑器帧 API
├── file_safety_session.rs # tracked/pending/worker 和结果应用
└── document_save.rs       # PreparedDocumentSave/SaveCompletion/执行函数
```

`crates/app/src` 最终保留：

```text
App
├── EditorRuntime
├── UiShell
├── ProductPaths / WorkspaceStore
├── product settings/theme registry/persistence
├── TextoraProduct / sync / native menu
├── product focus and chrome state
└── product event/action translation
```

如果编译 spike 证明某个私有会话必须拆成不同文件，可调整内部模块名，但不得扩大
公共 API 或改变上述所有权边界。

## 5. 公共契约冻结

实现前先冻结以下语义，后续任务不得用临时布尔值或字符串替代：

- `OpenDisposition::{Preview, Persistent}`；
- `EditorFocus::{Inactive, Active}`；
- `EditorInputContext { editor_rect, focus, modal_blocked }`；
- `EditorNotification` 和 `EditorOutcome`；
- `EditorRuntimeConfig`；
- `EditorDocumentSummary`；
- `PreparedDocumentSave` 和 `SaveCompletion`；
- `EditorFrame` 的短借用闭包与消费式 `present(self)`。

关闭确认在编译 spike 中固定为：

```rust
pub enum CloseConfirmation {
    Saved,
    Discard,
    Cancel,
}
```

语义：

- `Saved` 只能在对应 revision 已成功保存后使用；
- `Discard` 明确放弃未保存内容并关闭；
- `Cancel` 不改变文档；
- pinned tab 不能通过 confirmation 绕过；
- 关闭后迟到的 save/reshape/file-safety 结果必须被忽略。

## 6. 阶段总览

| 阶段 | 结果 | 进入下一阶段的门槛 |
|---|---|---|
| ER0 | 行为、架构和字段所有权基线固定 | 基线测试与架构检查通过 |
| ER1 | runtime 接管 Workspace/TabRuntimeStore 和 tab 生命周期 | App 不再直接持有两个字段 |
| ER2 | runtime 接管编辑输入、IME、焦点和编辑通知 | 非 Editor 焦点不能修改文档 |
| ER3 | runtime 接管 window/GPU/Text/reshape 和帧提交 | textora 通过 `EditorFrame` 绘制 |
| ER4 | runtime 接管文件安全与异步保存机制 | 保存竞态和外部修改测试通过 |
| ER5 | 假产品完成第二消费者验收并删除迁移接口 | 全量验证与手工回归通过 |

---

## ER0：建立不可回退的行为与架构基线

### Task ER0-1：固定编辑器行为和任意矩形基线

**文件：**

- 修改：`crates/app/src/app_tests.rs`
- 修改：`crates/app/src/events.rs`
- 修改：`crates/app/tests/render_smoke.rs`

**步骤：**

1. 先运行现有打开、切换、编辑、关闭、Markdown/Mindmap 和 IME 相关测试，记录
   当前测试名；不为计划中的新 API改写断言。
2. 在 `app_tests.rs` 增加基线测试，覆盖：
   - `PreparedTab` 安装后的 model/runtime 使用同一 `TabId`；
   - 编辑后 `content_revision` 递增且 dirty 变为 true；
   - Markdown toggle 前后内容和 `TabId` 不变；
   - Mindmap 编辑仍通过 transaction 更新同一文档。
3. 在 `events.rs` 增加产品优先路由基线：
   - modal 消费键盘、IME 和鼠标后编辑器不变；
   - search/widget 获得焦点时键盘输入不进入文档；
   - 编辑器拖拽开始后，移出 editor rect 的 move/up 仍完成或取消原拖拽。
4. 在 `render_smoke.rs` 增加非零 `x/y`、非全屏尺寸的 editor rect smoke，
   断言裁剪、光标和命中坐标使用矩形原点。

**验证：**

```bash
cargo test -p textora-app --lib app_tests
cargo test -p textora-app --lib events::tests
cargo test -p textora-app --test render_smoke
```

**提交：**

```bash
git add crates/app/src/app_tests.rs crates/app/src/events.rs crates/app/tests/render_smoke.rs
git commit -m "test(runtime): freeze editor extraction behavior"
```

### Task ER0-2：强化 shared crate 架构守卫

**文件：**

- 修改：`scripts/check_architecture.sh`
- 修改：`crates/appkit-shell/src/lib.rs`

**步骤：**

1. 先运行 `bash scripts/check_architecture.sh`，记录现有通过基线。
2. 在架构脚本中增加 source guard：
   - `appkit-shell` 禁止 `textora_markdown`、`textora_sync`、`TextoraProduct`、
     `NoteId`、`.edit+`、`.notora`；
   - `appkit-core` 禁止产品路径和产品类型；
   - 禁止 runtime 模块公开命中 `workspace_mut`、`document_mut`、`gpu_mut`、
     `tab_runtime_store_mut`。
3. 在 shell `lib.rs` 的测试中增加公共依赖边界检查，禁止为了 runtime
   反向引入 `textora-app`。
4. guard 中拆分禁用词字符串，避免检查器误报自身测试源码。

**验证：**

```bash
bash scripts/check_architecture.sh
cargo tree -p textora-appkit-core
cargo tree -p textora-appkit-shell
```

**提交：**

```bash
git add scripts/check_architecture.sh crates/appkit-shell/src/lib.rs
git commit -m "test(architecture): guard editor runtime boundaries"
```

### Task ER0-3：完成公共 API 和借用编译 spike

**文件：**

- 新增：`crates/appkit-shell/src/editor_runtime/contract.rs`
- 新增：`crates/appkit-shell/src/editor_runtime/mod.rs`
- 修改：`crates/appkit-shell/src/lib.rs`

**步骤：**

1. 先为下列类型写构造、合并和模式匹配测试：
   - `EditorRuntimeConfig`；
   - `OpenDisposition`、`EditorFocus`、`EditorInputContext`；
   - `EditorNotification`、`EditorOutcome`；
   - `CloseConfirmation`、`EditorDocumentSummary`；
   - `EditorRuntimeError` 和领域错误包装。
2. 实现最小类型和空 runtime 壳，不接管 App 字段。
3. 用编译测试确认：
   - `EditorOutcome` 使用 `SmallVec<[EditorNotification; 4]>`；
   - `EditorFrame<'_>` 的 layout/paint 闭包不能逃逸 context；
   - `present(self)` 消费 frame；
   - 产品可在同一 frame 中按“chrome → editor → overlay”顺序调用；
   - 无需裸指针、`unsafe` 或长期保存 `LayoutCtx`/`PaintCtx`。
4. spike 只冻结借用形式，不实现 GPU 提交；不能增加可变内部 getter。

**验证：**

```bash
cargo test -p textora-appkit-shell editor_runtime::contract
cargo check -p textora-appkit-shell
bash scripts/check_architecture.sh
```

**提交：**

```bash
git add crates/appkit-shell/src/editor_runtime crates/appkit-shell/src/lib.rs
git commit -m "feat(appkit-shell): define editor runtime contracts"
```

**ER0 完成条件：**

- 行为基线全部通过；
- 公共契约可编译；
- 架构脚本能阻止产品语义和可变逃生接口进入 shared crates；
- `App` 字段按“runtime / product / 迁移期”完成清单核对，没有无归属字段。

---

## ER1：收拢文档会话与 tab 生命周期

### Task ER1-1：实现无窗口的 runtime model session

**文件：**

- 新增：`crates/appkit-shell/src/editor_runtime/model_session.rs`
- 修改：`crates/appkit-shell/src/editor_runtime/mod.rs`
- 修改：`crates/appkit-shell/src/workspace.rs`

**步骤：**

1. 先写失败测试：
   - runtime 构造时持有空 `Workspace + TabRuntimeStore`；
   - 安装 prepared tab 后两侧 ID 集合严格一致；
   - preview 被下一次 preview 替换时只移除对应 runtime；
   - persistent tab 不被 preview 替换；
   - activate/close 后不存在孤儿 runtime。
2. 给 `Workspace` 增加基于稳定 `TabId` 和 `OpenDisposition` 的窄语义方法。
   `preview_index` 继续私有，不向 runtime 公开 index setter。
3. `ModelSession` 私有持有 `Workspace` 和 `TabRuntimeStore`，集中执行 effect
   reconciliation；调用者不得忘记清理 runtime。
4. 未知 `TabId` 返回明确 no-op/outcome，不 panic。
5. 为分批迁移 textora 旧调用点，允许一个精确命名且
   `#[doc(hidden)]` 的 `with_model_session_for_migration` 闭包桥：
   - 闭包借用不能逃逸；
   - 不返回 `Workspace`、`TabRuntimeStore` 或 `DocumentModel` 引用；
   - 架构测试只允许这一处定义；
   - ER2/ER3/ER4 每迁完一组行为就缩小其用途；
   - ER5-2 必须删除，不能成为最终公共 API。

**验证：**

```bash
cargo test -p textora-appkit-shell editor_runtime::model_session
cargo test -p textora-appkit-shell workspace::tests
```

**提交：**

```bash
git add crates/appkit-shell/src/editor_runtime/model_session.rs \
  crates/appkit-shell/src/editor_runtime/mod.rs \
  crates/appkit-shell/src/workspace.rs
git commit -m "feat(runtime): own workspace and tab runtime session"
```

### Task ER1-2：实现安装、激活、关闭和窄查询

**文件：**

- 修改：`crates/appkit-shell/src/editor_runtime/model_session.rs`
- 修改：`crates/appkit-shell/src/editor_runtime/mod.rs`
- 修改：`crates/appkit-shell/src/editor_runtime/contract.rs`

**步骤：**

1. 为 `install_prepared_tab`、`activate`、`request_close`、
   `confirm_close` 先写 RED 测试。
2. 所有状态变化返回 `EditorOutcome`：
   - active 变化发送 `ActiveDocumentChanged`；
   - dirty close 发送 `CloseRequested`；
   - path/dirty 的实际变化只发送一次通知；
   - shell 通用行为只进入 `ShellEffect`。
3. 实现只读 query：
   - `active_tab_id`；
   - `tab_id_for_path`；
   - `document_summary`；
   - 当前 tab 顺序的只读 summary 快照。
4. `EditorDocumentSummary` 只含产品展示和保存调度需要的数据，不暴露
   `DocumentModel` 或 `TabRuntime` 引用。
5. `confirm_close(Saved)` 校验当前 dirty/revision 状态，不能把未成功保存的文档
   当作已保存关闭。

**验证：**

```bash
cargo test -p textora-appkit-shell editor_runtime
cargo check -p textora-appkit-shell
```

**提交：**

```bash
git add crates/appkit-shell/src/editor_runtime
git commit -m "feat(runtime): add semantic document lifecycle"
```

### Task ER1-3：为 App 建立单一 runtime 适配边界

**文件：**

- 修改：`crates/app/src/app.rs`
- 修改：`crates/app/src/app_tab.rs`
- 修改：`crates/app/src/app_init.rs`

**步骤：**

1. 先增加 source test，统计 `App` 外部直接访问 `workspace` 和
   `tab_runtime_store` 的生产调用点，作为待清零清单。
2. 在 `app_tab.rs` 中集中 tab/session 查询、产品命令和 notification 应用；
   不在 `App` 上新增通用 `workspace_mut`/`document_mut` getter。
3. 适配方法在本任务中仍委托给现有 `App.workspace` 和
   `App.tab_runtime_store`，以保证后续文件可逐批迁移且每次提交都能编译。
4. ER1-4 至 ER1-9 只能改用这些适配方法，不能提前创建第二份 workspace
   source of truth。
5. `App::new` 创建真正 runtime 和删除旧字段延后到 ER1-10 原子完成。
6. source test 阻止适配边界之外新增直接字段访问。

**验证：**

```bash
cargo test -p textora-app --lib app_tab
cargo test -p textora-app --lib app_init
cargo check -p textora-app
```

**提交：**

```bash
git add crates/app/src/app.rs crates/app/src/app_tab.rs crates/app/src/app_init.rs
git commit -m "refactor(app): establish editor runtime adapter"
```

### Task ER1-4：迁移 workspace 持久化和产品装配调用点

**文件：**

- 修改：`crates/app/src/workspace_persistence.rs`
- 修改：`crates/app/src/workspace_product.rs`
- 修改：`crates/app/src/workspace_tab_factory.rs`

**步骤：**

1. 先运行 workspace restore、dirty snapshot 和 typed untitled 全部测试。
2. 恢复流程继续由产品构造 `PreparedTab`，然后经 App 适配边界安装；不把文件
   加载、typed untitled 或产品默认内容移入 runtime。
3. 持久化经适配边界取得只读 session snapshot 构造原 DTO；不得在本文件继续
   读取两个旧字段。
4. 删除、重命名和 detach 等产品操作改为适配边界上的语义命令；path/dirty
   通知由产品更新 workspace store、history 和文件监控。
5. 保持 TOML 字段、snapshot filename、active tab 和 preview 恢复语义不变。

**验证：**

```bash
cargo test -p textora-app --lib workspace_persistence
cargo test -p textora-app --lib workspace_product
cargo test -p textora-app --lib workspace_tab_factory
```

**提交：**

```bash
git add crates/app/src/workspace_persistence.rs \
  crates/app/src/workspace_product.rs \
  crates/app/src/workspace_tab_factory.rs
git commit -m "refactor(app): route workspace assembly through runtime"
```

### Task ER1-5：迁移 tab/chrome/action 调用点

**文件：**

- 修改：`crates/app/src/events.rs`
- 修改：`crates/app/src/dispatch/tabs.rs`
- 修改：`crates/app/src/dispatch/chrome.rs`

**步骤：**

1. tab index 只在当前 widget action 翻译时存在，立即解析成稳定 `TabId`。
2. tab 切换、close others/right/all、pin 和 popup snapshot 改为调用 runtime
   语义 API 或产品持有的稳定 ID 快照。
3. 禁止把 widget index 保存为跨帧运行时关联键。
4. 验证 preview 替换、reorder 后 popup 目标和批量关闭仍指向原 `TabId`。

**验证：**

```bash
cargo test -p textora-app --lib events::tests
cargo test -p textora-app --lib dispatch::tabs
cargo test -p textora-app --lib dispatch::chrome
```

**提交：**

```bash
git add crates/app/src/events.rs crates/app/src/dispatch/tabs.rs \
  crates/app/src/dispatch/chrome.rs
git commit -m "refactor(app): use runtime tab lifecycle commands"
```

### Task ER1-6：迁移通用 dispatch、scroll 和 window 调用点

**文件：**

- 修改：`crates/app/src/app_dispatch.rs`
- 修改：`crates/app/src/app_scroll.rs`
- 修改：`crates/app/src/app_window.rs`

**步骤：**

1. 将 active tab、session、summary、viewport 和插件查询改经 ER1-3 的 App
   适配边界。
2. `app_dispatch` 中所有 index 只用于当前布局，跨调用前转成 `TabId`。
3. `app_scroll` 不再直接组合 workspace/runtime store；滚动操作通过稳定 tab
   session 适配方法完成。
4. `app_window` 的 cursor blink、viewport 和窗口标题查询只消费窄 summary。
5. 本任务不改变输入、reshape 或 window 所有权，只消除直接字段访问。

**验证：**

```bash
rg -n 'self\\.(workspace|tab_runtime_store)|app\\.(workspace|tab_runtime_store)' \
  crates/app/src/app_dispatch.rs crates/app/src/app_scroll.rs crates/app/src/app_window.rs
cargo test -p textora-app --lib app_dispatch
cargo test -p textora-app --lib app_scroll
cargo test -p textora-app --lib app_window
```

**提交：**

```bash
git add crates/app/src/app_dispatch.rs crates/app/src/app_scroll.rs \
  crates/app/src/app_window.rs
git commit -m "refactor(app): isolate dispatch session access"
```

### Task ER1-7：迁移 editor、mouse 和 WYSIWYG 调用点

**文件：**

- 修改：`crates/app/src/dispatch/editor.rs`
- 修改：`crates/app/src/dispatch/mouse.rs`
- 修改：`crates/app/src/dispatch/wysiwyg.rs`

**步骤：**

1. 把直接 workspace/runtime store 访问替换为稳定 `TabId` session 适配调用。
2. 保持实际输入状态和编辑算法不变；它们将在 ER2 按 RED-GREEN 迁移。
3. transaction 完成后的 dirty/content revision 从同一个 session 读取，不能
   再次按 index 查找。
4. 同文件测试 fixture 也必须走适配边界，避免测试保留逃生路径。

**验证：**

```bash
rg -n 'self\\.(workspace|tab_runtime_store)|app\\.(workspace|tab_runtime_store)' \
  crates/app/src/dispatch/editor.rs crates/app/src/dispatch/mouse.rs \
  crates/app/src/dispatch/wysiwyg.rs
cargo test -p textora-app --lib dispatch::editor
cargo test -p textora-app --lib dispatch::mouse
cargo test -p textora-app --lib dispatch::wysiwyg
```

**提交：**

```bash
git add crates/app/src/dispatch/editor.rs crates/app/src/dispatch/mouse.rs \
  crates/app/src/dispatch/wysiwyg.rs
git commit -m "refactor(app): isolate editor session access"
```

### Task ER1-8：迁移 lifecycle、save command 和 renderer 调用点

**文件：**

- 修改：`crates/app/src/app_lifecycle.rs`
- 修改：`crates/app/src/dispatch/commands.rs`
- 修改：`crates/app/src/app_renderer.rs`

**步骤：**

1. lifecycle 的 file-safety candidate 和结果应用经稳定 `TabId` 适配方法完成。
2. 保存命令暂时保持同步行为，但不再直接取得 document 可变引用；ER4 再替换为
   异步协议。
3. renderer 使用只读 frame/session snapshot，不自行组合 workspace/store。
4. 保持现有渲染、文件安全竞态和 Save/Save As 测试完全不变。

**验证：**

```bash
rg -n 'self\\.(workspace|tab_runtime_store)|app\\.(workspace|tab_runtime_store)' \
  crates/app/src/app_lifecycle.rs crates/app/src/dispatch/commands.rs \
  crates/app/src/app_renderer.rs
cargo test -p textora-app --lib app_lifecycle
cargo test -p textora-app --lib dispatch::commands
cargo test -p textora-app --lib app_renderer
```

**提交：**

```bash
git add crates/app/src/app_lifecycle.rs crates/app/src/dispatch/commands.rs \
  crates/app/src/app_renderer.rs
git commit -m "refactor(app): isolate lifecycle and render session access"
```

### Task ER1-9：迁移 reshape 调用点并清零直接字段访问

**文件：**

- 修改：`crates/app/src/app_reshape.rs`
- 修改：`crates/app/src/app_tab.rs`

**步骤：**

1. `app_reshape` 通过适配边界取得 active presentation/session，不直接读取
   workspace/store。
2. 检查 ER1 涉及的所有生产源文件，确保只有 `app.rs`、`app_init.rs` 和
   `app_tab.rs` 的受控适配实现仍可命中旧字段。
3. 在 `app_tab.rs` source test 中维护精确 allowlist；任何新增命中都失败。

**验证：**

```bash
rg -n '(self|app)\\.(workspace|tab_runtime_store)' crates/app/src --glob '*.rs'
cargo test -p textora-app --lib app_reshape
cargo test -p textora-app --lib app_tab
cargo check -p textora-app
```

预期：生产命中只剩所有权字段、构造和适配实现；测试 fixture 命中将在 ER1-10
随所有权切换一起清理。

**提交：**

```bash
git add crates/app/src/app_reshape.rs crates/app/src/app_tab.rs
git commit -m "refactor(app): close direct model session access"
```

### Task ER1-10：迁移集中测试 fixture

**文件：**

- 修改：`crates/app/src/app_tests.rs`

**步骤：**

1. 将测试中的 `app.workspace`、`app.tab_runtime_store` 直接访问替换为 ER1-3
   提供的测试适配方法。
2. fixture 安装文档时必须构造 `PreparedTab` 并取得稳定 `TabId`。
3. 断言文档内容、runtime 状态时使用窄 query 或测试专用短借用闭包，不能为测试
   把生产字段改成 `pub`。
4. 适配仍委托旧字段，因此本提交独立可编译；下一任务只切换适配实现。

**验证：**

```bash
rg -n 'app\\.(workspace|tab_runtime_store)' crates/app/src/app_tests.rs
cargo test -p textora-app --lib app_tests
cargo check -p textora-app
```

**提交：**

```bash
git add crates/app/src/app_tests.rs
git commit -m "refactor(test): use editor session fixtures"
```

### Task ER1-11：切换 App 的最终 model 所有权

**文件：**

- 修改：`crates/app/src/app.rs`
- 修改：`crates/app/src/app_tab.rs`
- 修改：`crates/app/src/app_init.rs`

**步骤：**

1. `App::new` 使用产品构造的 plugin registry、route table、settings、theme 和
   snapshots path 创建唯一 `EditorRuntime`。
2. 让 ER1-3 的 App 适配方法切换到 runtime 窄 API；尚未迁移的编辑/渲染操作只能
   通过 ER1-1 明确标记的 migration closure 短借用完成。
3. 删除 `App.workspace` 和 `App.tab_runtime_store`，禁止保留镜像或缓存副本。
4. ER1-10 已修正的测试 fixture 继续通过 runtime 安装和查询文档，不重新开放
   内部字段。
5. 删除 ER1-3 的旧字段 allowlist，改为只允许唯一 migration closure 调用点。
6. 增加 source test：
   - `struct App` 不再声明两个旧字段；
   - app 生产代码不能直接调用 shell `Workspace` 的可变入口；
   - model/runtime ID 一致性由 runtime 自己维护。

**验证：**

```bash
cargo test -p textora-app --lib app_tests
cargo test -p textora-appkit-shell editor_runtime
cargo check -p textora-app
bash scripts/check_architecture.sh
```

**提交：**

```bash
git add crates/app/src/app.rs crates/app/src/app_tab.rs crates/app/src/app_init.rs
git commit -m "refactor(app): transfer tab session ownership to runtime"
```

**ER1 完成条件：**

- `App` 不直接持有 `Workspace` 或 `TabRuntimeStore`；
- 所有安装、激活、关闭和 preview reconciliation 由 runtime 原子完成；
- 产品只用稳定 `TabId` 和只读 summary；
- textora 的恢复、切换、批量关闭、pin 和持久化测试全部通过。

---

## ER2：迁移编辑输入、IME、焦点和通知

### Task ER2-1：建立 EditorInputSession

**文件：**

- 新增：`crates/appkit-shell/src/editor_runtime/input_session.rs`
- 修改：`crates/appkit-shell/src/editor_runtime/mod.rs`
- 修改：`crates/appkit-shell/src/mouse_state.rs`

**步骤：**

1. 把 modifiers、editor mouse/drag、IME preedit、cursor blink 和 WYSIWYG
   输入会话收进私有 `EditorInputSession`。
2. `MouseState` 增加明确的 capture query，区分：
   - 未捕获；
   - 文本选择拖拽；
   - canvas/plugin 拖拽。
3. 先写状态机测试：
   - 非 Editor focus 拒绝键盘和 IME；
   - modal blocked 结束 hover 但不消费产品点击；
   - drag capture 期间 rect 外 move/up 继续路由；
   - focus loss 取消或结束需要取消的会话；
   - IME disable 清空 preedit。
4. 用 enum 表示互斥捕获状态，不组合多个 bool。
5. 在 runtime 内实现 product-neutral 的键盘/IME/指针门和通用编辑命令执行；
   产品命令、search/modal 翻译和 dirty snapshot 命名仍留给 app，通过输入前置判断
   或 `EditorNotification` 衔接。

**验证：**

```bash
cargo test -p textora-appkit-shell editor_runtime::input_session
cargo test -p textora-appkit-shell mouse_state
```

**提交：**

```bash
git add crates/appkit-shell/src/editor_runtime/input_session.rs \
  crates/appkit-shell/src/editor_runtime/mod.rs \
  crates/appkit-shell/src/mouse_state.rs
git commit -m "feat(runtime): add typed editor input session"
```

### Task ER2-2：迁移键盘、IME 和编辑事务分发

**文件：**

- 修改：`crates/app/src/app_lifecycle.rs`
- 修改：`crates/app/src/dispatch/editor.rs`
- 修改：`crates/app/src/edit_transaction.rs`

**步骤：**

1. 产品层先执行 modal、search、chrome 和产品快捷键路由。
2. 仅将未消费事件与 `EditorInputContext` 交给
   `EditorRuntime::handle_window_event`。
3. 标准编辑与 transaction 成功后统一生成：
   - `ContentChanged { tab_id, content_revision }`；
   - 必要时 `DirtyChanged`；
   - reshape/redraw `ShellEffect`。
4. 禁止产品通过比较编辑前后字符串推断内容变化。
5. IME candidate area 改为从 `active_ime_cursor_rect()` 查询；产品输入框仍使用
   自己的 widget cursor rect。

**验证：**

```bash
cargo test -p textora-app --lib dispatch::editor
cargo test -p textora-app --lib edit_transaction
cargo test -p textora-app --lib app_lifecycle
```

**提交：**

```bash
git add crates/app/src/app_lifecycle.rs \
  crates/app/src/dispatch/editor.rs \
  crates/app/src/edit_transaction.rs
git commit -m "refactor(input): route keyboard and ime through runtime"
```

### Task ER2-3：迁移鼠标、选择和 canvas drag

**文件：**

- 修改：`crates/app/src/dispatch/mouse.rs`
- 修改：`crates/app/src/events.rs`
- 修改：`crates/app/src/mouse.rs`

**步骤：**

1. `events.rs` 只负责产品 hit-test 和 action 翻译。
2. 编辑器区域的 press/move/up、选择、WYSIWYG 命中和 canvas drag 进入 runtime。
3. 坐标统一以传入的 `editor_rect` 为基准，禁止重新读取 `UiShell` 布局。
4. 保留并扩展测试：
   - 非零 rect 的 caret/selection 命中；
   - 产品 chrome 点击不清空或移动编辑器选择；
   - canvas drag 跨出 rect 后只应用一次 transaction；
   - stale generation drag 结果被拒绝。
5. 迁移完成后删除 app 自有 editor mouse state；产品 sidebar/tab drag 状态不动。

**验证：**

```bash
cargo test -p textora-app --lib dispatch::mouse
cargo test -p textora-app --lib events::tests
cargo check -p textora-app
```

**提交：**

```bash
git add crates/app/src/dispatch/mouse.rs crates/app/src/events.rs crates/app/src/mouse.rs
git commit -m "refactor(input): move editor pointer sessions into runtime"
```

### Task ER2-4：迁移 WYSIWYG 输入会话

**文件：**

- 修改：`crates/app/src/dispatch/wysiwyg.rs`
- 修改：`crates/app/src/dispatch/wysiwyg_test.rs`
- 修改：`crates/appkit-shell/src/editor_runtime/input_session.rs`

**步骤：**

1. 把 preferred X、递归保护和 augmentation session 迁入 runtime。
2. Markdown 插件仍由 textora 构造并注入；runtime 只通过通用 plugin trait 和
   typed edit transaction 工作。
3. 保留 Enter/Backspace augmentation、上下移动 sticky X、IME commit 和
   undo/redo 行为。
4. source test 禁止 `appkit-shell` 出现 `textora_markdown` 或具体 Markdown
   类型名。

**验证：**

```bash
cargo test -p textora-app --lib dispatch::wysiwyg
cargo test -p textora-app --lib dispatch::wysiwyg_test
bash scripts/check_architecture.sh
```

**提交：**

```bash
git add crates/app/src/dispatch/wysiwyg.rs \
  crates/app/src/dispatch/wysiwyg_test.rs \
  crates/appkit-shell/src/editor_runtime/input_session.rs
git commit -m "refactor(runtime): own wysiwyg input session"
```

### Task ER2-5：删除 App 输入状态并验证焦点边界

**文件：**

- 修改：`crates/app/src/app.rs`
- 修改：`crates/app/src/app_init.rs`
- 修改：`crates/app/src/app_window.rs`

**步骤：**

1. 从 `App` 删除已迁移的 modifiers、editor mouse、preedit、cursor blink 和
   WYSIWYG 字段。
2. `App` 只保留产品 chrome 自己的焦点/动画状态。
3. `update_ime_cursor_area` 按产品 focus 决定使用 widget rect 还是 runtime query。
4. source test 禁止重新增加已迁移字段。

**验证：**

```bash
cargo test -p textora-app --lib app_window
cargo test -p textora-app --lib app_lifecycle
cargo check -p textora-app
```

**提交：**

```bash
git add crates/app/src/app.rs crates/app/src/app_init.rs crates/app/src/app_window.rs
git commit -m "refactor(app): remove migrated editor input state"
```

**ER2 完成条件：**

- 产品 focus 非 Editor 时键盘和 IME 不修改文档；
- modal blocked 不发生编辑器 fallthrough；
- editor rect 非零偏移时命中与 IME 坐标正确；
- `ContentChanged` 和 `DirtyChanged` revision 准确且不重复；
- `App` 不再持有编辑器专属输入会话字段。

---

## ER3：迁移 reshape、窗口资源与帧 API

### Task ER3-1：抽取 ReshapeSession

**文件：**

- 新增：`crates/appkit-shell/src/editor_runtime/reshape_session.rs`
- 修改：`crates/appkit-shell/src/editor_runtime/mod.rs`
- 修改：`crates/app/src/app_reshape.rs`

**步骤：**

1. 将 shared font system、worker、generation、pending reshapes、ahead debounce
   和 stale result 过滤放入 `ReshapeSession`。
2. editor rect 宽度变化通过语义方法触发 viewport/layout invalidation，不修改
   文档内容或 revision。
3. 先写测试：
   - invalidate 递增 generation 并取消旧任务；
   - 关闭 tab 后迟到结果被忽略；
   - 非零 editor rect 使用自身宽度；
   - zoom 更新 runtime settings snapshot 并产生 reshape effect。
4. app 仅把产品设置变化映射为 runtime settings 更新和持久化 effect。

**验证：**

```bash
cargo test -p textora-appkit-shell editor_runtime::reshape_session
cargo test -p textora-app --lib app_reshape
```

**提交：**

```bash
git add crates/appkit-shell/src/editor_runtime/reshape_session.rs \
  crates/appkit-shell/src/editor_runtime/mod.rs \
  crates/app/src/app_reshape.rs
git commit -m "refactor(runtime): own reshape lifecycle"
```

### Task ER3-2：抽取 RenderSession 和窗口生命周期

**文件：**

- 新增：`crates/appkit-shell/src/editor_runtime/render_session.rs`
- 修改：`crates/appkit-shell/src/editor_runtime/mod.rs`
- 修改：`crates/app/src/app_window.rs`

**步骤：**

1. `RenderSession` 持有 window、GPU、TextState、FrameCache、scale factor、
   pending resize、frame timing、redraw 和 first-present 状态。
2. `EditorRuntime::resume` 接收产品提供的 `WindowAttributes`；runtime 不读取产品
   settings 文件或构造产品路径。
3. 产品在调用 `resume` 前负责：
   - 读取窗口尺寸和位置；
   - 设置标题和产品特有 attributes；
   - 注入 settings/theme snapshot。
4. runtime 负责创建 window/GPU/Text、resize surface、window focus 和 shutdown。
5. `window()` 保持只读；禁止返回 GPU/Text 引用。

**验证：**

```bash
cargo test -p textora-appkit-shell editor_runtime::render_session
cargo test -p textora-app --lib app_window
cargo test -p textora-app --test render_smoke
```

**提交：**

```bash
git add crates/appkit-shell/src/editor_runtime/render_session.rs \
  crates/appkit-shell/src/editor_runtime/mod.rs \
  crates/app/src/app_window.rs
git commit -m "refactor(runtime): own window and render resources"
```

### Task ER3-3：实现 EditorFrame

**文件：**

- 新增：`crates/appkit-shell/src/editor_runtime/editor_frame.rs`
- 修改：`crates/appkit-shell/src/editor_runtime/mod.rs`
- 修改：`crates/appkit-shell/src/paint_backend.rs`

**步骤：**

1. 在无真实 surface 的测试 backend 中先写调用顺序和消费语义测试。
2. 实现：

   ```rust
   with_layout_context(...)
   with_paint_context(...)
   paint_editor(editor_rect)
   present(self)
   ```

3. `EditorFrame` 私有持有 surface texture、draw list 和短借用 context。
4. layout/paint context 只能存在于闭包调用期间；不能存入产品状态。
5. `paint_editor` 验证 rect 有限且非负；零尺寸时安全跳过绘制但仍允许产品 frame
   完成。
6. `present(self)` 只提交一次，并返回 reshape/redraw 等 `EditorOutcome`。

**验证：**

```bash
cargo test -p textora-appkit-shell editor_runtime::editor_frame
cargo test -p textora-appkit-shell paint_backend
```

**提交：**

```bash
git add crates/appkit-shell/src/editor_runtime/editor_frame.rs \
  crates/appkit-shell/src/editor_runtime/mod.rs \
  crates/appkit-shell/src/paint_backend.rs
git commit -m "feat(runtime): add composable editor frame"
```

### Task ER3-4：把编辑器绘制主体迁入 runtime

**文件：**

- 修改：`crates/app/src/app_renderer.rs`
- 修改：`crates/appkit-shell/src/editor_runtime/editor_frame.rs`
- 修改：`crates/appkit-shell/src/render_pipeline.rs`

**步骤：**

1. 先把 editor-only 绘制与 chrome/overlay 绘制标出明确边界。
2. 将文本、选区、cursor、preedit、plugin/canvas、gutter 和 editor scrollbar
   绘制迁入 `paint_editor`。
3. textora 的 tab/sidebar/status/search/title/settings overlay 继续由 `UiShell`
   生成纯 DrawList，并通过 frame paint context 绘制。
4. 产品帧顺序固定为：

   ```text
   begin_frame
     -> UiShell layout，得到 editor_rect
     -> paint product chrome
     -> paint_editor(editor_rect)
     -> paint product overlay/tooltip
     -> present
   ```

5. 不在 runtime 内读取 `UiShell`，不复制 editor rect 推导公式。
6. 保留 first-frame、redraw-during-frame、scrollbar drag 和 canvas prepare 测试。

**验证：**

```bash
cargo test -p textora-app --lib app_renderer
cargo test -p textora-appkit-shell render_pipeline
cargo test -p textora-app --test render_smoke
```

**提交：**

```bash
git add crates/app/src/app_renderer.rs \
  crates/appkit-shell/src/editor_runtime/editor_frame.rs \
  crates/appkit-shell/src/render_pipeline.rs
git commit -m "refactor(render): compose textora through editor frame"
```

### Task ER3-5：迁移 redraw、resize 和 about-to-wait 调度

**文件：**

- 修改：`crates/app/src/app_lifecycle.rs`
- 修改：`crates/app/src/app_window.rs`
- 修改：`crates/appkit-shell/src/editor_runtime/render_session.rs`

**步骤：**

1. `handle_shell_event` 负责 drain reshape 和 runtime 后台结果。
2. `about_to_wait` 负责 cursor blink、runtime animation、file-safety deadline 和
   redraw 请求；产品仍先轮询 native menu/product wake。
3. 产品动画 deadline 与 runtime deadline 取最早值后设置 `ControlFlow`。
4. resize 节流进入 runtime；改变 presentation 但不改变文档内容。
5. `ApplicationHandler` 保持薄入口和现有 panic boundary。

**验证：**

```bash
cargo test -p textora-app --lib app_lifecycle
cargo test -p textora-app --lib app_window
cargo test -p textora-appkit-shell editor_runtime::render_session
```

**提交：**

```bash
git add crates/app/src/app_lifecycle.rs \
  crates/app/src/app_window.rs \
  crates/appkit-shell/src/editor_runtime/render_session.rs
git commit -m "refactor(runtime): own frame scheduling and resize"
```

### Task ER3-6：删除 App 渲染/reshape 字段并完成阶段验证

**文件：**

- 修改：`crates/app/src/app.rs`
- 修改：`crates/app/src/app_init.rs`
- 修改：`crates/appkit-shell/src/editor_runtime/mod.rs`

**步骤：**

1. 从 `App` 删除 window/GPU/Text/FrameCache/reshape/resize/frame timing 等已迁移字段。
2. `App` 只通过 `window()`、`begin_frame()`、`about_to_wait()` 和语义更新方法操作。
3. 删除旧 re-export 和仅为旧字段存在的 import。
4. source test 禁止 `App` 重新持有 `GpuState`、`TextState`、`ReshapeWorker` 或
   `FrameCache`。

**验证：**

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test -p textora-appkit-shell
cargo test -p textora-app --lib
cargo test -p textora-app --test render_smoke
bash scripts/check_architecture.sh
./scripts/verify.sh
```

**提交：**

```bash
git add crates/app/src/app.rs crates/app/src/app_init.rs \
  crates/appkit-shell/src/editor_runtime/mod.rs
git commit -m "refactor(app): finish render runtime ownership transfer"
```

**ER3 完成条件：**

- runtime 持有 window/GPU/Text/reshape 和帧时序；
- 产品通过 `EditorFrame` 在同一 surface 绘制 chrome、editor 和 overlay；
- editor rect 只有一个产品计算来源；
- resize、DPI、首帧、redraw 和迟到 reshape 测试通过；
- `./scripts/verify.sh` 通过。

---

## ER4：迁移文件安全和异步保存机制

### Task ER4-1：为 DocumentModel 增加保存快照/完成语义

**文件：**

- 修改：`crates/appkit-core/src/document/model.rs`
- 修改：`crates/appkit-core/src/file_safety.rs`

**步骤：**

1. 先写失败测试：
   - 序列化快照保留 CRLF 和 BOM；
   - save completion revision 匹配时清 dirty；
   - completion 期间继续编辑时保持 dirty；
   - 新路径保存成功后 path/revision 正确更新；
   - stale completion 不调用 `mark_as_clean`。
2. 将当前私有序列化逻辑收敛为语义化不可变快照方法，不暴露 `TextBuffer` 可变引用。
3. 增加应用写盘结果的原子方法，集中更新 path、disk revision、dirty 和
   TextBuffer clean baseline。
4. 复用 `save_file_if_unchanged`/`DiskRevision`，不再实现第二套并发修改判断。

**验证：**

```bash
cargo test -p textora-appkit-core document::model
cargo test -p textora-appkit-core file_safety
```

**提交：**

```bash
git add crates/appkit-core/src/document/model.rs crates/appkit-core/src/file_safety.rs
git commit -m "feat(appkit-core): support revisioned save snapshots"
```

### Task ER4-2：实现共享异步保存协议

**文件：**

- 新增：`crates/appkit-shell/src/editor_runtime/document_save.rs`
- 修改：`crates/appkit-shell/src/editor_runtime/mod.rs`
- 修改：`crates/appkit-shell/src/editor_runtime/contract.rs`

**步骤：**

1. 定义 `PreparedDocumentSave`、`SaveCompletion`、`SavePrepareError`。
2. `prepare_save(tab_id)` 从当前文档生成独立 `Vec<u8>`、path、expected disk
   revision 和 content revision。
3. 提供可在产品 worker/thread pool 中调用的共享执行函数；函数不借用 runtime，
   不阻塞 UI 线程。
4. `apply_save_completion`：
   - tab 已关闭时安全忽略；
   - 成功时始终记录 worker 实际 disk revision；
   - 仅 revision 一致时清 dirty；
   - revision 不一致时保持 dirty；
   - `ConcurrentModification` 发送 `SaveFailed`，不覆盖；
   - 成功/失败通知携带稳定 `TabId` 和 revision。
5. 使用临时目录写真实竞态测试，不 mock 掉 revision 判断。

**验证：**

```bash
cargo test -p textora-appkit-shell editor_runtime::document_save
cargo test -p textora-appkit-core file_safety
```

**提交：**

```bash
git add crates/appkit-shell/src/editor_runtime/document_save.rs \
  crates/appkit-shell/src/editor_runtime/mod.rs \
  crates/appkit-shell/src/editor_runtime/contract.rs
git commit -m "feat(runtime): add asynchronous save protocol"
```

### Task ER4-3：保持 textora 手动保存策略

**文件：**

- 修改：`crates/app/src/dispatch/commands.rs`
- 修改：`crates/app/src/dispatch/tabs.rs`
- 修改：`crates/app/src/app_lifecycle.rs`

**步骤：**

1. Save/Save As 和关闭提示的产品交互仍留在 textora。
2. 产品选择目标路径后调用 runtime prepare；后台执行保存并通过 `ShellEvent`
   唤醒，UI 线程只 apply completion。
3. 批量关闭必须按稳定 `TabId` 追踪待保存项；每项保存成功后才
   `confirm_close(Saved)`。
4. 保存失败保持 tab 打开和 dirty；Cancel 不启动保存；Discard 不写盘。
5. 不增加自动保存 timer，证明 runtime 没有改变 textora 策略。

**验证：**

```bash
cargo test -p textora-app --lib dispatch::commands
cargo test -p textora-app --lib dispatch::tabs
cargo test -p textora-app --lib app_lifecycle
```

**提交：**

```bash
git add crates/app/src/dispatch/commands.rs \
  crates/app/src/dispatch/tabs.rs \
  crates/app/src/app_lifecycle.rs
git commit -m "refactor(app): use runtime save protocol"
```

### Task ER4-4：抽取 FileSafetySession

**文件：**

- 新增：`crates/appkit-shell/src/editor_runtime/file_safety_session.rs`
- 修改：`crates/appkit-shell/src/editor_runtime/mod.rs`
- 修改：`crates/app/src/app_lifecycle.rs`

**步骤：**

1. 将 tracked paths、pending requests、request ID、next-check deadline 和
   `FileSafetyWorker` 放入 session。
2. file-safety command/result 关联从 path 优先改为稳定 `TabId + path +
   content_revision` 校验。
3. 外部 reload/conflict/rename/delete 应用后生成类型化通知；产品负责将通知
   映射为 textora status、history、monitor roots 和持久化 effect。
4. `LibraryFileMonitor` 暂留 textora 产品层，只负责加速唤醒；通用一致性逻辑不能
   留在产品 monitor 中。
5. 保留竞态测试：
   - 等待结果期间内容变化；
   - tab 关闭；
   - path 改变；
   - conflict copy；
   - ambiguous rename；
   - clean reload。

**验证：**

```bash
cargo test -p textora-appkit-shell editor_runtime::file_safety_session
cargo test -p textora-app --lib file_safety_race_tests
cargo test -p textora-appkit-core file_safety
```

**提交：**

```bash
git add crates/appkit-shell/src/editor_runtime/file_safety_session.rs \
  crates/appkit-shell/src/editor_runtime/mod.rs \
  crates/app/src/app_lifecycle.rs
git commit -m "refactor(runtime): own file safety session"
```

### Task ER4-5：删除 App 文件安全和同步保存状态

**文件：**

- 修改：`crates/app/src/app.rs`
- 修改：`crates/app/src/app_init.rs`
- 修改：`crates/app/src/app_renderer.rs`

**步骤：**

1. 删除 App 中 worker、tracked/pending/request ID/deadline 字段。
2. renderer 的状态标签从产品保存的 notification/view model 读取，不访问 runtime
   内部集合。
3. 删除 UI 线程上的 `DocumentModel::save/save_as` 调用；保留 core API 只供兼容
   测试，生产 app 必须走 prepare/execute/apply。
4. source test 阻止同步保存调用重新进入 `dispatch` 或 lifecycle。

**验证：**

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test -p textora-appkit-core
cargo test -p textora-appkit-shell
cargo test -p textora-app --lib
bash scripts/check_architecture.sh
./scripts/verify.sh
```

**提交：**

```bash
git add crates/app/src/app.rs crates/app/src/app_init.rs crates/app/src/app_renderer.rs
git commit -m "refactor(app): remove migrated file safety state"
```

**ER4 完成条件：**

- UI 线程不执行文档写盘；
- 保存期间继续编辑不会错误清 dirty；
- 外部修改不会被自动覆盖；
- 关闭后迟到 save/file-safety 结果安全忽略；
- textora 仍只按原手动保存策略保存；
- `./scripts/verify.sh` 通过。

---

## ER5：第二消费者证明、清理和最终验收

### Task ER5-1：建立不依赖 textora-app 的假产品

**文件：**

- 新增：`crates/appkit-shell/tests/editor_runtime_fake_product.rs`
- 修改：`crates/appkit-shell/Cargo.toml`

**步骤：**

1. 在集成测试中定义最小 fake product：
   - 自己构造 plugin registry 和 route table；
   - 只注册通用纯文本 editor plugin；
   - 自己计算带左/上偏移的 editor rect；
   - 自己维护 `FocusTarget` 和 notification 列表；
   - 不导入 `textora_app`、Markdown、sync 或 `UiShell`。
2. 完整测试：
   - 创建 runtime；
   - 安装 `PreparedTab`；
   - 激活、输入、reshape；
   - 在非零偏移 rect layout/paint；
   - prepare/execute/apply save；
   - request/confirm close；
   - 迟到结果 no-op。
3. headless 环境不创建真实窗口时使用测试 render backend；真实 surface smoke
   继续由 textora render smoke 覆盖。

**验证：**

```bash
cargo test -p textora-appkit-shell --test editor_runtime_fake_product
cargo tree -p textora-appkit-shell
bash scripts/check_architecture.sh
```

**提交：**

```bash
git add crates/appkit-shell/tests/editor_runtime_fake_product.rs \
  crates/appkit-shell/Cargo.toml
git commit -m "test(runtime): prove a product-neutral editor host"
```

### Task ER5-2：收窄公共 API 并删除迁移接口

**文件：**

- 修改：`crates/appkit-shell/src/editor_runtime/mod.rs`
- 修改：`crates/appkit-shell/src/lib.rs`
- 修改：`crates/app/src/lib.rs`

**步骤：**

1. 只公开设计中的构造、生命周期、frame、save 和 query API。
2. 私有化所有 session struct 和内部集合。
3. 删除迁移期 re-export、兼容别名、直接 session access 和已废弃 App 模块入口。
   明确删除 ER1-1 的 `with_model_session_for_migration`，最终产物不得保留
   `migration` 命名的运行时入口。
4. 执行 source/API 审查，确认不存在：
   - `workspace_mut`；
   - `tab_runtime_store_mut`；
   - `document_mut`；
   - `gpu_mut`；
   - `Deref<Target = Workspace>`；
   - 字符串 action 或产品类型。
5. `window()` 只返回不可变引用；需要新能力时增加语义命令或 query。

**验证：**

```bash
rg -n 'workspace_mut|tab_runtime_store_mut|document_mut|gpu_mut|Deref.*Workspace' \
  crates/appkit-shell/src/editor_runtime crates/app/src
rg -n 'textora_markdown|textora_sync|notora|TextoraProduct|NoteId' \
  crates/appkit-shell
cargo test -p textora-appkit-shell
cargo check -p textora-app
bash scripts/check_architecture.sh
```

**提交：**

```bash
git add crates/appkit-shell/src/editor_runtime/mod.rs \
  crates/appkit-shell/src/lib.rs crates/app/src/lib.rs
git commit -m "refactor(runtime): close the public editor facade"
```

### Task ER5-3：最终自动化验收

**文件：**

- 修改：`crates/app/tests/public_api.rs`
- 修改：`crates/app/tests/smoke.rs`
- 修改：`crates/appkit-shell/tests/editor_runtime_fake_product.rs`

**步骤：**

1. public API 测试确认 `textora` binary 和原 `App` 入口仍存在。
2. smoke 测试覆盖启动、恢复、打开、编辑、保存、关闭和 shutdown。
3. fake product 测试覆盖任意矩形、焦点门、类型化通知和异步保存竞态。
4. 运行全部架构、格式化、clippy、测试和 workspace 编译。

**验证：**

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p textora-appkit-core
cargo test -p textora-appkit-shell
cargo test -p textora-app --lib
cargo test -p textora-app --tests
bash scripts/check_architecture.sh
./scripts/verify.sh
```

**提交：**

```bash
git add crates/app/tests/public_api.rs crates/app/tests/smoke.rs \
  crates/appkit-shell/tests/editor_runtime_fake_product.rs
git commit -m "test(runtime): close editor extraction acceptance"
```

### Task ER5-4：手工回归并记录结果

**文件：**

- 修改：`docs/manual_test_protocol.md`
- 新增：`docs/plans/2026-07-30-minimal-editor-runtime-manual-results.md`

**步骤：**

1. 更新手工协议中的当前包名和命令，但不删除历史阶段记录。
2. 按协议回归：
   - 启动、窗口移动/resize/DPI/最小化恢复；
   - 打开、最近文件、typed untitled、workspace 恢复；
   - 切换、preview/persistent、pin、批量关闭；
   - 普通文本输入、选择、剪贴板、undo/redo；
   - IME preedit/commit/candidate area；
   - Markdown WYSIWYG toggle 和增强编辑；
   - Mindmap 显示、编辑、拖拽和样式面板；
   - 手动 Save/Save As、外部修改冲突、关闭提示；
   - 设置、同步、native menu 和 sidebar/tabs 模式。
3. 结果文档记录日期、平台、DPI、命令、通过项和失败项；失败项必须修复并重跑，
   不能只在文档中豁免。

**验证：**

```bash
./scripts/verify.sh
```

**提交：**

```bash
git add docs/manual_test_protocol.md \
  docs/plans/2026-07-30-minimal-editor-runtime-manual-results.md
git commit -m "docs(runtime): record editor extraction acceptance"
```

## 7. 每阶段统一审查清单

每个阶段合并前逐项检查：

- [ ] 本阶段每个任务修改不超过 3 个逻辑文件。
- [ ] 行为变化先有 RED 测试，纯移动前后使用同组测试。
- [ ] `cargo fmt --all -- --check` 通过。
- [ ] 相关 crate 测试通过。
- [ ] `cargo check -p textora-app` 通过。
- [ ] `bash scripts/check_architecture.sh` 通过。
- [ ] 没有新 `.unwrap()`；确定不 panic 的位置使用带原因的 `expect`。
- [ ] 没有 `data/info/temp/res/flag` 等宽泛新命名。
- [ ] 没有魔法 timeout、rect padding 或状态字符串。
- [ ] 没有死代码、多余注释、未使用 import 或迁移兼容层遗留。
- [ ] `ui` 只接收纯数据输入，没有依赖 `DocumentView`、Workspace、Commands 或 Events。
- [ ] runtime 没有产品路径、产品 ID、产品保存策略或具体产品插件注册。
- [ ] `TabId` 是唯一跨层运行时关联键。
- [ ] 所有后台结果都校验 `TabId + revision/path/generation` 后再应用。

## 8. 最终完成定义

只有全部满足时才完成：

1. textora 使用 `EditorRuntime`，不再直接持有已迁移的编辑器状态；
2. `EditorRuntime` 公共 API 不暴露内部可变引用或集合；
3. 产品 chrome、modal、焦点、命中和编辑器输入边界有自动化测试；
4. 产品与编辑器通过同一个 `EditorFrame` 绘制到同一 surface；
5. fake product 不依赖 `textora-app`，可在任意矩形完成打开、编辑、reshape、
   保存和关闭；
6. 保存策略属于产品，保存机制属于 runtime；
7. 外部修改、保存竞态、迟到 reshape/save/file-safety 结果均安全；
8. shared crates 不含 textora/notora 产品语义；
9. textora 的用户行为、快捷键、二进制名和持久化格式不变；
10. `./scripts/verify.sh` 与手工回归全部通过；
11. 完成上述验收后，才允许开始创建 notora 产品 crate。
