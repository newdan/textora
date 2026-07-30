# Task 16 ShellRuntime 前置解耦设计

## 背景

原 Task 16A 要求一次性把 `App` 收敛为：

```rust
pub struct App {
    shell: appkit_shell::ShellRuntime,
    product: TextoraProduct,
}
```

但当前 `ShellRuntime` 应持有的多个类型仍定义在 `textora-app`：

- `Workspace`
- `TabRuntimeStore`
- `TabSession`
- `UiShell`
- `MouseState`
- `SmoothScroll`
- `ViewRouteTable`
- `EditorPlugin`
- `MindmapStylePanelSession`

其中 `UiShell` 仍直接识别 `TextoraSettingsOverlay` 和
`SyncSettingsAction`，`Workspace` 同时承担插件构造、文档加载、runtime
创建和持久化恢复。直接把这些类型写入 `appkit-shell::ShellRuntime` 会形成
`appkit-shell -> textora-app` 反向依赖，违反既定架构红线。

## 目标

在不改变 textora 行为和持久化格式的前提下，先闭合共享类型的依赖，再创建
真正可理解其状态并能承担生命周期/dispatch/渲染职责的 `ShellRuntime`。

完成后：

- `appkit-shell` 只依赖 `appkit-core`、`ui`、窗口与渲染基础设施；
- `TextoraProduct` 持有 sync、native menu、产品路径和产品服务；
- `ShellRuntime` 持有窗口、workspace/runtime、通用 UI/input、渲染与
  file-safety 状态；
- `App` 只负责两者组合以及必须留在产品层的动作翻译；
- 不引入 `ShellRuntime<T>`、`Box<dyn Any>` 状态袋或 app 反向依赖。

## 方案比较

### 方案 A：泛型状态袋

让 `ShellRuntime<T>` 持有 app 定义的 `T`，可以绕开 Cargo 依赖，但 shell
无法对 `T` 执行生命周期、dispatch 或渲染逻辑。它只改变字段嵌套，不形成
真实运行时边界，拒绝采用。

### 方案 B：迁移期允许 shell 依赖 app

改动最少，但形成依赖环并使架构检查失去意义，拒绝采用。

### 方案 C：叶子优先迁移，再切换 runtime 字段

先移除共享类型中的产品引用，再按依赖顺序迁移类型；随后创建
`ShellRuntime`，分组迁入字段并逐步替换调用点。改动较多，但每一步可独立
测试、可回滚，最终边界真实，采用此方案。

## 类型所有权

### `appkit-core`

- `DocumentModel`
- `WorkspaceModel<DocumentModel>`
- `PersistedTab` / `PersistedWorkspace`
- `WorkspaceStore`
- snapshot、file history、file safety 的无窗口数据和 I/O

Task 17A/17B 提前到 Task 16 runtime 收敛之前执行。serde 字段、默认值和
`~/.edit+` 兼容格式保持不变。

### `appkit-shell`

- `ViewRouteTable`
- 通用 `EditorPlugin`
- `MindmapStylePanelSession`
- `TabRuntime` / `TabRuntimeStore`
- `TabSession` / `TabSessionMut`
- `EditorHostWidget`
- `MouseState` 及纯输入会话状态；文档命中测试算法可单独迁移
- `SmoothScroll`
- 去除产品 downcast 后的 `UiShell`
- 通用 `Workspace` runtime controller
- 最终 `ShellRuntime`

### `textora-app`

- `TextoraProduct`
- `ProductPaths`
- sync controller、sync settings 页面和动作
- native menu
- markdown/mindmap/novel 插件注册
- 产品路由表的规则数据
- settings/workspace/history 的产品路径注入
- widget action 到 textora action 的翻译

## 前置迁移顺序

### 1. 提前完成 Task 17 持久化边界

先把 `PersistedTab`、`PersistedWorkspace` 和 `WorkspaceStore` 移入
`appkit-core`。这消除 `Workspace` 对 app 持久化类型的依赖，但不改变文件
格式或路径注入方式。

### 2. 移动无产品依赖的叶子类型

按以下顺序逐项移动，每项保留 app 临时语义重导出：

1. `ViewRouteTable`
2. `EditorPlugin`
3. `MindmapStylePanelSession`
4. `SmoothScroll`
5. `MouseState` 与拖拽会话状态
6. `EditorHostWidget`
7. `TabRuntime` / `TabRuntimeStore`
8. `TabSession` / `TabSessionMut`

移动测试时使用 shell 内的最小测试插件，不从 app 导入
`plugins::editor::EditorPlugin`。

### 3. 清除 `UiShell` 产品语义

删除 `UiShell::take_pending_sync_settings_action`。`UiShell` 只继续提供已有的
泛型 `active_overlay_widget_mut<T: Any>`；app 侧通过
`ModalFrame -> TextoraSettingsOverlay` downcast 提取产品动作。

产品 overlay 集成测试留在 app，通用 dock、overlay、focus、scrollbar 和
layout 测试随 `UiShell` 移入 shell。之后物理迁移 `ui_shell.rs`。

### 4. 拆分并迁移 `Workspace`

`Workspace` 的职责拆成两侧：

- shell controller：持有 `WorkspaceModel<DocumentModel>`、导航历史、
  preview 状态、`PluginRegistry`、`ViewRouteTable`，并管理
  `TabRuntimeStore` 的稳定 `TabId` 生命周期；
- app adapter：加载文件、构造 textora 插件注册表、选择产品路由、创建
  typed untitled 文档、处理产品菜单和路径。

app adapter 向 shell 提交已经准备好的：

```rust
pub struct PreparedTab {
    pub document: DocumentModel,
    pub runtime: TabRuntime,
}
```

shell 不引用 `DocumentView`、`textora_markdown` 或 app 的插件模块。恢复流程
从 core DTO 读数据，由 app 注入注册表/路由，shell 重建通用 runtime。

### 5. 扩充产品容器

将以下产品状态收拢到 `TextoraProduct` 或其内部服务结构：

- `ProductPaths`
- settings/workspace/history 的持久化入口
- theme source load report
- library file monitor
- sync controller 和 native menu（已经完成）

shell 可以持有运行时 settings/theme 快照和通用 file-safety worker，但所有
产品路径必须通过构造参数注入，禁止重建 `~/.edit+`。

## `ShellRuntime` 迁移策略

完成前置迁移后再新增 `appkit-shell/src/runtime.rs`。

字段分三组迁入，每组独立测试和提交：

1. model/session：workspace、tab runtime store、popup tab ID snapshot；
2. UI/input：settings/theme snapshot、`UiShell`、mouse/modifier/IME、
   focus/animation 状态；
3. window/render：window/GPU/text、frame cache、reshape worker、字体系统、
   resize/redraw/帧时序和 file-safety 状态。

迁移期间：

- `App` 使用明确的 `shell()` / `shell_mut()` 和领域访问器；
- 不为 `App` 或 `ShellRuntime` 实现迁移用 `Deref`；
- 不把全部字段设为 `pub`；
- 调用点按模块分批改为 accessor，每个子任务最多修改 3 个文件；
- 所有字段迁完后，`App` 才收敛为 `shell + product` 两个字段。

## 生命周期与动作流

`ApplicationHandler<ShellEvent>` 继续实现于本地 `App`，避免 winit 入口直接
依赖产品类型。

```text
winit event
  -> App 识别产品/通用路由
  -> ShellRuntime 执行通用状态转移
  -> 必要时通过 ProductHost 唤醒/排空产品服务
  -> ShellEffect
  -> App 执行产品持久化或菜单动作
```

`ProductWake` 仍无 payload。sync、recent files 和 open-document payload
继续只存在于 `TextoraProduct` channel。

## 测试与完成门槛

每个行为调整遵循 RED-GREEN；纯移动在迁移前后运行同组测试。

每个前置任务必须满足：

- 修改不超过 3 个文件；
- `cargo fmt --all -- --check`；
- 相关 shell/core 测试；
- `cargo check -p textora-app`；
- `scripts/check_architecture.sh` 不出现新违规。

`ShellRuntime` 创建前必须确认：

- `rg 'textora_sync|TextoraSettings|NativeMenu|textora_markdown' crates/appkit-shell`
  无匹配；
- runtime 所有字段类型来自 std、core、ui、winit、wgpu、render、shaping、
  `appkit-core` 或 `appkit-shell`；
- fake registry/routes 构造测试不引用 textora sync 或 markdown 类型。

Task 16 完成后运行：

```bash
cargo fmt --all -- --check
bash scripts/check_architecture.sh
cargo check --workspace
cargo test -p textora-appkit-core
cargo test -p textora-appkit-shell
cargo test -p textora-app --lib
```

P5/Task 18 最终阶段仍运行 `./scripts/verify.sh` 和手工回归协议。
