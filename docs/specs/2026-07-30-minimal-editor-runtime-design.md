# 最小 EditorRuntime 抽取设计

日期：2026-07-30

状态：方案初稿，待评审

## 1. 背景

textora 已经完成了第一轮应用架构拆分：

- `appkit-core` 持有 `DocumentModel`、`WorkspaceModel`、`TabId`、文件安全和持久化基础能力；
- `appkit-shell` 持有 `Workspace`、`PreparedTab`、`TabRuntimeStore`、插件路由、输入与渲染基础模块；
- `textora-app` 仍持有窗口生命周期、GPU/Text 状态、编辑分发、reshape 调度和产品 UI 组合。

notora 将成为 appkit 的第二个真实产品消费者。它需要复用同一套文本、
Markdown WYSIWYG 和 Mindmap 编辑能力，但采用“左侧导航 + 中间文件卡片 +
右侧编辑器”的产品布局，不能复用当前固定组合旧 Sidebar/TabBar 的 `UiShell`。

如果直接创建 notora 并复制 `textora-app::App`，会产生两套窗口、输入、IME、
reshape、渲染、文件安全和编辑生命周期实现。后续任何编辑器修复都需要同步修改
两个产品，这不是可接受的复用方式。

本设计先抽取一个最小、真实、可嵌入产品布局的 `EditorRuntime`。它只负责编辑器
表面和文档会话，不理解 textora 或 notora 的产品导航、文件卡片、同步、标签、
星标和回收站。

## 2. 目标

完成后，textora 和 notora 都能够：

1. 注入各自的插件注册表和路径路由；
2. 向 runtime 提交已经构造好的 `PreparedTab`；
3. 在产品计算出的任意 `editor_rect` 内布局和绘制编辑器；
4. 将窗口事件按产品焦点和命中结果交给编辑器；
5. 获得类型化编辑通知和通用 `ShellEffect`；
6. 使用相同的窗口、GPU、文本塑形、IME、reshape、文件安全和保存基础能力；
7. 在不暴露 runtime 内部字段的情况下管理打开、激活、关闭和保存。

## 3. 非目标

本阶段不做：

- 不创建 notora binary；
- 不实现三栏布局、笔记目录、索引、搜索、标签、星标或回收站；
- 不把当前 `UiShell` 改造成笔记 UI；
- 不让 `appkit-shell` 依赖 `textora-markdown`、`textora-sync`、`textora-app`
  或未来的 `notora-app`；
- 不设计动态产品加载、字符串动作表或 `Box<dyn Any>` 产品状态袋；
- 不把所有 `App` 字段一次性移动到一个大结构；
- 不改变 textora 当前用户行为、快捷键和持久化格式；
- 不同时升级 `winit`、`wgpu` 或其他基础依赖。

## 4. 设计原则

### 4.1 EditorRuntime 是编辑表面，不是产品 Shell

`EditorRuntime` 负责右侧编辑区域以及支撑它的窗口和渲染资源。产品负责：

- 窗口内的导航、列表、标题栏和设置等 chrome；
- 产品焦点状态；
- 产品动作与产品后台服务；
- 自动保存或手动保存策略；
- 产品级会话和业务持久化。

runtime 只接收产品计算好的编辑区域，不主动创建 Sidebar、TabBar 或三栏布局。

### 4.2 产品策略通过命令表达，不进入共享状态

notora 的“笔记自动保存、外部文件手动保存”属于产品策略。runtime 只提供：

- 内容变更通知；
- 可并发执行的保存快照；
- 保存完成结果应用；
- dirty 和磁盘 revision 的正确状态转移。

runtime 不保存 `is_note`、`auto_save` 等布尔字段，也不认识 `NoteId`。

### 4.3 稳定 ID 是唯一跨层关联键

产品用自己的实体 ID 映射到共享 `TabId`：

```text
notora NoteId / ExternalFileId
              │ product-owned map
              ▼
             TabId
              ├── WorkspaceModel<DocumentModel>
              └── TabRuntimeStore
```

禁止使用 tab index、文件卡片 index 或当帧排序位置关联运行时状态。

### 4.4 产品 UI 先处理事件，编辑器只接收剩余事件

产品必须先处理 modal、左栏、中栏、分隔条和设置 UI。只有满足以下条件时，
事件才进入 `EditorRuntime`：

- 鼠标事件命中 `editor_rect`，或编辑器正在捕获拖拽；
- 键盘焦点属于编辑器；
- IME 目标属于编辑器；
- 事件未被产品 modal 拦截。

这避免 runtime 抢走搜索框、标签重命名或文件卡片快捷键。

## 5. 所有权边界

### 5.1 EditorRuntime 持有

`EditorRuntime` 最终持有以下共享编辑状态：

```text
EditorRuntime
├── Workspace
├── TabRuntimeStore
├── editor settings/theme runtime snapshot
├── editor focus/input session
│   ├── modifiers
│   ├── mouse and drag state
│   ├── IME preedit
│   ├── cursor blink
│   └── WYSIWYG input session
├── render session
│   ├── Window / GPU / TextState
│   ├── FrameCache
│   ├── shared FontSystem
│   ├── ReshapeWorker
│   ├── generation and pending reshapes
│   └── resize/redraw/frame timing
└── file safety session
    ├── tracked paths
    ├── pending checks
    └── FileSafetyWorker
```

窗口与 GPU 由 runtime 持有，是因为产品 chrome 和编辑器最终必须进入同一帧、
同一 DrawList 和同一 surface。产品通过受控的 frame API 绘制自己的 UI，
不能直接取得 `GpuState`、`TextState` 或 reshape 内部集合。

### 5.2 产品持有

textora 或 notora 产品层继续持有：

- `ProductIdentity` 与已经解析的产品路径；
- 产品设置文件和产品会话持久化；
- 产品根布局与 widget 状态；
- textora sync/native menu 或 notora 索引/标签/回收站；
- `TabId` 与产品实体 ID 的映射；
- 插件注册表和路由规则的构造代码；
- 文件加载、typed untitled 和产品默认内容；
- 自动保存定时策略及错误提示；
- winit `ApplicationHandler<ShellEvent>` 的薄组合入口。

### 5.3 UiShell 的位置

当前 `appkit-shell::UiShell` 固定组合了 textora 的现有 chrome。迁移期间它作为
textora 产品适配器继续存在，但不进入 `EditorRuntime`。

长期可以把它重命名或拆成纯 widget 组合，但这不是 notora 的前置条件。
notora 将拥有自己的 `NotoraShell`，两者都把最终 `editor_rect` 交给相同的
`EditorRuntime`。

## 6. 公共类型

### 6.1 构造输入

```rust
pub struct EditorRuntimeConfig {
    pub plugin_registry: ui::plugin::PluginRegistry,
    pub view_routes: ViewRouteTable,
    pub initial_settings: ui::settings::Settings,
    pub initial_theme: ui::theme::Theme,
    pub snapshots_directory: PathBuf,
}
```

约束：

- 所有路径由产品解析后注入；
- runtime 不读取 `$HOME`，不拼接 `.edit+` 或 `.notora`；
- 插件注册表由产品构造，shell 不硬编码 Markdown/Mindmap 插件；
- `ViewRouteTable` 在构造时完成冲突和未知插件校验。

### 6.2 打开位置

```rust
pub enum OpenDisposition {
    Preview,
    Persistent,
}
```

- `Preview` 复用 `Workspace` 已有 preview 生命周期，适合 notora 单击卡片；
- `Persistent` 不被下一次 preview 自动替换，适合外部文件、恢复文档和显式固定；
- 不能使用 `preview: bool`。

### 6.3 编辑器输入门

```rust
pub enum EditorFocus {
    Inactive,
    Active,
}

pub struct EditorInputContext {
    pub editor_rect: ui::Rect,
    pub focus: EditorFocus,
    pub modal_blocked: bool,
}
```

`EditorFocus` 是产品布局与编辑器之间唯一的键盘焦点输入。runtime 不查询产品
widget，也不通过 downcast 判断焦点。

### 6.4 类型化通知

```rust
pub enum EditorNotification {
    ActiveDocumentChanged { tab_id: Option<TabId> },
    ContentChanged { tab_id: TabId, content_revision: u64 },
    PathChanged { tab_id: TabId, path: PathBuf },
    DirtyChanged { tab_id: TabId, dirty: bool },
    SaveCompleted { tab_id: TabId, content_revision: u64 },
    SaveFailed { tab_id: TabId, message: String },
    CloseRequested { tab_id: TabId, decision: CloseTabDecision },
}

pub struct EditorOutcome {
    pub shell_effect: ShellEffect,
    pub notifications: smallvec::SmallVec<[EditorNotification; 4]>,
}
```

`ShellEffect` 继续只表示 reshape、redraw、窗口标题等通用 effect；需要携带
`TabId` 或错误消息的信息必须走类型化 notification，禁止塞进字符串 action。

## 7. 最小 API

建议的语义 API 如下，具体借用形式可在实施计划的编译 spike 中调整：

```rust
impl EditorRuntime {
    pub fn new(config: EditorRuntimeConfig) -> Result<Self, EditorRuntimeError>;

    pub fn resume(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        window_attributes: winit::window::WindowAttributes,
    ) -> Result<EditorOutcome, EditorRuntimeError>;

    pub fn install_prepared_tab(
        &mut self,
        prepared: PreparedTab,
        suggested_file_name: Option<String>,
        disposition: OpenDisposition,
    ) -> EditorOutcome;

    pub fn activate(&mut self, tab_id: TabId) -> EditorOutcome;

    pub fn request_close(&mut self, tab_id: TabId) -> EditorOutcome;

    pub fn confirm_close(
        &mut self,
        tab_id: TabId,
        confirmation: CloseConfirmation,
    ) -> EditorOutcome;

    pub fn handle_window_event(
        &mut self,
        event: &winit::event::WindowEvent,
        input: EditorInputContext,
    ) -> EditorOutcome;

    pub fn handle_shell_event(&mut self, event: ShellEvent) -> EditorOutcome;

    pub fn about_to_wait(&mut self) -> EditorOutcome;

    pub fn begin_frame(&mut self) -> Result<EditorFrame<'_>, RenderError>;

    pub fn prepare_save(&self, tab_id: TabId) -> Result<PreparedDocumentSave, SavePrepareError>;

    pub fn apply_save_completion(&mut self, completion: SaveCompletion) -> EditorOutcome;

    pub fn shutdown(&mut self);
}
```

只读查询使用窄接口：

```rust
pub fn active_tab_id(&self) -> Option<TabId>;
pub fn tab_id_for_path(&self, path: &Path) -> Option<TabId>;
pub fn document_summary(&self, tab_id: TabId) -> Option<EditorDocumentSummary>;
pub fn active_ime_cursor_rect(&self) -> Option<ui::Rect>;
pub fn window(&self) -> Option<&winit::window::Window>;
```

禁止公开：

- `workspace_mut()`；
- `tab_runtime_store_mut()`；
- `gpu_mut()`；
- `document_mut()`；
- 任意返回内部集合可变引用的 getter；
- `Deref<Target = Workspace>`。

需要新增能力时优先增加语义命令或 query。

## 8. 帧与产品布局集成

产品和编辑器必须绘制到同一个 frame，但 runtime 不拥有产品 widget。采用显式
帧对象：

```rust
pub struct EditorFrame<'runtime> {
    // 私有：surface frame、DrawList、PaintCtx、text/gpu references
}

impl EditorFrame<'_> {
    pub fn with_layout_context<T>(
        &mut self,
        layout: impl FnOnce(&mut ui::LayoutCtx<'_>) -> T,
    ) -> T;
    pub fn with_paint_context<T>(
        &mut self,
        paint: impl FnOnce(&mut ui::PaintCtx<'_>) -> T,
    ) -> T;
    pub fn paint_editor(&mut self, editor_rect: ui::Rect) -> Result<(), RenderError>;
    pub fn present(self) -> Result<EditorOutcome, RenderError>;
}
```

产品帧流程：

```text
EditorRuntime::begin_frame
  -> 产品布局并得到 editor_rect
  -> 产品绘制左栏/中栏/顶部 chrome
  -> EditorFrame::paint_editor(editor_rect)
  -> 产品绘制 overlay/tooltip
  -> EditorFrame::present
```

具体绘制顺序允许产品控制，但必须保证 modal 和 tooltip 在编辑器之后绘制。
`EditorFrame` 消费式 `present(self)` 防止同一 surface 被重复提交。

frame 使用闭包提供 `LayoutCtx`/`PaintCtx` 的短借用，禁止把 context 保存到
产品状态，也禁止为绕过借用引入裸指针或扩大 `unsafe`。

## 9. 保存协议

### 9.1 策略与机制分离

runtime 不决定何时保存。产品收到：

```rust
EditorNotification::ContentChanged { tab_id, content_revision }
```

之后：

- textora 按现有手动保存策略处理；
- notora 对笔记设置 800ms idle deadline；
- notora 对外部文件只响应显式保存命令。

### 9.2 非阻塞保存

自动保存不能在 UI 线程同步写盘。runtime 生成不可变快照：

```rust
pub struct PreparedDocumentSave {
    pub tab_id: TabId,
    pub path: PathBuf,
    pub serialized_contents: Vec<u8>,
    pub expected_disk_revision: Option<DiskRevision>,
    pub content_revision: u64,
}
```

保存 worker 写入后返回：

```rust
pub struct SaveCompletion {
    pub tab_id: TabId,
    pub content_revision: u64,
    pub result: Result<DiskRevision, DocumentSaveError>,
}
```

应用完成结果时：

- 磁盘 revision 必须更新为 worker 实际写入后的 revision；
- 仅当当前 `content_revision` 与完成结果一致时清除 dirty；
- 如果保存期间又发生编辑，保持 dirty，并由产品重新安排保存；
- `ConcurrentModification` 不允许自动覆盖；
- 关闭文档后迟到的 completion 必须被安全忽略。

该协议同时服务 textora 和 notora，不包含自动保存策略。

## 10. 输入、IME 与焦点

- 产品维护顶层 `FocusTarget`，例如 `Navigation`、`CardList`、`Editor`、`Overlay`；
- 只有 `FocusTarget::Editor` 映射为 `EditorFocus::Active`；
- IME preedit 只存在 runtime，产品输入框继续由各自 widget 管理；
- 产品需要调用 `active_ime_cursor_rect()` 并设置 OS candidate area；
- modal 打开时传入 `modal_blocked = true`，runtime 必须结束 hover，但不能消费点击；
- 编辑器拖拽捕获期间，即使鼠标移出 `editor_rect`，MouseMove/MouseUp 仍交给 runtime；
- editor rect 改变必须触发 viewport/layout invalidation，但不改变文档模型。

## 11. 错误模型

构造、打开、渲染、保存和关闭使用明确错误类型，不返回宽泛字符串作为内部错误：

```rust
pub enum EditorRuntimeError {
    InvalidRoute(ViewRouteError),
    WindowCreation { message: String },
    GpuInitialization(GpuError),
    FontInitialization { message: String },
}
```

跨 crate 难以保留具体 source 时可以保存 `message`，但调用者必须知道失败发生
在哪个领域。可恢复错误通过 notification 进入产品 UI；不可恢复的启动错误返回
给产品入口，由产品显示并退出。

## 12. 迁移方案

### 阶段 ER0：建立行为基线

- 固定 textora 打开、切换、编辑、保存、关闭、恢复、Markdown toggle、
  Mindmap 和 IME 的现有测试；
- 为自定义 editor rect 建立 render smoke；
- 记录 `App` 字段归属清单，禁止迁移中静默丢失状态。

### 阶段 ER1：模型与 runtime store 收拢

- 新增 `EditorRuntime`，先只持有 `Workspace + TabRuntimeStore`；
- 把 `PreparedTab` 安装、activate、close reconciliation 收进语义方法；
- textora 通过 accessor 使用，不改变窗口和渲染所有权。

### 阶段 ER2：编辑输入会话迁移

- 迁移编辑器专属 mouse/modifier/IME/cursor/WYSIWYG 状态；
- 建立 `EditorInputContext` 和 `EditorOutcome`；
- 产品 chrome 事件继续由 textora 自己分发。

### 阶段 ER3：reshape 与渲染迁移

- 迁移 frame cache、font system、reshape worker、GPU/Text 状态和帧时序；
- 建立 `EditorFrame`；
- textora 使用现有 `UiShell` 计算 editor rect，再调用 runtime 绘制。

### 阶段 ER4：文件安全和异步保存

- 把通用 file-safety session 纳入 runtime；
- 建立 `PreparedDocumentSave / SaveCompletion`；
- 保持 textora 默认仍为手动保存，证明策略未被 runtime 改写。

### 阶段 ER5：第二消费者门槛

- 在 `appkit-shell` 测试中创建不依赖 `textora-app` 的 fake product chrome；
- 注入纯文本测试插件；
- 在非零偏移 editor rect 中完成打开、编辑、reshape、保存和关闭；
- 通过后才开始创建 notora 产品 crate。

每个实施任务最多修改 3 个逻辑文件。具体文件级 RED-GREEN 步骤将在本设计
批准后另写到 `docs/plans/`，不在本设计中把阶段伪装成可直接执行的大提交。

## 13. 兼容与迁移约束

- textora 仍生成原 `textora` binary；
- textora 原 workspace、settings、snapshot 和 history 格式不变；
- appkit shared crates 不硬编码 `.edit+`、`.notora`；
- `appkit-shell` 不依赖任何具体产品或 Markdown 插件；
- 产品路由仍由产品注入，`.mmap.md` 必须优先于 `.md`；
- `DocumentModel` 与 `TabRuntime` 仍只通过 `TabId` 对应；
- 不为迁移方便把内部字段批量设为 `pub`；
- 不使用 `Deref`、字符串 action、全局回调表或任意类型状态袋；
- 现有 `ProductHost` 的 `ProductWake` 继续保持无 payload。

## 14. 测试与验收

### 14.1 单元与契约测试

- `PreparedTab` 安装后 model/runtime 精确使用同一 `TabId`；
- preview 被替换时只关闭对应 runtime；
- persistent tab 不被 preview 替换；
- active/close/reorder 后无孤儿 runtime；
- 产品焦点非 Editor 时键盘事件不修改文档；
- editor rect 非零偏移时鼠标命中、光标和 IME 坐标正确；
- resize 后 presentation 重建但文档内容不变；
- 保存期间继续编辑不会错误清除 dirty；
- 外部修改产生 `ConcurrentModification`；
- 关闭后迟到的 reshape/save result 被忽略；
- fake product 不依赖 textora 产品代码。

### 14.2 架构检查

```bash
cargo tree -p textora-appkit-core
cargo tree -p textora-appkit-shell
rg -n 'textora_markdown|textora_sync|notora|TextoraProduct' crates/appkit-shell
rg -n '\\.edit\\+|\\.notora' crates/appkit-core crates/appkit-shell
bash scripts/check_architecture.sh
```

### 14.3 阶段完成验证

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test -p textora-appkit-core
cargo test -p textora-appkit-shell
cargo test -p textora-app --lib
./scripts/verify.sh
```

并按 `docs/manual_test_protocol.md` 回归 textora 的打开、编辑、保存、恢复、
Markdown WYSIWYG、Mindmap、设置、同步和窗口行为。

## 15. 完成定义

满足以下条件才认为最小 `EditorRuntime` 抽取完成：

1. textora 使用 runtime，而不是继续直接持有被迁移的编辑器内部状态；
2. 一个不依赖 `textora-app` 的测试宿主能把编辑器绘制在任意矩形；
3. 产品 chrome 和编辑器的事件、焦点与绘制边界明确；
4. 产品能够通过 notification 实现不同保存策略；
5. shared crates 不出现 textora/notora 产品语义；
6. textora 行为和持久化兼容测试全部通过；
7. `./scripts/verify.sh` 通过。
