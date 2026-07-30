# Syncthing UI 适配与剩余安全硬化实施计划

> 本计划执行已确认的 [Syncthing UI Adapter 设计](/Users/dan/.codex/worktrees/2eef/edit+/docs/superpowers/specs/2026-07-18-syncthing-ui-adapter-design.md)，完成后继续执行既有 Phase 4 文件安全计划中尚未落地的硬化项。

## 目标

把现有 headless `SyncController` 接入重构后的纯 UI widget，提供可操作的同步面板，同时保持 UI/app 分层、API Key 安全和编辑器事件隔离。随后将当前 app 内的磁盘版本与单文件安全 worker 收敛到 core/资料库级模型，补齐启动期间 dirty snapshot 对账。

## 不变约束

- `crates/ui` 只依赖纯输入结构和纯用户 action，不依赖 `SyncController`、`LibraryRecord`、`DocumentView` 或 REST DTO。
- API Key 只存在于面板输入控件和 controller 调用参数，不进入 `SyncPanelInput`、Debug、日志或 notice 文本。
- UI 线程不执行 HTTP、Keychain、目录扫描、文件读取或哈希。
- `PublishLibrary` 与 `AcceptRemoteLibrary` 的目录选择由 app 的 `rfd` 完成；取消选择不得产生 controller 命令。
- 移除映射/注销只调用显式 controller 命令，永不删除本地文件。
- 每个子任务先写针对性测试，再写实现；每个子任务完成后至少运行定向测试和 `cargo check -p textora-app`。
- 保留当前工作区中已有但未提交的同步控制面改动，不使用 reset、checkout 或清理命令。

## 阶段一：同步面板 UI 与 app 适配

### 子任务 1：建立纯 UI 数据契约

文件：

- Create `crates/ui/src/widgets/sync_panel.rs`
- Modify `crates/ui/src/widgets/mod.rs`
- Modify `crates/ui/src/lib.rs`

步骤：

1. 在 `sync_panel.rs` 的测试中先覆盖 `SyncConnectionView`、`LibrarySyncState`、`PendingFolderView`、`SyncNoticeView` 和 `SyncPanelInput` 的默认/构造约束；测试确认输入中不存在 API Key 字段。
2. 定义设计文档中的纯数据枚举与结构，并为 UI 使用的字符串字段实现必要的 `Clone`、`Debug`、`PartialEq`。
3. 从 `widgets` 模块和 UI 根模块导出 `sync_panel`，不引入任何 app crate。
4. 运行：`cargo test -p textora-ui --lib -- sync_panel`，再运行 `cargo check -p textora-ui`。

提交：`feat(ui): define pure Syncthing panel contract`

### 子任务 2：实现面板 widget 与安全文本输入

先完成密码输入能力：

- Modify `crates/ui/src/widgets/text_box.rs`
- Modify `crates/ui/src/widgets/sync_panel.rs`

1. 先写 `TextBox` 密码模式测试：内部 `text()` 返回真实值，绘制命令只出现掩码字符，关闭密码模式恢复普通显示。
2. 增加语义化的 `password_mode` 和 `set_password_mode`，不复制或记录 API Key。
3. 在 `SyncPanelWidget` 内组合 endpoint、API Key、远端 Device ID、远端名称和远端地址输入框；输入状态只留在 widget 内部，`SyncPanelInput` 更新时不得覆盖正在编辑的敏感字段。

再完成 widget action 路径：

- Modify `crates/ui/src/core/widget.rs`
- Modify `crates/ui/src/widgets/sync_panel.rs`

1. 先写失败测试：点击各按钮产生准确的 `SyncPanelAction`，列表索引稳定，Escape 产生 `Close`，面板外部区域不产生业务 action。
2. 增加 `WidgetAction::SyncPanel(SyncPanelAction)`，实现 `SyncPanelWidget` 的 `Widget` trait、布局、绘制、命中和键盘/IME 路由。
3. 对扫描、暂停/恢复、修复、移除映射、注销等按钮根据 `LibraryView` 能力字段禁用；注销/移除只发 action，不在 UI 里确认或操作文件。
4. 运行：`cargo test -p textora-ui --lib -- text_box::tests`、`cargo test -p textora-ui --lib -- sync_panel`、`cargo check -p textora-ui`。

提交：`feat(ui): implement Syncthing panel widget`

### 子任务 3：扩展 controller 快照并建立 ViewModel 映射

文件：

- Modify `crates/app/src/sync_controller.rs`
- Create `crates/app/src/sync_view_model.rs`
- Modify `crates/app/src/lib.rs`

1. 先写 `sync_view_model` 映射测试：覆盖未配置、连接中、已连接、认证失败、版本不兼容、不可用；覆盖资料库各注册阶段/暂停/错误/配置漂移；覆盖 pending folder 和 remote event notice。
2. 扩展 `SyncControllerSnapshot`，以纯 app 数据提供 endpoint、是否存在 API Key、pending folders 和当前 libraries；pending folder 由后台 service 异步刷新，不能在 renderer 直接发 REST 请求。
3. 增加不持久化配置的 `TestConnection` controller 命令；`ConfigureConnection` 继续负责 Keychain/metadata 持久化。错误只转换为稳定中文消息，不把原始 HTTP body 带进 ViewModel。
4. 实现 `sync_view_model.rs`：将 `SyncConnectionState::Incompatible` 等全部状态映射到 UI 枚举，将 `LibraryRecord` 映射为 `LibraryView`，将 notice 映射为稳定的 `SyncNoticeView`；不得把 API Key 放进返回值或 `Debug` 内容。
5. 运行：`cargo test -p textora-app --lib -- sync_view_model`、`cargo test -p textora-app --lib -- sync_controller`、`cargo check -p textora-app`。

提交：`feat(app): expose Syncthing panel view model`

### 子任务 4：增加同步面板入口与 action 翻译

先改 UI 菜单，保持小范围：

- Modify `crates/ui/src/widgets/popup_menu/types.rs`
- Modify `crates/ui/src/widgets/sidebar/menu.rs`

1. 先写菜单测试，确认 Sidebar 设置菜单出现“打开同步面板”。
2. 增加 `PopupMenuAction::OpenSyncPanel` 并从菜单返回该 action，不访问 app 状态。
3. 运行：`cargo test -p textora-ui --lib -- sidebar::menu`、`cargo check -p textora-ui`。

再接入 app action：

- Modify `crates/app/src/actions.rs`
- Modify `crates/app/src/events.rs`
- Modify `crates/app/src/app_dispatch.rs`

1. 先写翻译测试，覆盖 `OpenSyncPanel`、`Close`、连接配置/测试、发布/接收和每个 library index action。
2. 增加 `AppAction::OpenSyncPanel`、`AppAction::SyncPanelAction`，让 `events.rs` 只做 WidgetAction → AppAction 的纯翻译。
3. 在 `reduce_action` 中接入 app 层同步 action；目录选择与 controller 调用留在后续副作用子任务。
4. 运行：`cargo test -p textora-app --lib -- translate_sync_panel_action`、`cargo check -p textora-app`。

提交：`feat(app): route Syncthing panel actions`

### 子任务 5：把面板作为 modal overlay 接入 UiShell

先完成 overlay 生命周期：

- Modify `crates/app/src/ui_shell.rs`
- Modify `crates/app/src/dispatch/chrome.rs`

1. 先写 UiShell 测试：面板固定在右侧、不改变 `dock.fill_rect`，打开时优先接收事件，外部 MouseDown 不落到 editor，Escape 可关闭。
2. 为 overlay 增加 modal outside-click 策略；保留现有 popup overlay API 行为不变。
3. 增加 `WidgetId::SYNC_PANEL`、panel input 注入、布局和 downcast 更新；同步面板使用固定语义宽度常量并按 DPI/屏幕高度布局。
4. 扩展 `forward_key`/`forward_ime`，使焦点在 panel 时转发到 overlay，而不是只支持 SearchBar。

再接入渲染与焦点：

- Modify `crates/app/src/app_renderer.rs`
- Modify `crates/app/src/app_lifecycle.rs`

1. renderer 每帧从 `SyncControllerSnapshot`/notices 构造 ViewModel 并注入已存在的 panel；不在 render 中创建网络任务。
2. lifecycle 对 panel 的键盘/IME action 做通用 widget 翻译；MouseDown 命中 panel 时设置 `KeyboardFocusTarget::Widget(SYNC_PANEL)`。
3. 运行：`cargo test -p textora-app --lib -- ui_shell::tests::sync_panel`、`cargo test -p textora-app --lib -- sync_panel`、`cargo check -p textora-app`。

提交：`feat(app): render Syncthing panel as modal overlay`

### 子任务 6：接入目录选择与 controller 副作用

文件：

- Modify `crates/app/src/app_dispatch.rs`
- Modify `crates/app/src/dispatch/commands.rs`
- Modify `crates/app/src/app.rs`

1. 先写 action 测试：取消发布/接收目录选择无副作用；非法 endpoint/API Key 只生成 notice；index 越界不发送命令；remove mapping 不触碰磁盘；unregister 仅调用 controller。
2. 配置/测试连接时将 endpoint 解析为 `LoopbackEndpoint`，API Key 交给 controller，app 不持久化副本。
3. 发布 action 打开目录选择器并将远端字符串解析为 `RemoteDeviceSpec`；接收 action 根据 pending index 选择空目录，再调用 `accept_remote_library`。
4. 对扫描、暂停/恢复、repair、移除映射、注销按 library id 映射，任何过期 index 都安全忽略并请求刷新。
5. 运行：`cargo test -p textora-app --lib -- sync_panel_action`、`cargo check -p textora-app`。

提交：`feat(app): execute Syncthing panel commands safely`

### 子任务 7：阶段一验收

- 运行 `cargo fmt --all -- --check`。
- 运行 `cargo test -p textora-ui --lib -- sync_panel`。
- 运行 `cargo test -p textora-app --lib -- sync_panel`。
- 运行 `cargo check -p textora-app`。
- 检查 `rg -n "api_key|new_api_key" crates/ui/src crates/app/src/sync_view_model.rs`，确认 UI 输入模型、绘制和 ViewModel 没有 API Key 持久字段或日志输出。
- 不修复与本阶段无关的既有 `crates/ui/src/widgets/sidebar/state.rs` clippy baseline；最后统一由 `./scripts/verify.sh` 记录该基线。

## 阶段二：继续既有 Phase 4 文件安全硬化

阶段一验收通过后，按现有 `docs/superpowers/plans/2026-07-17-syncthing-phase4-file-safety.md` 执行以下未完成任务；每项仍按“测试先失败 → 实现 → 定向测试 → compile”推进。

### 子任务 8A：在 core 建立 DiskRevision 与原子保存

文件：

- Create `crates/core/src/disk_revision.rs`
- Modify `crates/core/src/file.rs`
- Modify `crates/core/src/lib.rs`

补齐同 mtime/size 内容变化、inode 替换、删除、目录错误和 `save_file_if_unchanged` 测试；app 只保留 orchestration，不重复实现 hash/原子保存。

运行：`cargo test -p textora-core --lib -- disk_revision`、`cargo test -p textora-core --lib -- save_file_if_unchanged`、`cargo check -p textora-app`。

### 子任务 8B：迁移 app 的 revision 与保存边界

文件：

- Modify `crates/app/src/file_safety.rs`
- Modify `crates/app/src/document_view/mod.rs`

把 app 的 `DiskRevision`/保存调用改为使用 core 类型；先运行现有 file safety 与 document save 测试，再运行 `cargo check -p textora-app`。

### 子任务 9A：建立资料库级文件监控器

文件：

- Create `crates/app/src/library_file_monitor.rs`
- Modify `crates/app/src/app.rs`
- Modify `crates/app/Cargo.toml`

增加 notify 根目录动态替换、200ms 事件归并、忽略 Textora 临时文件和后台 revision 读取队列。

运行：`cargo test -p textora-app --lib -- library_file_monitor`、`cargo check -p textora-app`。

### 子任务 9B：实现外部变化分类与生命周期接入

文件：

- Create `crates/app/src/external_document_change.rs`
- Modify `crates/app/src/app_lifecycle.rs`

增加纯分类结果并覆盖 clean reload、dirty conflict、delete recovery、明确 rename 与歧义 rename；接入生命周期时保留 cursor/selection/scroll clamp 和 dirty buffer 保护。

运行：`cargo test -p textora-app --lib -- classify_external_change`、`cargo test -p textora-app --lib -- external_change`、`cargo check -p textora-app`。

### 子任务 10：dirty snapshot 离线对账

文件：

- Modify `crates/app/src/dirty_snapshot.rs`
- Modify `crates/app/src/app_lifecycle.rs`
- Modify `crates/app/src/external_change_tests.rs`

持久化可向前兼容的基线 revision；启动时先恢复 buffer，再后台比较当前磁盘版本；外部修改生成 conflict copy，删除则转为未命名 dirty recovery；旧 snapshot 缺 revision 时采用保守恢复，不自动写回。

运行：`cargo test -p textora-app --lib -- startup_external_change`、`cargo test -p textora-app --lib -- dirty_external_conflict`、`cargo check -p textora-app`。

### 子任务 11A：切换 app 到资料库级监控

文件：

- Modify `crates/app/src/app_init.rs`
- Modify `crates/app/src/app.rs`
- Modify `crates/app/src/app_lifecycle.rs`

先确认普通文件和同步资料库都由新 monitor 覆盖，再停止旧 2 秒 polling；运行外部变化回归和 `cargo check -p textora-app`。

### 子任务 11B：删除旧 watcher 并做全量验证

文件：

- Delete `crates/app/src/file_watcher.rs`（仅在新监控回归测试通过后）

检查 `rg -n "FileWatcher|poll_external" crates/app/src` 无结果。

最终运行：

- `cargo fmt --all -- --check`
- `cargo test --workspace`
- `cargo clippy -p textora-sync --all-targets -- -D warnings`
- `cargo clippy -p textora-app --all-targets --no-deps -- -D warnings`
- `./scripts/verify.sh`（记录已知 UI baseline，如仍存在）

## 执行顺序

严格按子任务 1 → 11B 执行。阶段一每个子任务完成后停在编译检查点；阶段二不得跳过 core revision 与分类测试直接删除旧 watcher。除非测试暴露设计缺陷，否则不修改已确认规格中的 action 边界。
