# Notora 工作区行尾操作与目录创建实施方案

日期：2026-08-13

状态：待实施

## 1. 背景

Notora 当前允许用户选择一个已有目录作为工作区根目录，但产品界面尚未提供完整的工作区
生命周期入口：用户不能从左侧导航新建工作区、切换到其他工作区，也不能在工作区目录树中
新建空子目录。

底层能力并非完全缺失：

- `Workspace::open_or_initialize` 已能为普通目录创建 `.notora/workspace.toml`；
- `WorkspaceCommand::Create` 已能创建目录并启动 catalog、索引 worker 与文件监听；
- `TreeListWidget` 已维护整行 hover 状态。

主要缺口位于产品动作、通用树组件的行尾操作、安全的工作区切换事务、目录领域命令以及
空目录导航数据源。

## 2. 产品决策

### 2.1 保持单工作区模型

Notora 同一窗口同一时刻只允许一个活动工作区。本阶段不引入多根工作区、工作区集合或
最近工作区列表。

左侧导航的工作区根节点可见文案固定为：

```text
工作区
```

即使活动工作区已经打开，也不把目录名称显示为根节点标题。根目录的绝对路径仅可通过
tooltip 或无障碍描述暴露，避免把单工作区导航误解为多工作区列表。

### 2.2 采用 VS Code Explorer 风格的 hover actions

工作区操作集中放在左侧导航树的行尾，仅在对应行 hover 或键盘聚焦时显示：

```text
工作区                         [新建工作区] [打开工作区] [新建目录]
  docs                                                   [新建目录]
    plans                                                [新建目录]
  notes                                                  [新建目录]

星标
回收站
文件
```

行为约束：

- 工作区根节点提供“新建工作区”“打开工作区”“新建目录”三个操作；
- 普通目录节点只提供“新建目录”；
- 尚无活动工作区时仍显示“工作区”根节点；新建和打开可用，新建目录不可用；
- 每个按钮必须提供 tooltip 和独立无障碍名称；
- 行尾按钮点击不触发行选择或展开；
- 行尾按钮按从右到左的稳定顺序布局，不因 hover 出现而推动标题；
- 根节点的绝对路径不作为常驻文案，仅作为 tooltip；
- 工作区切换成功后导航仍停留在“工作区”范围。

### 2.3 新建目录采用树内行内输入

点击“新建目录”后，在目标父目录的第一行子节点位置插入临时编辑行：

- Enter：提交创建；
- Escape：取消；
- 失焦：提交非空名称，空名称则取消；
- 创建失败：保留输入内容和焦点，在行下或产品错误区域展示错误；
- 创建成功：展开父目录、选中新目录并结束编辑；
- 同一时刻只允许一个目录创建草稿，使用互斥状态表达，禁止多个布尔字段组合。

根节点的新建目录目标为工作区根；目录节点的新建目录目标为该目录。

### 2.4 新建与打开工作区使用不同语义

“新建工作区”打开包含以下字段的模态框：

- 工作区名称；
- 保存位置；
- 只读的最终路径预览；
- 创建与取消按钮。

第一版不向 workspace manifest 增加 `display_name`。工作区名称只用于创建目录名，避免本次
引入 manifest schema 迁移。

“打开工作区”继续使用系统目录选择器，允许普通目录被初始化为 Notora 工作区。两者语义
必须区分：

- `Create`：目标目录必须不存在，只创建一个末级目录；
- `OpenExisting`：目标必须是已存在目录，可为其补充 `.notora` metadata；
- 目标已存在时，“新建工作区”返回明确冲突，不静默退化为打开已有目录；
- 用户取消选择时保持当前工作区和编辑状态不变。

## 3. 非目标

- 不支持同一窗口挂载多个工作区根；
- 不显示工作区目录名称作为左栏根节点标题；
- 不实现最近工作区列表；
- 不在设置页重复放置新建、打开入口；
- 不创建默认 README、示例笔记或固定目录结构；
- 不支持一次输入 `a/b/c` 隐式创建多级目录；
- 不允许 UI 层直接访问 `WorkspaceController`、`NotoraState` 或真实文件系统。

## 4. 目标交互状态

### 4.1 无活动工作区

```text
工作区                         [新建工作区] [打开工作区]
星标
回收站
文件
```

卡片区空状态说明可保留，但不再作为唯一工作区入口。新建笔记按钮保持禁用。

### 4.2 活动工作区

```text
▼ 工作区                       [新建工作区] [打开工作区] [新建目录]
    docs                                                [新建目录]
    notes                                               [新建目录]
```

工作区根节点默认展开；折叠只隐藏目录后代，不影响工作区激活状态。根节点选中时查询
`NavigationScope::WorkspaceRoot`。

### 4.3 创建目录草稿

```text
▼ 工作区
    [新目录名称________________]
    docs
    notes
```

若在 `docs` 行点击新建目录，草稿插入到 `docs` 的直接子节点区，不改变其他目录的相对顺序。

## 5. UI 组件设计

### 5.1 TreeList 通用行尾动作

`crates/ui` 只能接收纯数据输入，不得依赖 Notora 领域动作。建议增加：

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TreeRowActionKey(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeRowActionInput {
    pub key: TreeRowActionKey,
    pub icon: String,
    pub tooltip: String,
    pub accessibility_label: String,
    pub enabled: bool,
}

pub struct TreeRowInput {
    // 既有纯展示字段
    pub trailing_actions: Vec<TreeRowActionInput>,
}
```

TreeList 输出通用动作：

```rust
TreeListAction::TrailingActionActivated {
    row_key: TreeRowKey,
    action_key: TreeRowActionKey,
}
```

组件内部职责：

- 为行尾动作始终预留布局宽度；
- hover、选中或键盘聚焦行时绘制按钮；
- 单独维护 hover/pressed action，不用宽泛布尔字段；
- 命中优先级为行尾动作、展开箭头、整行选择；
- disabled action 不响应激活；
- `PointerLeave` 与 `InteractionCancel` 清理瞬态状态；
- 为每个行尾动作生成可独立激活的无障碍节点；
- `tooltip_at` 返回动作提示，工作区根标签区域返回绝对路径提示；
- 在窄栏中优先保留行尾操作，标签文本裁剪或省略。

需要为新建工作区与新建目录补充语义清晰的图标；不要复用同一个 `plus` 让三个动作只能靠
位置区分。建议至少增加 `folder-plus`，打开工作区继续使用 `folder-open`，新建工作区使用
独立的 workspace/create 图标。

### 5.2 TreeList 行内编辑

目录草稿属于通用树能力，应由 TreeList 组合无边框 `TextBox`，而不是由 Notora shell 重新
实现文本、IME、光标与焦点。

建议增加纯输入状态：

```rust
pub struct TreeRowEditorInput {
    pub key: TreeRowKey,
    pub parent_key: TreeRowKey,
    pub depth: usize,
    pub value: String,
    pub placeholder: String,
}
```

输出动作至少包括：

```rust
TreeListAction::EditorTextChanged { key, value }
TreeListAction::EditorCommitRequested { key, value }
TreeListAction::EditorCancelled { key }
```

应用层继续持有草稿真源，TreeList 只负责展示与编辑事件。IME、键盘光标和文本选择委托给
现有 `TextBox`。

## 6. Notora 产品状态与动作

### 6.1 类型化状态

扩展 `NotoraState`，让目录草稿和工作区转换状态均为互斥枚举：

```rust
pub enum DirectoryCreationState {
    Inactive,
    Editing {
        parent_relative_path: PathBuf,
        draft_name: String,
    },
    Submitting {
        parent_relative_path: PathBuf,
        directory_name: String,
    },
}

pub enum WorkspaceTransitionState {
    Idle,
    AwaitingDirtySaves { request: WorkspaceTransitionRequest },
    Applying { request: WorkspaceTransitionRequest },
}
```

不得用 `creating_directory`、`switching_workspace`、`save_pending` 等多个 bool 拼装状态。

### 6.2 产品动作

建议增加以下领域动作：

```rust
NewWorkspaceRequested
OpenWorkspaceRequested
WorkspaceCreationSubmitted { parent: PathBuf, name: String }
WorkspaceTransitionConfirmed(WorkspaceTransitionRequest)
WorkspaceTransitionFailed(String)
BeginDirectoryCreation { parent_relative_path: PathBuf }
DirectoryCreationTextChanged(String)
DirectoryCreationCommitRequested
DirectoryCreationCancelled
DirectoryCreationCompleted { relative_path: PathBuf }
DirectoryCreationFailed(String)
```

`NotoraRenderModel` 保存两个映射表：

- `TreeRowKey -> NavigationScope / relative directory path`；
- `(TreeRowKey, TreeRowActionKey) -> NotoraAction`。

TreeList action key 不承载路径或领域枚举，确保 `ui` 与 `notora-app` 解耦。

### 6.3 Effect 边界

系统目录选择、模态框和文件系统命令均从 reducer 输出 effect：

```rust
ShowWorkspaceCreationDialog
ChooseExistingWorkspace
PrepareWorkspaceTransition(WorkspaceTransitionRequest)
ExecuteDirectoryCommand(WorkspaceDirectoryCommand)
```

Reducer 不读取目录、不创建文件，也不直接调用 `rfd`。

## 7. 工作区目录领域命令

### 7.1 命令模型

在 core 或工作区 worker 可消费的领域模块中增加：

```rust
pub enum WorkspaceDirectoryCommand {
    Create {
        parent_relative_path: PathBuf,
        name: String,
    },
}
```

第一版只实现单目录创建。命令由活动工作区的唯一后台 worker 执行，主线程不得直接调用
`std::fs::create_dir`。

### 7.2 校验与安全

目录名必须满足：

- 去除首尾空白后非空；
- 只能是单一路径分量；
- 拒绝 `.`、`..`、路径分隔符和绝对路径；
- 拒绝保留 metadata 名 `.notora`；
- 拒绝平台保留名称与尾部句点/空格；
- 父目录必须存在且为目录；
- 经 canonicalize 后父目录仍在活动工作区内；
- 拒绝通过 symlink 逃逸；
- 目标不得已存在，不覆盖普通文件、目录或 symlink；
- 使用 `fs::create_dir`，不能用 `create_dir_all` 隐式创建未声明父级。

命令成功返回标准化后的工作区相对路径。错误使用结构化枚举表达，runtime 再映射为用户文案。

## 8. 空目录导航数据源

当前 `Catalog::navigation_tree` 只从活动笔记的 `relative_path` 推导目录。新建空目录不会进入
catalog，因而即使磁盘创建成功，下一次导航树刷新也会消失。

目标实现必须拆分两类数据：

```text
目录树：来自工作区文件系统扫描
标签及计数：来自 catalog
```

建议由工作区 index worker 统一构造 `CatalogNavigationTree` 的替代输入或新的
`WorkspaceNavigationTree`：

- 后台读取工作区目录；
- 忽略 `.notora`、Finder metadata、资源分叉和 symlink；
- 返回所有真实目录，包括空目录；
- 使用稳定的路径排序；
- catalog 继续提供标签与活动笔记计数；
- watcher 发现目录创建、删除或改名后刷新导航树；
- 应用内部目录创建成功后主动刷新，不等待 watcher 才更新 UI。

不要把空目录写入 SQLite 作为占位记录。目录属于文件系统事实，catalog 不应复制其生命周期。

## 9. 工作区新建与安全切换事务

### 9.1 必须解决的问题

当前 `execute_workspace_command` 在打开新工作区后会清除自动保存调度，但不会关闭旧工作区
笔记 Tab。旧笔记将继续显示，却不再属于活动工作区，存在自动保存失效和领域身份错配风险。

因此不能仅把 hover 按钮直接连接到既有 `WorkspaceCommand::Create` 或 `OpenExisting`。

### 9.2 事务顺序

```text
用户选定目标
  -> 校验目标但不破坏当前工作区
  -> 收集旧工作区 dirty 笔记
  -> 立即提交保存并等待完成
  -> 任一保存失败或冲突：取消切换，保留旧工作区
  -> 启动新工作区的 catalog / watcher / indexer
  -> 启动失败：保留旧工作区与旧 Tab
  -> 启动成功：关闭所有旧工作区 Note Tab
  -> 保留外部文件 Tab
  -> 重置导航、卡片、搜索、选择、overlay 与旧 generation 状态
  -> 激活新工作区并查询导航树
  -> 持久化 session
```

`WorkspaceController` 继续只负责工作区资源生命周期，不感知编辑器 dirty 状态。Runtime 负责
保存屏障与 Tab 清理。

### 9.3 Controller 的原子替换

当前 controller 先成功启动新 session，再关闭旧 session，这一原则必须保留。需要进一步
保证：

- `Create` 使用 `fs::create_dir`，目标存在即失败；
- 新工作区启动失败时不关闭旧 session；
- generation 仅在成功替换或明确关闭时推进；
- 新建过程中目录已创建但 metadata、catalog 或 watcher 初始化失败时，不递归删除用户目录；
- 错误信息说明可能已留下空目录，由用户决定是否保留。

### 9.4 编辑器文档隔离

切换成功后按文档来源处理：

- `DocumentOrigin::Note` 且 workspace id 属于旧工作区：关闭并从 registry 移除；
- `ExternalFile`：保留；
- `UntitledExternal`：保留；
- 旧工作区在途 save completion、scan completion、metadata mutation 和 search completion 必须
  通过 workspace id / generation 拒绝；
- 关闭旧 Note Tab 前必须确认保存屏障已经成功完成。

## 10. 导航层级调整

当前“工作区”是普通叶子行，一级目录与其平级。目标结构改为：

- “工作区”根：depth 0，可展开，标签固定；
- 一级目录：depth 1；
- 后续目录：按相对路径层级递增；
- 星标、标签、回收站和文件：仍是 depth 0 的独立导航项。

目录展开状态继续持久化相对路径；工作区根展开状态可使用独立产品字段，不使用空路径冒充
目录。切换工作区时清理旧工作区展开集合，恢复 session 时只恢复与 workspace id 匹配的集合。

## 11. 文件拆分与阶段实施

本功能必然修改超过三个文件，按项目规范拆成独立阶段，每阶段均保持可编译与可测试。

### 阶段 A：TreeList 行尾动作

目标：建立领域无关的 hover action 能力，不接入真实工作区 I/O。

主要修改：

- `crates/ui/src/widgets/tree_list/mod.rs`
- `crates/ui/src/widgets/tree_list/layout.rs`
- 必要的通用图标定义与导出文件

验收：

- hover/聚焦显示，离开隐藏；
- 点击动作不选中行；
- 展开箭头、整行选择行为不回归；
- tooltip、disabled 和无障碍激活有效；
- DPI 与窄宽度布局稳定。

### 阶段 B：工作区根层级与空目录树

目标：根节点变为真实父节点，并让空目录成为文件系统事实。

主要修改：

- `crates/notora-core` 的工作区目录扫描模块；
- `crates/notora-app/src/index_worker.rs`
- `crates/notora-app/src/workspace_controller.rs`
- `crates/notora-app/src/state.rs`
- `crates/notora-app/src/render.rs`

验收：

- 根节点始终显示“工作区”；
- 不显示目录名；
- 空目录与嵌套空目录可见；
- `.notora` 和 symlink 不进入导航；
- 目录 watcher 变化能刷新树。

### 阶段 C：新建子目录

目标：完成行内输入到后台目录命令的闭环。

主要修改：

- TreeList 行内编辑能力；
- core 目录命令及测试；
- index worker 命令与 completion；
- Notora action/state/effect/runtime/render 映射。

验收：

- 根目录和任意目录均可新建直接子目录；
- Enter、Escape、失焦与 IME 正常；
- 名称冲突和非法名称不改变磁盘；
- 成功后目录立即出现且父目录展开；
- 重启后空目录仍存在并显示。

### 阶段 D：安全工作区转换

目标：建立可复用的新建/打开工作区保存屏障。

主要修改：

- runtime 的 `PendingWorkspaceTransition`；
- document runtime 的 dirty 笔记收集、保存完成关联与旧 Note Tab 清理；
- workspace controller 的严格 Create 语义；
- reducer 状态复位和 session 持久化。

验收：

- dirty 保存成功后才切换；
- 保存失败、冲突或目标启动失败均保留旧工作区；
- 成功后旧 Note Tab 消失，外部文件 Tab 保留；
- 旧 generation 的异步完成不能污染新工作区。

### 阶段 E：新建/打开入口与模态框

目标：将根节点 hover actions 接入完整产品流程。

主要修改：

- Notora action/effect/reducer；
- `NotoraRenderModel` 的行尾动作映射；
- 新建工作区 modal widget；
- runtime 目录选择器与转换请求。

验收：

- 无工作区和有工作区时入口均可访问；
- 新建与打开语义不混用；
- 用户取消不改变当前状态；
- 创建目录已存在时提示冲突；
- 根节点名称始终为“工作区”。

## 12. 测试策略

### 12.1 UI 单元测试

- 只有被 hover、选中或键盘聚焦的行显示 actions；
- 行尾动作命中优先于行选择；
- action 之间移动时 hover 状态正确交接；
- PointerLeave、InteractionCancel 清理 hover/pressed 状态；
- disabled action 不输出激活动作；
- tooltip 对应正确 action；
- 无障碍树项下存在可独立激活的 action；
- 行内 TextBox 的 Enter、Escape、IME 与失焦行为；
- 高 DPI、窄侧栏和超长目录名称不会覆盖 actions。

### 12.2 Core 单元测试

- 仅创建一个直接子目录；
- 拒绝空名、`.`、`..`、分隔符、绝对路径和 `.notora`；
- 拒绝文件、目录和 symlink 冲突；
- 拒绝 symlink 父路径逃逸；
- 父目录缺失或不是目录时返回结构化错误；
- 目录扫描包含空目录并忽略 metadata 与 symlink；
- `WorkspaceCommand::Create` 拒绝已存在目标。

### 12.3 Reducer 与 runtime 测试

- 根行三个 action key 映射到正确 NotoraAction；
- 子目录只映射新建目录；
- 无工作区时新建目录无效；
- 创建草稿状态为互斥状态机；
- 成功创建后展开父目录并选择新路径；
- 切换时 dirty Note 保存失败会中止；
- 新 workspace 启动失败保留旧 controller session；
- 切换成功关闭旧 Note Tab 并保留 external/untitled external；
- 旧 workspace generation 的 completion 被丢弃；
- session 恢复时 workspace id 不匹配不恢复展开路径。

### 12.4 集成与手工验收

至少覆盖：

1. 首次启动，通过根行 hover 新建工作区；
2. 根节点文案始终是“工作区”，绝不显示目录名；
3. 在根和两级子目录中创建空目录；
4. 重启后空目录仍显示；
5. 打开普通目录后自动生成 `.notora` 并索引已有笔记；
6. dirty 笔记存在时切换工作区，确认保存完成后旧 Note Tab 被关闭；
7. 人为制造保存失败，确认工作区不切换；
8. 外部文件与未命名外部 Tab 在切换后保留；
9. 缩窄左栏，确认行尾按钮仍可操作且目录标题安全裁剪；
10. 键盘与屏幕阅读器可以访问三个根行操作。

每一阶段至少运行对应 crate 测试和 `cargo check --workspace`；整个重大修改完成后运行：

```bash
./scripts/verify.sh
```

## 13. 完成定义

满足以下全部条件才视为完成：

- 用户无需进入设置页即可从工作区根行新建或打开工作区；
- 用户可从工作区根或任意目录行创建直接子目录；
- 左栏根节点始终只显示“工作区”，不显示实际目录名称；
- 空目录是稳定、可重启恢复的导航节点；
- UI crate 不依赖 Notora 的状态、命令或文件系统；
- 新建和打开工作区都通过同一安全转换协议；
- 切换不会丢失 dirty 笔记，也不会留下归属旧工作区的 Note Tab；
- watcher、catalog、search 和 save 的旧异步结果不能污染新工作区；
- 单元测试、集成测试、`cargo check --workspace` 和 `./scripts/verify.sh` 全部通过。
