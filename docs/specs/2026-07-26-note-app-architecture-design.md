# textora appkit 架构拆分设计

日期：2026-07-26
状态：已收敛范围，待执行

> 文件名保留最初的笔记 App 方案名称，本文范围已经收敛为 **仅拆分 textora 架构**。本次不创建笔记 App、不增加笔记功能，也不承诺第二个产品的交付时间。

## 1. 目标与范围

本次工作的唯一目标是把现有 `crates/app` 拆成可解释、可验证的三层：

1. 无窗口的应用模型与持久化能力；
2. 通用编辑器窗口、输入与渲染运行时；
3. textora 产品组合与产品服务。

拆分完成后只保留现有 `textora` binary，用户行为必须保持不变。

### 1.1 交付内容

- 新增 `crates/appkit-core`：无窗口应用内核。
- 新增 `crates/appkit-shell`：窗口、输入、渲染和插件会话运行时。
- 收敛 `crates/app`：只保留 textora 组合、同步、原生菜单、平台集成与装配入口。
- 参数化产品配置路径，消除共享层对 `~/.edit+/` 的硬编码。
- 将 textora 专属同步设置 UI 从通用 `ui` 层移回产品层。
- 建立依赖边界测试，防止共享层重新依赖 textora 产品代码。

### 1.2 明确非目标

- 不创建 `crates/note`，不新增任何 binary。
- 不实现 Vault、文件树、快速切换器或文件管理。
- 不新增 wikilink、反向链接、图谱、标签、全局搜索或同步能力。
- 不为了假想消费者设计通用文件操作协议。
- 不改变编辑、渲染、快捷键、设置、同步和会话恢复行为。
- 不同时升级 `winit`、`wgpu` 或其他基础依赖。

## 2. 现状与问题

- `crates/app` 同时持有文档模型、插件实例、GPU、事件循环、产品设置与同步服务。
- `Workspace` 直接创建 `textora-markdown` 插件，路由规则与插件注册硬编码在应用内。
- `DocItem` 同时包含可持久化文档状态和 `ViewPlugin`、画布视口、思维导图面板等渲染会话。
- `DocumentView` 同时包含文本/文件状态与 `Viewport`、`AdvanceCacheEntry` 等展示状态。
- `AppEvent` 混合 reshape、文件安全等运行时事件和 sync、recent files 等产品事件。
- `ui::settings_view` 直接包含 Sync 分类、输入与动作，和“通用 widget 层”定位不一致。
- 配置路径除 `app_init.rs` 外还散落在 `dirty_snapshot.rs`、`settings_io.rs`、`app_tab.rs` 和 dispatch 代码中；设置 DTO 继续由产品层负责映射，避免 `appkit-core` 依赖 UI 枚举。

单纯移动文件无法解决这些问题；必须先切开模型与运行时的所有权。

## 3. 方案选择

### 3.1 采用：边界先行、逐步搬迁

先在现有 `crates/app` 内建立稳定 ID、模型/运行时分离和产品端口，再将已经解耦的模块搬入新 crate。每个阶段保持编译和测试通过。

优点：

- 失败面小，容易定位行为回归；
- 不需要在一次提交中同时解决模块路径和所有权问题；
- 可以用依赖测试证明拆分结果，而不是只依赖目录结构。

代价：迁移期间会短暂存在兼容适配层，阶段完成后必须删除。

### 3.2 不采用：先创建 crate 再批量移动

直接移动 `Workspace`、`DocumentView`、`events` 和 `ui_shell` 会把现有循环依赖原样带入新 crate，并造成大范围编译错误，无法判断错误来自边界设计还是路径迁移。

### 3.3 不采用：本次设计完整多产品泛型框架

当前只有 textora 一个消费者。通用 Vault、任意产品事件类型和可动态加载产品等抽象没有验证对象。本次只保留必要端口，使产品能力不反向进入共享层；未来第二个 App 必须以真实需求验证并扩展端口。

## 4. 目标架构

```text
crates/core
  文本缓冲区、文档基础抽象、模糊匹配等领域能力

crates/appkit-core
  无窗口应用内核
  ├── DocumentModel / WorkspaceModel / TabId
  ├── 编辑命令与事务中的纯模型部分
  ├── 导航、行索引、内容哈希
  ├── 文件安全判定与外部变更分类
  └── persistence / workspace_store / dirty_snapshot / file_history

crates/ui
  通用 widget、布局、主题、绘制协议与 ViewPlugin 渲染协议

crates/appkit-shell
  通用编辑器运行时
  ├── Window / winit / IME / clipboard
  ├── GPU、render pipeline、reshape worker
  ├── DocumentPresentation / TabRuntimeStore
  ├── ViewPlugin 实例、路由与画布会话
  ├── 通用 widget 事件翻译
  └── ShellEvent 与 ProductHost 端口

crates/app
  textora 产品
  ├── textora UI 组合
  ├── markdown/mindmap/novel 插件装配
  ├── sync 设置页面与 sync_controller
  ├── native_menu / macOS 集成
  ├── ProductIdentity / ProductPaths
  └── main
```

依赖方向：

```text
core ← appkit-core
core + ui + render + shaping + appkit-core ← appkit-shell
core + ui + appkit-core + appkit-shell + textora-markdown + textora-sync ← app
```

硬性边界：

- `appkit-core` 不依赖 `ui`、`winit`、`wgpu`、`render`、`shaping`、`textora-markdown` 或 `textora-sync`。
- `appkit-shell` 不依赖 `textora-markdown`、`textora-sync` 或 `crates/app`。
- `ui` 不依赖 `appkit-core`、`appkit-shell` 或 `crates/app`，也不包含 textora 专属 Sync 页面和动作。
- `app` 可以依赖所有共享层，负责完成产品装配。

## 5. 核心所有权边界

### 5.1 WorkspaceModel 与稳定 TabId

`appkit-core` 定义稳定的 `TabId`，并让 `WorkspaceModel` 只管理：

- tab 顺序、活动 tab、固定状态和导航历史；
- `TabId → DocumentModel`；
- 文件路径、dirty 状态、未命名文档名称等可持久化模型数据。

禁止跨层使用可变化的 tab index 关联插件或 UI 会话。index 只用于当帧展示和用户动作，跨调用关联一律使用 `TabId`。

### 5.2 DocumentModel 与 DocumentPresentation

现有 `DocumentView` 拆成两个职责：

- `DocumentModel` 属于 `appkit-core`：文本、光标、选区、文件路径、磁盘 revision、dirty 状态、编码和编辑 generation。
- `DocumentPresentation` 属于 `appkit-shell`：display line map、viewport、advance cache、render cache、可见行和 reshape 状态。

两者通过 `TabId` 关联。展示状态可以丢弃并重建，模型状态不能依赖任何像素、GPU 或窗口类型。

### 5.3 TabRuntimeStore

`appkit-shell` 使用 `TabRuntimeStore` 管理：

```text
TabId
  └── TabRuntime
      ├── active_plugin: Box<dyn ViewPlugin>
      ├── cached_toggle_plugin
      ├── DocumentPresentation
      ├── CanvasViewportSession
      └── 插件自身的 per-tab UI 会话
```

`ViewPlugin` 实例本来就是 per-tab 对象，插件专属会话优先由插件实例直接持有。本次不新增通用 `Any` 类型擦除容器。组合层需要的信息通过明确的 plugin message/query 或纯数据面板输入暴露。

关闭 tab 时，`WorkspaceModel` 先产出包含 `TabId` 的关闭结果，shell 再删除对应 `TabRuntime`。边界测试必须覆盖切换、关闭、恢复和重排后模型与运行时仍一一对应。

## 6. 插件注册与视图路由

插件属于渲染运行时，不进入 `appkit-core`。

- `app` 创建 `PluginRegistry` 并注册 editor、markdown、mindmap、novel 插件。
- `app` 同时提供类型化 `ViewRouteTable`，再注入 `appkit-shell`。
- `appkit-shell` 根据路径和当前视图状态创建、切换插件，并把持久化所需的纯数据快照交给 `appkit-core`。

路由规则必须：

- 使用明确的 `ViewRouteRule` 类型，不使用匿名 tuple；
- 支持完整文件名后缀和普通扩展名两种 matcher；
- 按 specificity 排序，使 `.mmap.md` 优先于 `.md`；
- 启动时验证规则引用的插件均已注册；
- 拒绝相同优先级的冲突规则。

## 7. 产品身份与持久化路径

`app` 构造并注入：

```rust
pub struct ProductPaths {
    pub config_dir: PathBuf,
    pub theme_dir: PathBuf,
    pub workspace_file: PathBuf,
    pub pinned_paths_file: PathBuf,
    pub snapshots_dir: PathBuf,
    pub history_file: PathBuf,
    pub settings_file: PathBuf,
}
```

共享层只接收已经解析好的路径，不读取 `$HOME`，也不拼接 `.edit+`。textora 继续使用现有目录和文件名，保证兼容已有用户数据。

本次只有一个固定 workspace key，不实现多 Vault 存储。`WorkspaceStore` 的接口保留将来加入 key 的空间，但不提前实现多 workspace 清理策略。

## 8. Shell 与产品端口

### 8.1 ShellEvent

`appkit-shell` 只定义运行时可理解的事件：

```rust
pub enum ShellEvent {
    StartBackgroundServices,
    ReshapeResultsReady,
    FileSafetyResultsReady,
    ProductWake,
}
```

产品后台线程把结果写入产品自有 channel，然后只发送 `ProductWake` 唤醒事件循环。shell 不解析 sync、recent files 或其他产品 payload。

### 8.2 ProductHost

`appkit-shell` 定义最小产品端口：

```rust
pub trait ProductHost {
    fn start_background_services(&mut self, wake: ProductWakeHandle);
    fn drain_product_events(&mut self) -> ShellEffect;
    fn shutdown(&mut self);
}
```

约束：

- textora 的 widget 组合与产品 action reducer 明确保留在 `app`，不为唯一消费者设计泛型 composition；
- 通用编辑、tab、viewport、search 等动作由 shell 处理；
- sync、原生菜单、recent files 等由 textora `App` 与 `ProductHost` 实现处理；
- `ShellRuntime` 需要启动、唤醒或关闭产品服务时，临时借用 `&mut impl ProductHost`；
- `ShellEffect` 只表达 redraw、reshape、持久化、退出等运行时 effect，不包含 sync 领域状态。

如果实现过程中需要 `Any`、字符串 action 名、全局回调表或泛型产品 action，必须停止并重新划分动作归属，禁止用类型擦除绕过 crate 边界。

## 9. ui 与产品组合边界

`ui` 继续提供业务无关 widget 和纯数据输入/动作。

当前 `ui::settings_view` 中的 Sync 分类是待清理的产品耦合：

- 通用 Appearance、Editor、Interface 页面保留在 `ui`；
- Sync 页面、`SyncSettingsAction`、sync view model 和敏感字段输入移入 `crates/app`；
- `app` 使用 `ui::widgets::form` 等公开基础组件渲染 Sync 页面；
- 通用 `SettingsViewAction` 不再包含 `Sync` variant。

本次不新增 file tree、quick switcher，也不扩展 `SidebarAction`。

## 10. 迁移阶段

### P0：建立边界保护

- 新增依赖边界测试和架构 DTO。
- 引入 `TabId`，消除跨调用用 index 关联运行时状态。
- 引入 `ProductPaths`，收敛硬编码配置路径。

### P1：拆模型与运行时

- 拆分 `DocumentModel` / `DocumentPresentation`。
- 拆分 `WorkspaceModel` / `TabRuntimeStore`。
- 插件注册与视图路由改为由 app 注入 shell。

这一阶段仍可暂时位于 `crates/app` 内，先证明所有权边界正确。

### P2：抽取 appkit-core

- 创建 `crates/appkit-core`。
- 分批搬迁已经不依赖 UI 的模型、编辑、导航和持久化模块。
- 每批搬迁后运行 crate 测试和 `cargo check -p textora-app`。

### P3：抽取 appkit-shell

- 创建 `crates/appkit-shell`。
- 搬迁窗口、事件、输入映射、渲染管线、reshape 和通用 dispatch。
- 接入 `ShellEvent`、`ProductHost` 和 `TabRuntimeStore`。

### P4：收敛 textora 产品层

- 将 sync 设置 UI 移出 `ui`。
- 将 sync、native menu、recent files 和 macOS 集成留在 `app`，通过产品端口接入 shell。
- 删除迁移适配层和旧模块，保持 `textora` binary 装配入口唯一。

### P5：全面验证

- `cargo fmt --all -- --check`
- `cargo check -p textora-appkit-core`
- `cargo check -p textora-appkit-shell`
- `cargo check -p textora-app`
- 各 crate 单元测试与现有 smoke/public API 测试
- `./scripts/verify.sh`
- 按 `docs/manual_test_protocol.md` 验证打开、编辑、保存、恢复、视图切换、思维导图、设置和同步。

## 11. 验收标准

- 仍然只产出 `textora` 一个 binary。
- textora 用户可见行为和持久化格式保持兼容。
- `cargo tree -p textora-appkit-core` 中不存在 `ui`、`winit`、`wgpu`、`render`、`shaping`、`textora-markdown` 或 `textora-sync`。
- `cargo tree -p textora-appkit-shell` 中不存在 `textora-markdown`、`textora-sync` 或 `textora-app`。
- `rg '\\.edit\\+' crates/appkit-core crates/appkit-shell` 无结果。
- `rg 'SyncSettings|textora_sync' crates/ui` 无结果。
- Workspace 的模型状态与 shell 的运行时状态只通过 `TabId` 关联。
- 插件注册和路由由 `app` 注入，shared crate 不硬编码 textora-markdown。
- 每个阶段编译通过，最终 `./scripts/verify.sh` 通过。

## 12. 风险与对策

| 风险 | 对策 |
|------|------|
| `DocumentView` 拆分影响编辑与渲染热路径 | 先在原 crate 内拆所有权，增加模型/展示同步测试，再移动文件 |
| Workspace 与插件状态分离后生命周期错位 | 使用稳定 `TabId`，为关闭、重排、恢复建立契约测试 |
| `ui_shell/events/dispatch` 机制与产品逻辑难分 | 先按事件与 effect 分类，再逐个 handler 迁移；禁止一次移动整个文件 |
| Sync 页面移出 ui 后需要基础 form API | 只公开已存在的业务无关 form 组件，不把 sync DTO 留在 ui |
| 配置路径参数化破坏旧数据恢复 | 用现有 `~/.edit+` 目录建立兼容性测试，所有路径由 app 一次性构造 |
| 迁移期出现双实现 | 每个适配层标记删除阶段，P4 完成前清零旧入口和死代码 |

## 13. 未来扩展原则

未来若启动笔记 App，应把它作为 `appkit-core` 与 `appkit-shell` 的第二个真实消费者重新设计，单独编写产品规范。本次架构拆分不实现也不预埋 Vault、文件树或快速切换器协议。
