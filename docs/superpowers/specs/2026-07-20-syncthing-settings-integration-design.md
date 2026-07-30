# Syncthing 设置页整合设计

## 1. 目标

将 Textora 的设置浮层恢复为唯一同步功能入口，在现有“外观 / 编辑器 / 界面”之外增加第四个“同步”分类，并把当前独立 `SyncPanelWidget` 的全部功能迁入该分类。

迁移完成后，用户通过侧栏设置按钮进入唯一的设置浮层，在“同步”分类内完成：

- 本机 Syncthing loopback 地址配置。
- API Key 输入、连接测试和保存连接。
- 连接状态、Syncthing 版本和本机 Device ID 查看。
- 发布本地资料库。
- 接收远端 pending 资料库。
- 扫描、暂停/恢复、修复、移除映射和注销资料库。
- 同步 notice 查看。

## 2. 非目标

- 不新增独立设置窗口或第二层同步 Overlay。
- 不改变 `textora-sync` 的 REST 协议、版本范围或工作线程模型。
- 不在本阶段新增“打开 Syncthing Web UI”“断开连接”或复制完整 Device ID 等当前面板尚未具备的功能。
- 不改变资料库发布、接收、移除映射和注销的领域语义。
- 不让 `ui` crate 访问 `SyncController`、Keychain、REST DTO、目录选择器或 app 状态结构体。
- 不顺带重构与同步设置无关的设置页、侧栏或 Overlay 行为。

## 3. 已确认的产品决策

- “设置”是同步功能的唯一用户入口。
- “同步”是 `SettingsView` 的第四个一级分类。
- 现有同步面板的全部功能迁入“同步”分类，不保留独立同步面板。
- 同步内容采用设置页内嵌页面，不通过“打开同步面板”按钮再弹出第二层 UI。
- 设置浮层继续保持单例、模态、窗口内居中显示。
- 同步页面必须支持内容滚动，资料库数量增加时不得越出设置浮层。

## 4. 方案选择

采用独立 `SyncSettingsPage` 内嵌 `SettingsView` 的方案。

未采用的方案：

- 原样嵌入 `SyncPanelWidget`：固定坐标、独立标题和关闭按钮不适合设置页内容区域，也无法可靠承载大量资料库。
- 在“同步”分类中保留“打开同步面板”按钮：仍然形成两层入口，与唯一入口决策冲突。

`SyncSettingsPage` 是设置业务页面，不是通用基础控件。它只组合现有基础控件和 Form 容器，并通过纯数据输入与语义 Action 和 app 通信。

## 5. 总体架构

```text
Sidebar 设置按钮
        │
        ▼
Settings Overlay
  └── ModalFrame
      └── SettingsView
          ├── CategoryNavigation
          │   ├── 外观
          │   ├── 编辑器
          │   ├── 界面
          │   └── 同步
          └── ActivePage
              ├── 普通设置 FormView
              └── SyncSettingsPage
                  └── FormView
                      ├── 连接
                      ├── 发布资料库
                      ├── 待接收资料库
                      ├── 已注册资料库
                      └── Notice
```

app 层数据流：

```text
SyncControllerSnapshot + SyncNotice
                │
                ▼
       app::sync_view_model
                │ SyncSettingsInput
                ▼
         SyncSettingsPage
                │ SyncSettingsAction
                ▼
       SettingsViewAction::Sync
                │
                ▼
   app::dispatch_sync_settings_action
                │
                ▼
 SyncController / rfd / AppEffect
```

## 6. UI 类型与模块边界

### 6.1 纯输入模型

将现有 `sync_panel` 的纯展示类型迁移并重命名到设置语义下：

```rust
pub enum SyncConnectionView {
    NotConfigured,
    Connecting,
    Connected { device_id: String, version: String },
    AuthenticationRequired,
    Incompatible { found: String },
    Unavailable { message: String },
}

pub struct SyncSettingsInput {
    pub endpoint: String,
    pub has_api_key: bool,
    pub connection: SyncConnectionView,
    pub libraries: Vec<LibraryView>,
    pub pending_folders: Vec<PendingFolderView>,
    pub notices: Vec<SyncNoticeView>,
}
```

`LibraryView`、`PendingFolderView`、`LibrarySyncState`、`SyncNoticeView` 和 `SyncNoticeSeverity` 继续保持纯 UI 类型，不携带领域对象或 REST DTO。

### 6.2 用户动作

```rust
pub enum SyncSettingsAction {
    TestConnection {
        endpoint: String,
        api_key: SensitiveText,
    },
    ConfigureConnection {
        endpoint: String,
        api_key: SensitiveText,
    },
    PublishLibrary {
        remote_device_id: String,
        remote_name: String,
        remote_addresses: Vec<String>,
    },
    AcceptRemoteLibrary { pending_index: usize },
    ScanLibrary { library_index: usize },
    SetLibraryPaused { library_index: usize, paused: bool },
    RepairLibrary { library_index: usize },
    RemoveLibraryMapping { library_index: usize },
    UnregisterLibrary { library_index: usize },
}
```

不再需要 `Close` 动作；设置浮层由现有 `ModalFrame` 和 `DismissPolicy` 统一关闭。

`SettingsViewAction` 增加：

```rust
Sync(SyncSettingsAction)
```

API Key 必须使用已有 `SensitiveText`。`SyncSettingsAction` 的 `Debug` 输出只能出现脱敏占位文本，不得包含明文 Key。

### 6.3 文件职责

- `crates/ui/src/widgets/settings_view/types.rs`：设置分类、普通设置输入与顶层 `SettingsViewAction`。
- `crates/ui/src/widgets/settings_view/sync_types.rs`：同步页面纯输入、状态和动作类型。
- `crates/ui/src/widgets/settings_view/sync_page.rs`：同步页面布局、焦点、绘制和控件动作翻译。
- `crates/ui/src/widgets/settings_view/widget.rs`：分类导航和活动页面路由，不解释 Syncthing 领域状态。
- `crates/app/src/sync_view_model.rs`：领域快照到 `SyncSettingsInput` 的映射。
- `crates/app/src/settings_overlay.rs`：创建设置浮层并更新普通设置与同步页面输入。
- `crates/app/src/app_dispatch.rs`：把 `SyncSettingsAction` 转换为 controller、目录选择器和 AppEffect 操作。

## 7. 页面布局与交互

### 7.1 分类导航

分类顺序固定为：

1. 外观
2. 编辑器
3. 界面
4. 同步

切换到“同步”时，右侧内容滚动位置回到顶部，键盘焦点移动到同步页面首个可交互控件。

### 7.2 同步页面分组

同步页面使用现有 `FormView`、`FormSection`、`FormRow`、`InlineGroup`、`Label`、`TextBox` 和 `Button` 组合：

- 连接：endpoint、API Key、测试连接、保存连接、状态、Device ID 和版本。
- 发布资料库：远端 Device ID、远端名称、远端地址和选择目录按钮。
- 待接收资料库：每个 pending folder 显示来源和“选择空目录”按钮。
- 已注册资料库：名称、根目录、状态和可用操作。
- Notice：按当前顺序逐条显示，不得绘制在同一坐标上。

页面纵向滚动，所有动态资料库行都参与内容高度计算。不同 DPI 和窄窗口下依赖 Form 容器现有响应式布局，不使用当前同步面板的固定绝对纵坐标。

### 7.3 编辑状态保护

- app 更新 `SyncSettingsInput` 时，不覆盖正在编辑的 endpoint。
- API Key 永不从 app 回填；只通过 `has_api_key` 生成“已配置”或“未配置”提示。
- API Key 提交后清空输入框。
- 空 API Key 是否允许由当前 controller 规则决定，本阶段不改变既有连接语义。
- 后台状态和 notice 更新不得改变当前分类、表单滚动位置或文本焦点。

## 8. app 接入与状态刷新

设置浮层打开时，app 使用当前 `SyncControllerSnapshot` 构造初始 `SyncSettingsInput`。如果后台服务尚未创建，则注入 `NotConfigured` 的空输入，待服务启动后刷新。

设置浮层存续期间：

1. `AppEvent::SyncResultsReady` 继续由现有 controller drain 逻辑处理。
2. renderer 或现有帧更新边界检测活动设置浮层。
3. app 从 controller snapshot 和待展示 notices 构造新输入。
4. 只更新 `SettingsView` 内的 `SyncSettingsPage`。

同步页面关闭后不为 UI 单独轮询；controller 继续遵循当前后台生命周期。

普通设置更新调用 `refresh_settings_overlay()` 时，只更新普通设置输入和持久化状态，不重建同步页面，不清空敏感输入或焦点。

## 9. 唯一入口与旧路径清理

迁移完成后删除以下可见或内部路径：

- `PopupMenuAction::OpenSyncPanel`。
- `AppAction::OpenSyncPanel` 和独立 `AppAction::SyncPanel`。
- `ChromeDispatchAction::OpenSyncPanel`、`CloseSyncPanel`。
- `UiShell::open_sync_panel`、`set_sync_panel_input`、`close_sync_panel`、`sync_panel_is_open` 和同步面板专用布局。
- 侧栏旧设置菜单中的“打开同步面板”。
- 独立 `SyncPanelWidget` 及 `SYNC_PANEL` WidgetId。

侧栏设置按钮继续只产生现有设置动作并打开唯一 `SettingsView`。原有不可达设置菜单若仍被其他兼容测试引用，只清理同步专用项；删除整个旧菜单不属于本功能范围。

## 10. 错误处理与安全

- endpoint 继续由 app 使用 `LoopbackEndpoint` 校验，拒绝非 loopback 地址。
- Device ID、远端地址和索引校验继续在 app/controller 边界执行。
- 目录选择取消保持无副作用。
- 过期 pending/library index 产生稳定 notice，不 panic。
- 移除映射和注销继续不删除本地文件。
- API Key 不进入 `SyncSettingsInput`、DrawList、notice、日志或普通 `Debug` 输出。
- UI 线程不执行 HTTP、Keychain、目录扫描、文件读取或哈希。

## 11. 测试策略

### 11.1 UI 契约

- `SettingsCategory::Sync` 是第四个分类并可选中。
- 同步输入中不存在 API Key 字段。
- `SyncSettingsAction` 的 Debug 不包含 API Key 明文。
- 独立同步 `Close` 动作不存在。

### 11.2 页面行为

- 切换“同步”后显示连接 FormSection。
- endpoint 和 API Key 输入产生正确语义动作。
- API Key 绘制为掩码，提交后清空。
- snapshot 更新不覆盖正在编辑的 endpoint/API Key。
- pending folder、资料库和多条 notice 形成不同布局行。
- 动态内容超高时可以滚动到底部。
- Tab、键盘和 IME 只进入同步页面当前焦点控件。

### 11.3 app 接线

- 侧栏设置动作打开的 `SettingsView` 包含同步分类。
- 打开设置浮层时注入当前同步快照。
- 同步后台结果刷新嵌入页面，而不是创建独立 overlay。
- `SettingsViewAction::Sync` 复用现有 controller 操作。
- 目录选择取消、非法输入和过期 index 行为保持不变。
- 工作区不存在 `OpenSyncPanel` 的可达动作或专用 overlay 生命周期。

### 11.4 验证命令

每个阶段先运行定向测试和对应编译检查。全部完成后运行：

```bash
cargo fmt --all -- --check
cargo test -p textora-ui
cargo test -p textora-app --lib
cargo check -p textora-app
./scripts/verify.sh
```

## 12. 实施拆分约束

本功能跨越 UI 契约、页面、app 数据流和旧入口清理，必须拆成独立子任务。每个子任务修改不超过 3 个文件，并遵循：

1. 先写失败测试并确认按预期失败。
2. 写最小实现使测试通过。
3. 运行定向测试和 `cargo check -p textora-app`。
4. 编译通过后再提交。
5. 同一问题连续两次修复失败时停止叠加补丁，返回数据流和架构边界重新分析。

## 13. 验收标准

- 侧栏设置按钮是唯一同步入口。
- 设置页显示“外观 / 编辑器 / 界面 / 同步”四个分类。
- “同步”分类完整承载当前同步面板全部能力。
- 不存在独立同步面板或第二层 Overlay。
- 同步状态能在设置页打开期间持续更新。
- 多资料库、多 pending folder 和多 notice 均可滚动查看且不重叠。
- API Key 不通过 ViewModel、绘制、notice、日志或 Debug 泄露。
- `ui` 与 app/sync 领域保持纯数据输入和语义 Action 边界。
- 所有定向测试、crate 测试、编译检查和 `./scripts/verify.sh` 通过。
