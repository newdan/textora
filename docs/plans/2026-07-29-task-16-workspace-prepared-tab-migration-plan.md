# Task 16 Workspace / PreparedTab 迁移实施计划

> **执行要求：** 使用 `subagent-driven-development` 逐任务执行；每个实现任务使用
> 新实现代理和新审查代理。行为修改先 RED 后 GREEN，纯移动前后运行同一组测试。

## 目标

在不改变文档、插件、恢复格式和 tab 行为的前提下：

1. 引入产品无关的：

   ```rust
   pub struct PreparedTab {
       pub document: DocumentModel,
       pub runtime: TabRuntime,
   }
   ```

2. 将文件读取、typed untitled、textora 插件选择、dirty snapshot、file history
   和系统剪贴板行为留在 `textora-app`；
3. 将 `WorkspaceModel<DocumentModel>`、导航/preview/pin 状态、
   `PluginRegistry` 和 `ViewRouteTable` 收拢进
   `appkit-shell::workspace::Workspace`，并由它通过 PreparedTab 安装 API 和
   typed effects 管理 `TabRuntimeStore` 的稳定 `TabId` 生命周期；
4. `Workspace` 迁移完成前不创建 `ShellRuntime`，不引入泛型状态袋、
   `Deref` 或 shell → app 反向依赖。

## 最终边界

```text
textora-app
  workspace_tab_factory.rs   文件/外部内容/typed untitled → PreparedTab
  workspace_persistence.rs   PersistedWorkspace + dirty snapshot I/O
  workspace_product.rs       stub hydration、file history、复制路径、删除恢复标题
            │
            ▼
appkit-shell::workspace::Workspace
  WorkspaceModel<DocumentModel>
  PluginRegistry + ViewRouteTable
  navigation / preview / pin / close decisions
            │ typed install/effect
            ▼
  TabRuntimeStore（本阶段仍是 App 的 sibling 字段）
```

`PreparedTab` 严格只包含 `DocumentModel` 和 `TabRuntime`；建议文件名、激活策略和
持久化 DTO 作为安装参数或 app adapter 状态传递，不能塞入通用 DTO。

本阶段完成后，`App` 暂时仍分别持有 shell `Workspace` 与
`TabRuntimeStore`。下一份 `ShellRuntime` 计划再将二者作为 model/session 字段组
一起迁入 runtime，从而保留现有渲染、reshape 和鼠标路径对 sibling 字段的分拆借用。

## 全局约束

- 每个任务最多修改 3 个逻辑文件；完整文件移动的源/目标算一个逻辑文件。
- app adapter 可以使用 `DocumentView`、`textora_markdown`、dirty snapshot、
  file history、`rfd` 和 `arboard`；shell Workspace 不得使用这些类型。
- `PluginRegistry` 和 `ViewRouteTable` 由 app 注入，但所有权最终属于 shell
  Workspace。
- `TabId` 由 Workspace 分配；PreparedTab 安装必须同时更新 model/store。
  `WorkspaceEffect` 标记为 `#[must_use]`，App 统一用它删除关闭 ID 的 runtime。
- 最终不暴露任意 `&mut TabRuntimeStore` getter；本阶段只在安装、session 组合和
  typed effect reconciliation 的窄接口中传入 store。
- 恢复格式、snapshot 文件内容、typed untitled 初始文本/光标/建议文件名不变。
- 中间兼容路径使用显式方法或 app 重导出；禁止 `Deref`。

---

## Stage 0：解除全量验证的既有阻塞

### Task 0：修复 settings view 的既有 Clippy 阻塞

当前 `./scripts/verify.sh` 在早于本阶段的
`crates/ui/src/widgets/settings_view/widget.rs:701` 被
`clippy::obfuscated-if-else` 阻断。先用独立单文件提交清理，避免重大 Workspace
迁移完成后无法执行仓库要求的全面验证。

**文件：**

- Modify: `crates/ui/src/widgets/settings_view/widget.rs`

**步骤：**

1. 保留原条件和数值，将 `then(...).unwrap_or(0.0)` 改为明确 `if/else`。
2. 不改变设置页高度、断点或布局断言。

**验证：**

```bash
cargo fmt --all -- --check
cargo test -p textora-ui settings_view
cargo clippy -p textora-ui --all-targets -- -D warnings
```

**提交：** `style(ui): clarify settings sidebar width`

---

### Task 0A：恢复 appkit-shell 迁移代码的 Clippy policy

Task 0 修复后，`./scripts/verify.sh` 能继续进入 shell crate，并暴露从 app 物理迁移
代码时丢失的 crate-level lint policy。只恢复 app 原先已声明的四项迁移期规则，
不新增 `dead_code` 或覆盖其它 lint。

**文件：**

- Modify: `crates/appkit-shell/src/lib.rs`

**步骤：**

1. 从 `crates/app/src/lib.rs` 原样复制并保留原因说明：
   - `clippy::empty_line_after_doc_comments`
   - `clippy::question_mark`
   - `clippy::too_many_arguments`
   - `clippy::redundant_locals`
2. 不使用文件级 `allow(warnings)`，不放宽
   `new_without_default` / `assertions_on_constants`。

**验证：**

```bash
cargo fmt --all -- --check
cargo test -p textora-appkit-shell --lib
cargo check -p textora-app --tests
```

**提交：** `chore(appkit-shell): preserve migration lint policy`

---

### Task 0B：为第一批 shell 状态类型实现 Default

**文件：**

- Modify: `crates/appkit-shell/src/cursor_motion.rs`
- Modify: `crates/appkit-shell/src/editor_plugin.rs`
- Modify: `crates/appkit-shell/src/frame_cache.rs`

**步骤：**

1. 分别为 `CursorRenderState`、`EditorPlugin`、`FrameCache` 实现 `Default`。
2. 每个实现只委托现有 `Self::new()`，不复制初始化逻辑。
3. 增加或保留 `Default` 与 `new()` 等价的定向断言。

**验证：**

```bash
cargo fmt --all -- --check
cargo test -p textora-appkit-shell cursor_motion
cargo test -p textora-appkit-shell editor_plugin
cargo test -p textora-appkit-shell frame_cache
cargo check -p textora-app --tests
```

**提交：** `refactor(appkit-shell): default core shell state`

---

### Task 0C：清理剩余 shell 全量 Clippy 阻塞

**文件：**

- Modify: `crates/appkit-shell/src/mouse_state.rs`
- Modify: `crates/appkit-shell/src/smooth_scroll.rs`
- Modify: `crates/appkit-shell/src/render_state.rs`

**步骤：**

1. 为 `MouseState`、`SmoothScroll` 实现只委托 `Self::new()` 的 `Default`。
2. 将测试中的运行期常量断言改为 const assertion，不改变 `ATLAS_SIZE`。
3. 执行 shell all-target Clippy 和仓库完整验证；若仍有 warning，先确认是否是
   本阶段当前 HEAD 的真实阻塞，不用 blanket allow 掩盖。

**验证：**

```bash
cargo fmt --all -- --check
cargo test -p textora-appkit-shell
cargo clippy -p textora-appkit-shell --all-targets -- -D warnings
```

**提交：** `refactor(appkit-shell): complete strict clippy baseline`

---

### Task 0D：清理 sync settings 测试模块同名阻塞

Task 0C 使 shell strict Clippy 通过后，`./scripts/verify.sh` 继续在
`crates/app/src/sync_settings_page.rs` 的同名测试子模块触发
`clippy::module_inception`。这是纯测试命名修正，单独提交。

**文件：**

- Modify: `crates/app/src/sync_settings_page.rs`

**步骤：**

1. 将文件内 `#[cfg(test)] mod sync_settings_page` 改为语义明确且不与外层模块
   同名的 `mod tests`。
2. 不移动测试、不改断言、不改变 production visibility。
3. 用 `--list` 确认该模块的代表性测试仍被发现，再执行完整验证。

**验证：**

```bash
cargo fmt --all -- --check
cargo test -p textora-app --lib -- --list | rg 'sync_settings_page::tests::'
cargo test -p textora-app --lib sync_settings_page::tests
cargo clippy -p textora-app --all-targets -- -D warnings
./scripts/verify.sh
```

**提交：** `test(app): clarify sync settings test module`

---

## Stage A：建立 PreparedTab 与产品构造边界

### Task 1：在 appkit-shell 定义 PreparedTab

**文件：**

- Add: `crates/appkit-shell/src/prepared_tab.rs`
- Modify: `crates/appkit-shell/src/lib.rs`

**步骤：**

1. 定义只含 `DocumentModel`、`TabRuntime` 的 `PreparedTab` 和
   `PreparedTab::new`。
2. 用 shell 的 `EditorPlugin` 构造最小测试，并用
   `let PreparedTab { document, runtime } = prepared;`（无 `..`）穷尽解构，
   证明 DTO 恰好两个字段且值原样保留。
3. 不增加建议文件名、路径、插件 ID 或 bool 状态。

**验证：**

```bash
cargo fmt --all -- --check
cargo test -p textora-appkit-shell -- --list | rg '^prepared_tab::tests::prepared_tab_preserves_document_and_runtime:'
cargo test -p textora-appkit-shell prepared_tab
cargo check -p textora-app --tests
bash scripts/check_architecture.sh
```

**提交：** `feat(appkit-shell): add prepared tab boundary`

---

### Task 2：让 app Workspace 只接收 PreparedTab 安装输入

**文件：**

- Modify: `crates/app/src/workspace.rs`

**步骤：**

1. 增加两个语义明确的入口：
   - `append_prepared_tab(&mut self, &mut TabRuntimeStore, PreparedTab,
     Option<String>) -> TabId`：不改变激活与历史；
   - `open_prepared_tab(&mut self, &mut TabRuntimeStore, PreparedTab,
     Option<String>) -> WorkspaceEffect`：记录导航并激活。
   两者共用私有 `insert_prepared_tab`，负责分配 `TabId`、插入 document，并
   在返回前向同一个 ID 插入 runtime；不返回半安装的 `OpenedTab`，不使用行为
   bool。
2. 增加
   `pub(crate) fn reconcile_runtime_store(&self, runtimes: &mut TabRuntimeStore)`：
   `Closed` 精确删除 closed ID，其他 effect 不修改 store。本任务暂不添加
   `#[must_use]`，等 Task 3–6 清理完所有 App 测试夹具的直接调用后再由 Task 7
   启用编译器约束。
3. 本任务只新增原子入口；现有 open/new helper 暂不改签名，由 Task 9–13
   切换调用者后删除，避免在一个文件内制造隐藏 store。
4. 增加 characterization tests：ID 稳定、document/runtime 双射、建议文件名、
   激活、preview auto-close、close reconciliation 和历史效果与旧路径一致。

**验证：**

```bash
cargo fmt --all -- --check
cargo test -p textora-app --lib workspace::tests
cargo check -p textora-app --tests
bash scripts/check_architecture.sh
```

**提交：** `refactor(workspace): accept prepared tabs`

---

### Task 3：建立 App 测试切换入口并迁移第一批夹具

**文件：**

- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/app_scroll.rs`
- Modify: `crates/app/src/app_renderer.rs`

**接口：**

```rust
#[cfg(test)]
pub(crate) fn switch_workspace_for_test(&mut self, index: usize);
```

该 helper 调用 `workspace.switch_to(index)` 后，必须立即调用
`effect.reconcile_runtime_store(&mut self.tab_runtime_store)`。它返回 `()`：
这些夹具只需要建立激活状态，不向测试泄漏一个之后可能被忽略的 must-use effect。

**步骤：**

1. 在 `App` 增加上述测试 helper。
2. 将三个文件中所有裸调用及 `let _ = app.workspace.switch_to(...)` 改为 helper。
3. 不改变测试断言和 tab 激活顺序。

**验证：**

```bash
if rg -n "^[[:space:]]*(let _ = )?app\.workspace\.switch_to" \
  crates/app/src/app.rs crates/app/src/app_scroll.rs crates/app/src/app_renderer.rs; then
  exit 1
fi
cargo fmt --all -- --check
cargo test -p textora-app --lib app
cargo test -p textora-app --lib app_scroll
cargo test -p textora-app --lib app_renderer
cargo check -p textora-app --tests
```

**提交：** `test(app): reconcile workspace switches in core fixtures`

---

### Task 4：迁移第二批 App 测试切换夹具

**文件：**

- Modify: `crates/app/src/app_window.rs`
- Modify: `crates/app/src/app_dispatch.rs`
- Modify: `crates/app/src/app_tests.rs`

**步骤：**

1. 将测试中的裸调用及 `let _ = app.workspace.switch_to(...)` 改为
   `app.switch_workspace_for_test(...)`。
2. 将 `app_tests.rs` 中的 `workspace.set_active_index_for_test(...)` 同步改为该
   helper，禁止绕过导航 effect。
3. 保持 production `app_dispatch.rs` 中
   `let workspace_effect = self.workspace.switch_to(index)` →
   `apply_workspace_effect` 的路径不变。

**验证：**

```bash
if rg -n "^[[:space:]]*(let _ = )?app\.workspace\.(switch_to|set_active_index_for_test)" \
  crates/app/src/app_window.rs crates/app/src/app_dispatch.rs crates/app/src/app_tests.rs; then
  exit 1
fi
cargo fmt --all -- --check
cargo test -p textora-app --lib app_window
cargo test -p textora-app --lib app_dispatch
cargo test -p textora-app --lib app_tests
cargo check -p textora-app --tests
```

**提交：** `test(app): reconcile workspace switches in window fixtures`

---

### Task 5：迁移输入分发测试切换夹具

**文件：**

- Modify: `crates/app/src/dispatch/mouse.rs`
- Modify: `crates/app/src/dispatch/wysiwyg.rs`
- Modify: `crates/app/src/events.rs`

**步骤：**

1. 将三个文件中所有测试直接切换改为
   `app.switch_workspace_for_test(...)`。
2. 保持 mouse、WYSIWYG 和 mmap cursor 的行为断言不变。

**验证：**

```bash
if rg -n "^[[:space:]]*(let _ = )?app\.workspace\.switch_to" \
  crates/app/src/dispatch/mouse.rs crates/app/src/dispatch/wysiwyg.rs \
  crates/app/src/events.rs; then
  exit 1
fi
cargo fmt --all -- --check
cargo test -p textora-app --lib dispatch::mouse
cargo test -p textora-app --lib dispatch::wysiwyg
cargo test -p textora-app --lib events
cargo check -p textora-app --tests
```

**提交：** `test(app): reconcile workspace switches in input fixtures`

---

### Task 6：收口剩余切换路径并统一 effect reconciliation

**文件：**

- Modify: `crates/app/src/dispatch/commands.rs`
- Modify: `crates/app/src/dispatch/tabs.rs`
- Modify: `crates/app/src/app_tab.rs`

**步骤：**

1. 将测试直接切换改为 `app.switch_workspace_for_test(...)`。
2. `App::apply_workspace_effect` 首先调用
   `effect.reconcile_runtime_store(&mut self.tab_runtime_store)`，删除现有手写
   `if let Closed { closed, .. }` runtime 删除逻辑。
3. 保持 production effect 的消费顺序、empty-workspace 补建 tab 和 UI effects
   不变。
4. 扫描整个 app（排除 `workspace.rs` 自身单元测试）不再存在测试式的裸
   `app.workspace.switch_to` / `let _ =` 调用。

**验证：**

```bash
if rg -n "^[[:space:]]*(let _ = )?app\.workspace\.switch_to" \
  crates/app/src --glob '*.rs' --glob '!workspace.rs'; then
  exit 1
fi
cargo fmt --all -- --check
cargo test -p textora-app --lib dispatch::commands
cargo test -p textora-app --lib dispatch::tabs
cargo test -p textora-app --lib app_tab
cargo check -p textora-app --tests
```

**提交：** `refactor(app): reconcile workspace effects centrally`

---

### Task 7：预清理 Workspace 自测的 effect 消费

**文件：**

- Modify: `crates/app/src/workspace.rs`

**步骤：**

1. 先临时添加 `#[must_use]` 并运行 all-target Clippy，记录真实未消费清单；
   本任务提交前移除该属性，把最终启用留给 Task 7B。
2. Workspace 单元测试中的每个 effect 必须被断言、用于 reconciliation，或在
   明确只测试 model 导航时写为 `let _ =`；不得留下裸调用。
3. 不修改 Workspace 行为；只建立最终编译器约束的单文件前置条件。

**验证：**

```bash
cargo fmt --all -- --check
cargo test -p textora-app --lib workspace::tests
cargo check -p textora-app --tests
bash scripts/check_architecture.sh
```

**提交：** `test(workspace): consume controller effects explicitly`

---

### Task 7A：预清理 App 剩余 effect 消费点

Task 7 的 must-use RED 清单还覆盖三个 Workspace 外调用文件，必须在最终启用属性
前独立清理，保持每任务不超过 3 个逻辑文件。

**文件：**

- Modify: `crates/app/src/dispatch/tabs.rs`
- Modify: `crates/app/src/app_tab.rs`
- Modify: `crates/app/src/events.rs`

**步骤：**

1. 对只做夹具准备或 append 安装、语义上明确不需要 UI effect 的调用写成
   `let _ = ...`。
2. 需要 runtime reconciliation 或 UI effect 的路径不得丢弃，必须走现有
   `apply_workspace_effect` / test helper。
3. 不改变 production load、close、popup action 或测试顺序。

**验证：**

```bash
cargo fmt --all -- --check
cargo test -p textora-app --lib dispatch::tabs
cargo test -p textora-app --lib app_tab
cargo test -p textora-app --lib events
cargo check -p textora-app --tests
```

**提交：** `test(app): consume remaining workspace effects`

---

### Task 7B：启用 WorkspaceEffect 的 must-use 约束

**文件：**

- Modify: `crates/app/src/workspace.rs`

**步骤：**

1. 将 `WorkspaceEffect` 标记为 `#[must_use]`。
2. 不再修改调用者；all-target Clippy 必须直接全绿，证明 Task 7/7A 清单已闭合。

**验证：**

```bash
cargo fmt --all -- --check
cargo test -p textora-app --lib workspace::tests
cargo clippy -p textora-app --all-targets -- -D warnings
bash scripts/check_architecture.sh
```

**提交：** `refactor(workspace): require effect reconciliation`

---

### Task 8：新增 textora ProductTabFactory

**文件：**

- Add: `crates/app/src/workspace_tab_factory.rs`
- Modify: `crates/app/src/lib.rs`
- Modify: `crates/app/src/workspace.rs`

**接口：**

```rust
pub(crate) struct ProductPreparedTab {
    pub(crate) prepared: appkit_shell::prepared_tab::PreparedTab,
    pub(crate) suggested_file_name: Option<String>,
}

pub(crate) fn prepare_file(
    workspace: &Workspace,
    path: &Path,
    dimensions: ViewportDimensions,
) -> Result<ProductPreparedTab, String>;

pub(crate) fn prepare_external_content(
    workspace: &Workspace,
    path: &Path,
    content: &str,
    dimensions: ViewportDimensions,
) -> ProductPreparedTab;

impl Workspace {
    pub(crate) fn plugin_route_for_path(&self, path: &Path) -> Option<ViewRouteRule>;
}
```

`prepare_untitled` / `prepare_typed_untitled` 使用同一返回类型；typed kind 只存在于
app factory。`ViewportDimensions` 从 Workspace 移入本模块并由 app 重导出兼容。

**步骤：**

1. 在 `workspace_tab_factory.rs` 定义产品构造函数：
   - `prepare_file`
   - `prepare_external_content`
   - `prepare_untitled`
   - `prepare_typed_untitled`
2. factory 负责 `DocumentView`、dirty snapshot ID、typed untitled 规格和产品
   插件 ID；返回 app-local
   `ProductPreparedTab { prepared: PreparedTab, suggested_file_name: Option<String> }`。
3. Workspace 只公开产品无关的 route/plugin factory 方法：
   `create_plugin_for_path`、`create_plugin_by_name` 和
   `plugin_route_for_path(&Path) -> Option<ViewRouteRule>`；最后一个接口复制
   route rule，供恢复路径同时保留 `default_plugin` 与 `toggle_target`，不暴露
   route table。fallback 使用 shell `EditorPlugin`。
4. `ViewportDimensions` 从 Workspace 移到 factory；`workspace.rs` 暂时
   `pub(crate) use crate::workspace_tab_factory::ViewportDimensions`，保持既有
   `crate::workspace::ViewportDimensions` 调用路径直至物理迁移。
5. 先增加 factory characterization tests，覆盖 txt/md/mmap、外部内容、
   三种 typed untitled 的文本、光标、插件和建议文件名。

**验证：**

```bash
cargo fmt --all -- --check
cargo test -p textora-app --lib workspace_tab_factory
cargo check -p textora-app --tests
bash scripts/check_architecture.sh
```

**提交：** `refactor(app): add product tab factory`

---

### Task 9：切换生产 open/new 路径到 ProductTabFactory

**文件：**

- Modify: `crates/app/src/workspace_tab_factory.rs`
- Modify: `crates/app/src/dispatch/tabs.rs`
- Modify: `crates/app/src/app_lifecycle.rs`

**步骤：**

1. `App::open_file` 和外部内容路径先查 existing tab，再调用 factory；
   new untitled / typed untitled 直接构造新 tab。启动 `App::load_file` 保持旧
   `push_entry_for_file` 的无条件 append 行为，不引入去重或切换。
2. 普通 open/new/external 路径将 `ProductPreparedTab` 拆给
   `workspace.open_prepared_tab(&mut tab_runtime_store, ...)`；启动阶段
   `App::load_file` 必须调用 `append_prepared_tab`，保持旧
   `push_entry_for_file` 的“只追加、不记录导航、不产生 Activated effect”
   语义。
3. 增加启动文件回归测试，断言追加后 active tab 与历史/effect 行为不变。
4. 保持 `WorkspaceEffect`、history、native menu、redraw/persist effect 顺序。
5. 不在 factory 中访问窗口、菜单或 App 状态。

**验证：**

```bash
cargo fmt --all -- --check
cargo test -p textora-app --lib dispatch::tabs
cargo test -p textora-app --lib app_lifecycle
cargo check -p textora-app --tests
bash scripts/check_architecture.sh
```

**提交：** `refactor(app): route tab creation through product factory`

---

### Task 10：迁移 recent-file 打开路径

**文件：**

- Modify: `crates/app/src/dispatch/commands.rs`
- Modify: `crates/app/src/workspace_tab_factory.rs`

**接口：**

recent-file 命令使用与 `App::open_file` 相同的 `ProductPreparedTab`，安装并激活后
再恢复 cursor line/column 和 scroll anchor；existing tab 仍只切换，不重复构造。

**步骤：**

1. 将 `dispatch/commands.rs` 的 `workspace.open_file_with_viewport` 改为 factory
   + `workspace.open_prepared_tab(&mut tab_runtime_store, ...)`。
2. 保持“安装/激活 → 恢复 cursor → 恢复 scroll anchor → reshape/redraw”的顺序。
3. 增加 recent-file 新开与 existing-tab 两条回归测试。

**验证：**

```bash
if rg -n "open_file_with_viewport" crates/app/src/dispatch/commands.rs; then
  exit 1
fi
cargo fmt --all -- --check
cargo test -p textora-app --lib dispatch::commands
cargo check -p textora-app --tests
bash scripts/check_architecture.sh
```

字段扫描预期无匹配。

**提交：** `refactor(app): prepare recent file tabs`

---

### Task 11：迁移跨模块测试夹具到产品入口

**文件：**

- Modify: `crates/app/src/app_window.rs`
- Modify: `crates/app/src/events.rs`
- Modify: `crates/app/src/dispatch/tabs.rs`

**步骤：**

1. 将直接 `workspace.new_untitled/new_typed_untitled` 的 app 测试改为 App
   产品 helper 或 `ProductTabFactory`。
2. 不改变几何、事件、tab 顺序或 effect 断言。
3. 扫描三个文件不再调用 Workspace 产品构造方法。

**验证：**

```bash
if rg -n "workspace\\.(new_untitled|new_typed_untitled|open_external_content)" \
  crates/app/src/app_window.rs crates/app/src/events.rs crates/app/src/dispatch/tabs.rs; then
  exit 1
fi
cargo fmt --all -- --check
cargo test -p textora-app --lib app_window
cargo test -p textora-app --lib events
cargo test -p textora-app --lib dispatch::tabs
cargo check -p textora-app --tests
```

**提交：** `test(app): use product tab preparation fixtures`

---

### Task 12：迁移 external-change 的原子 tab 测试夹具

**文件：**

- Modify: `crates/app/src/external_change_tests.rs`

**步骤：**

1. 将 `Workspace::new()` 改为显式 `build_product_workspace()`。
2. 将直接 `workspace.push_entry_for_test(...)` 改为：拆分 `DocumentView`，
   构造 `PreparedTab`，并通过本地 `TabRuntimeStore` 调用
   `append_prepared_tab`。测试继续直接断言 Workspace 的删除恢复行为。
3. 不改变 recovery content/title/dirty/revision 断言；本地 runtime store 只用于
   维护 model/runtime 双射。

**验证：**

```bash
if rg -n "Workspace::new\(\)|workspace\.push_entry_for_test" \
  crates/app/src/external_change_tests.rs; then
  exit 1
fi
cargo fmt --all -- --check
cargo test -p textora-app --lib external_change_tests
cargo check -p textora-app --tests
```

**提交：** `test(app): prepare external change workspace tabs`

---

### Task 12A：删除零调用的 OpenTabResult 兼容 reducer

Task 9 切换生产路径后，`dispatch/tabs.rs` 仍残留零调用的
`App::apply_open_tab_result`，它会阻塞 Task 13 删除 `OpenTabResult` 与
`install_opened_tab`。单独清理该死代码，保持三文件删除任务边界闭合。

**文件：**

- Modify: `crates/app/src/dispatch/tabs.rs`

**步骤：**

1. 用全 app 扫描确认 `apply_open_tab_result` 只有定义、没有调用。
2. 删除该方法；不改任何 active production 调用链。
3. 扫描 `dispatch/tabs.rs` 不再引用 `OpenTabResult` / `install_opened_tab`。

**验证：**

```bash
if rg -n "apply_open_tab_result|OpenTabResult|install_opened_tab" \
  crates/app/src/dispatch/tabs.rs; then
  exit 1
fi
cargo fmt --all -- --check
cargo test -p textora-app --lib dispatch::tabs
cargo check -p textora-app --tests
cargo clippy -p textora-app --all-targets -- -D warnings
```

**提交：** `refactor(app): remove obsolete open tab reducer`

---

### Task 13：删除 Workspace 的产品 tab 构造职责

**文件：**

- Modify: `crates/app/src/workspace.rs`
- Modify: `crates/app/src/workspace_tab_factory.rs`
- Modify: `crates/app/src/app_tab.rs`

**步骤：**

1. 将 workspace 内 file/typed/plugin 路由集成测试移到 product factory。
2. 删除 tab 构造路径中的 `NewDocumentKind`、typed spec、`split_tab` 和
   open/new 产品方法；本阶段保留 snapshot/restore 与 lazy-load 仍需的
   `DocumentView`，它们分别由 Task 14–15 与 Task 19–21 外移。
3. 删除已被原子 PreparedTab 安装替代的 `OpenedTab` / `OpenTabResult` 以及
   `App::install_opened_tab`。
4. 保留 `App::push_entry_for_test(DocumentView, Box<dyn ViewPlugin>) -> TabId`
   的外部签名，但其内部将 `DocumentView` 拆成 model/presentation，构造
   `PreparedTab` 并调用 `workspace.append_prepared_tab`，不再依赖 Workspace
   产品测试 helper。
5. Workspace 内部测试统一通过 `PreparedTab + TabRuntimeStore` 安装夹具，并删除
   Workspace 自身的 `push_entry_for_test`。
6. Workspace 保留 PreparedTab 安装、model、导航、close/pin 和通用 plugin
   factory 能力。

**验证：**

```bash
if rg -n "NewDocumentKind|typed_untitled_spec|open_file_with_viewport|open_external_content|new_untitled|new_typed_untitled|push_entry_for_file|struct OpenedTab|enum OpenTabResult|fn split_tab|fn push_entry_for_test" \
  crates/app/src/workspace.rs; then
  exit 1
fi
if rg -n "\\.(open_file_with_viewport|open_external_content|new_untitled|new_typed_untitled|push_entry_for_file)\\(" \
  crates/app/src --glob '*.rs'; then
  exit 1
fi
if rg -n "fn push_entry_for_test|workspace\.push_entry_for_test" \
  crates/app/src/workspace.rs crates/app/src --glob '*.rs' --glob '!app_tab.rs'; then
  exit 1
fi
cargo fmt --all -- --check
cargo test -p textora-app --lib workspace
cargo test -p textora-app --lib workspace_tab_factory
cargo check -p textora-app --tests
bash scripts/check_architecture.sh
```

两项产品构造扫描预期无匹配。

**提交：** `refactor(workspace): remove product tab construction`

---

## Stage B：外移持久化与产品副作用

### Task 14：提取 Workspace 持久化适配器

**文件：**

- Add: `crates/app/src/workspace_persistence.rs`
- Modify: `crates/app/src/lib.rs`
- Modify: `crates/app/src/workspace.rs`

**接口：**

```rust
pub(crate) struct RestoredWorkspace {
    pub(crate) workspace: Workspace,
    pub(crate) runtimes: TabRuntimeStore,
}

pub(crate) fn snapshot_workspace(
    workspace: &Workspace,
    runtimes: &TabRuntimeStore,
    sidebar_pinned: bool,
    sidebar_width: Option<f32>,
    snapshots_dir: &Path,
) -> PersistedWorkspace;

pub(crate) fn restore_workspace(
    workspace: Workspace,
    snapshot: PersistedWorkspace,
    dimensions: ViewportDimensions,
    line_height: f64,
    snapshots_dir: &Path,
) -> io::Result<RestoredWorkspace>;
```

调用者通过 `build_product_workspace()` 注入带产品 registry/routes 的空 Workspace。
adapter 只能使用 `entries`、entry/ID accessors、plugin factory 和
`plugin_route_for_path`、`append_prepared_tab`；不得公开 `model`、`registry`、
`view_routes` 或 `split_tab` 字段/内部函数。

**步骤：**

1. 将 snapshot/restore、dirty diff I/O、revision baseline 和相关测试移到
   `workspace_persistence.rs`。
2. 先在 Workspace 保留薄兼容 wrapper，避免同任务修改调用者。
3. 恢复仍输出 `{ workspace, runtimes }`；每个恢复 tab 经 PreparedTab 和
   `append_prepared_tab` 原子安装。恢复每个 file-backed tab 时通过
   `plugin_route_for_path` 同时保留 route 的 `default_plugin` 与
   `toggle_target`，序列化字段与恢复策略不变。
4. 所有 document/runtime 安装完成后，按 `snapshot.active_index` 调用
   `switch_to`，消费并通过 `reconcile_runtime_store` 处理 must-use effect；随后
   对最终 active session 调用现有 `ensure_cursor_visible(line_height)`。回归测试
   同时断言 active ID、cursor/selection、scroll anchor 和 runtime 双射。

**验证：**

```bash
cargo fmt --all -- --check
cargo test -p textora-app --lib workspace_persistence
cargo test -p textora-app --lib workspace
cargo check -p textora-app --tests
bash scripts/check_architecture.sh
```

**提交：** `refactor(app): extract workspace persistence adapter`

---

### Task 14A：迁移 ProductTabFactory 的持久化测试调用

Task 14 提取 adapter 后，`workspace_tab_factory.rs` 的 typed/untitled 测试仍通过
Workspace 薄 wrapper 做 snapshot/restore。先切换这些测试，避免 Task 15 删除
wrapper 时产生第四个逻辑文件。

**文件：**

- Modify: `crates/app/src/workspace_tab_factory.rs`

**步骤：**

1. 将测试中的 `snapshot_with_runtime_store` 改为
   `workspace_persistence::snapshot_workspace`。
2. 将 `Workspace::restore_with_viewport` 改为
   `workspace_persistence::restore_workspace(build_product_workspace(), ...)`。
3. 保持 typed/untitled persistence 往返、runtime 双射和 snapshot ID 断言。

**验证：**

```bash
if rg -n "snapshot_with_runtime_store|restore_with_viewport" \
  crates/app/src/workspace_tab_factory.rs; then
  exit 1
fi
cargo fmt --all -- --check
cargo test -p textora-app --lib workspace_tab_factory
cargo test -p textora-app --lib workspace_persistence
cargo check -p textora-app --tests
```

**提交：** `test(app): call workspace persistence adapter directly`

---

### Task 15：切换持久化调用者并删除兼容 wrapper

**文件：**

- Modify: `crates/app/src/app_tab.rs`
- Modify: `crates/app/src/app_window.rs`
- Modify: `crates/app/src/workspace.rs`

**步骤：**

1. save/restore 调用改为 `workspace_persistence` free functions。
2. 删除 Workspace 中的 snapshot/restore wrapper 和 app persistence imports。
3. 保持启动恢复、active stub、cursor/selection/scroll anchor 和 runtime 恢复顺序。

**验证：**

```bash
if rg -n "PersistedWorkspace|RestoredWorkspace|snapshot_with_runtime_store|restore_with_viewport" \
  crates/app/src/workspace.rs; then
  exit 1
fi
cargo fmt --all -- --check
cargo test -p textora-app --lib workspace_persistence
cargo test -p textora-app --lib app_window
cargo test -p textora-app --lib app_tab
cargo check -p textora-app --tests
```

Workspace 产品持久化扫描预期无匹配。`dirty_snapshot` 删除恢复逻辑按
Task 16–17 提取，完整产品依赖扫描留到 Task 21。

**提交：** `refactor(workspace): remove persistence side effects`

---

### Task 16：提取 Workspace 产品副作用适配器

**文件：**

- Add: `crates/app/src/workspace_product.rs`
- Modify: `crates/app/src/lib.rs`
- Modify: `crates/app/src/workspace.rs`

**接口：**

```rust
pub(crate) fn hydrate_active_stub(workspace: &mut Workspace) -> bool;
pub(crate) fn detach_deleted_document(
    workspace: &mut Workspace,
    index: usize,
    original_path: &Path,
);
pub(crate) fn history_entry(
    workspace: &Workspace,
    index: usize,
    scroll_anchor: ScrollAnchor,
) -> Option<FileHistoryEntry>;
pub(crate) fn copy_tab_path(workspace: &Workspace, index: usize);
```

adapter 只通过 Workspace 的 entry/ID accessors 工作；返回 `bool` 仅表示 hydration
是否替换了 stub，不编码多种互斥状态。

**步骤：**

1. 提取以下 app-only 行为并保留薄 wrapper：
   - active/inactive stub 文件 hydration
   - 删除文件后的 recovery title 与 dirty snapshot ID
   - `FileHistoryEntry` 构造
   - tab context menu 的 CopyPath 系统剪贴板动作
2. 增加产品适配器测试，保持失败吞没、恢复标题和历史字段不变。

**验证：**

```bash
cargo fmt --all -- --check
cargo test -p textora-app --lib workspace_product
cargo test -p textora-app --lib workspace
cargo check -p textora-app --tests
bash scripts/check_architecture.sh
```

**提交：** `refactor(app): extract workspace product effects`

---

### Task 17：迁移删除恢复调用者

**文件：**

- Modify: `crates/app/src/app_lifecycle.rs`
- Modify: `crates/app/src/external_change_tests.rs`
- Modify: `crates/app/src/workspace_product.rs`

**步骤：**

1. 删除/外部变更路径调用 app adapter，而不是 Workspace 产品 wrapper。
2. 保持 file path 清除、disk revision 清除、dirty 标记、snapshot ID 和
   `恢复：` 标题行为。

**验证：**

```bash
cargo fmt --all -- --check
cargo test -p textora-app --lib external_change_tests
cargo test -p textora-app --lib app_lifecycle
cargo check -p textora-app --tests
```

**提交：** `refactor(app): route deleted files through workspace adapter`

---

### Task 18：迁移 history 与 context-menu 产品动作

**文件：**

- Modify: `crates/app/src/app_tab.rs`
- Modify: `crates/app/src/dispatch/tabs.rs`
- Modify: `crates/app/src/workspace.rs`

**步骤：**

1. file history 构造改为 adapter free function。
2. CopyPath 在 app adapter 执行；close/pin 仍由 Workspace typed effects 处理。
3. 删除 Workspace 的 `get_history_entry` 和产品 context-menu wrapper。

**验证：**

```bash
cargo fmt --all -- --check
cargo test -p textora-app --lib app_tab
cargo test -p textora-app --lib dispatch::tabs
cargo test -p textora-app --lib workspace_product
cargo check -p textora-app --tests
```

**提交：** `refactor(app): keep workspace product actions local`

---

### Task 19：让导航只产生通用 effect，hydration 留在 app

**文件：**

- Modify: `crates/app/src/workspace.rs`
- Modify: `crates/app/src/dispatch/tabs.rs`
- Modify: `crates/app/src/workspace_product.rs`

**步骤：**

1. `go_back/go_forward/switch_to/close` 不再读取文件。
2. 提取 App 内共享的 active-change 收尾 helper；`App::apply_workspace_effect`
   和 `App::handle_nav_effect` 都通过该 helper 在 active ID 变化后调用
   `hydrate_active_stub`，然后保持既有 layout/reshape/redraw 顺序，禁止只修
   其中一条导航入口。
3. 用失败文件、active stub、preview auto-close 和 back/forward 回归测试证明
   行为不变。
4. 删除 Workspace 的 lazy-load wrapper。

**验证：**

```bash
if sed -n '1,/^mod tests {/p' crates/app/src/workspace.rs |
  rg -n "DocumentView::from_file|workspace_product|lazy_load"; then
  exit 1
fi
cargo fmt --all -- --check
cargo test -p textora-app --lib workspace
cargo test -p textora-app --lib dispatch::tabs
cargo test -p textora-app --lib workspace_product
cargo check -p textora-app --tests
bash scripts/check_architecture.sh
```

Workspace 生产区文件加载扫描预期无匹配；测试夹具中的 `DocumentView` 按
Task 20–21 迁移。

**提交：** `refactor(workspace): emit pure navigation effects`

---

### Task 20：迁移跨文件产品 Workspace 测试夹具

**文件：**

- Modify: `crates/app/src/app_lifecycle.rs`
- Modify: `crates/app/src/app_tests.rs`

**步骤：**

1. 将 app 测试中的 `crate::workspace::Workspace::new()` 改为
   `crate::app_init::build_product_workspace()`。
2. Workspace 自身测试留给 Task 21 改为通用 registry/routes。
3. 扫描 Workspace 模块之外不再调用测试专用 `Workspace::new` 或
   `workspace.set_active_index_for_test`。

**验证：**

```bash
if rg -n "(crate::workspace::)?Workspace::new\(\)|workspace\.set_active_index_for_test" \
  crates/app/src --glob '*.rs' --glob '!workspace.rs'; then
  exit 1
fi
cargo fmt --all -- --check
cargo test -p textora-app --lib app_lifecycle
cargo test -p textora-app --lib app_tests
cargo check -p textora-app --tests
```

**提交：** `test(app): use explicit product workspace fixtures`

---

### Task 21：建立产品无关 Workspace 测试边界

**文件：**

- Modify: `crates/app/src/workspace.rs`
- Modify: `crates/app/src/workspace_tab_factory.rs`
- Modify: `crates/app/src/workspace_product.rs`

**步骤：**

1. Workspace 测试使用 shell `EditorPluginFactory` 和最小 `ViewRouteTable`；
   不再调用 `build_product_workspace`，并删除产品化的测试专用
   `Workspace::new()`。
2. md/mmap/typed、snapshot、history、clipboard 和 recovery 测试归入对应 app
   adapter。
3. 增加源码边界测试并确保扫描覆盖完整生产模块。

**验证：**

```bash
if rg -n "crate::(document_view|dirty_snapshot|file_safety|file_history|app_init|plugins)|DocumentView|NewDocumentKind|arboard|textora_markdown" \
  crates/app/src/workspace.rs; then
  exit 1
fi
cargo fmt --all -- --check
cargo test -p textora-app --lib workspace
cargo test -p textora-app --lib workspace_tab_factory
cargo test -p textora-app --lib workspace_product
cargo check -p textora-app --tests
bash scripts/check_architecture.sh
```

产品扫描预期无匹配。

**提交：** `test(workspace): enforce product free controller`

---

## Stage C：物理迁移通用 Workspace

### Task 22：移动 Workspace 到 appkit-shell

**文件：**

- Move: `crates/app/src/workspace.rs` →
  `crates/appkit-shell/src/workspace.rs`
- Modify: `crates/appkit-shell/src/lib.rs`
- Modify: `crates/app/src/lib.rs`

**跨 crate 接口：**

- public types: `Workspace`, `WorkspaceEffect`, `CloseTabDecision`；
- construction/plugin:
  `with_plugins`, `create_plugin_for_path`, `create_plugin_by_name`,
  `plugin_route_for_path`；
- PreparedTab lifecycle:
  `append_prepared_tab`, `open_prepared_tab`,
  `WorkspaceEffect::reconcile_runtime_store`, `WorkspaceEffect::nav_effect`；
- model/session inputs:
  `is_empty`, `len`, `tab_indices`, `active_index`, `active_entry`,
  `active_entry_mut`, `active_doc`, `active_doc_mut`, `entry`, `entry_mut`,
  `entry_doc`, `entry_doc_mut`, `entries`, `entry_title`,
  `suggested_file_name`, `clear_suggested_file_name`；
- stable IDs/navigation:
  `tab_id_at`, `index_of`, `tab_ids`, `find_by_path`, `switch_to`, `go_back`,
  `go_forward`, `has_back_history`, `has_forward_history`, `record_nav_step`；
- plugin/pin/close:
  `switch_plugin_with_runtime`, `toggle_target`, `is_toggled_for_plugin`,
  `try_close_entry`, `close_entry`, `toggle_pin`, `toggle_pin_at`, `is_pinned`,
  `pinned_indices`, `pinned_paths`, `restore_pinned`,
  `upgrade_preview_if_needed`。

未列出的 helper 保持 private；不得通过把字段或整个 registry/model 设为
`pub(crate)` 来绕过接口。

**步骤：**

1. 迁移前运行 app Workspace 全测试和 app tests check。
2. `git mv` 后将 import 改为 shell-local `prepared_tab`、`tab_runtime`、
   `tab_session`、`editor_plugin` 和 `appkit_core::navigator`。
3. shell 导出 `pub mod workspace`；app 在 `lib.rs` 保留 inline 兼容 facade：

   ```rust
   pub(crate) mod workspace {
       pub(crate) use appkit_shell::workspace::*;
       pub(crate) use crate::workspace_tab_factory::ViewportDimensions;
   }
   ```

   由此同时保留 `crate::workspace::{Workspace, WorkspaceEffect, ...}` 和 app-local
   `ViewportDimensions` 路径；不得把 `ViewportDimensions` 搬回 shell。
4. 只公开真实 app adapter/调用者需要的方法；model、registry、routes 和
   preview/history 字段保持私有。

**验证：**

```bash
cargo fmt --all -- --check
cargo test -p textora-appkit-shell workspace
cargo check -p textora-app --tests
cargo test -p textora-app --lib
bash scripts/check_architecture.sh
if rg -n "textora_|DocumentView|NewDocumentKind|dirty_snapshot|file_history|arboard|crate::app" \
  crates/appkit-shell/src/workspace.rs; then
  exit 1
fi
```

产品扫描预期无匹配。

**提交：** `refactor(appkit-shell): move workspace controller`

---

## Stage D：Workspace 阶段验证与审查

### Task 23：Workspace 阶段验证与审查

**文件：** 无实现文件。

**验证：**

```bash
rg -n "pub struct PreparedTab|pub struct Workspace" crates/app crates/appkit-shell
if rg -n "textora_|DocumentView|NewDocumentKind|dirty_snapshot|file_history|arboard|crate::app" \
  crates/appkit-shell/src/workspace.rs crates/appkit-shell/src/prepared_tab.rs; then
  exit 1
fi
if rg -n "std::fs|rfd::|WorkspaceStore|PersistedWorkspace" \
  crates/appkit-shell/src/workspace.rs crates/appkit-shell/src/prepared_tab.rs; then
  exit 1
fi
cargo fmt --all -- --check
bash scripts/check_architecture.sh
cargo check --workspace
cargo test -p textora-appkit-core
cargo test -p textora-appkit-shell
cargo test -p textora-app --lib
./scripts/verify.sh
```

阶段正式审查必须确认：

- `PreparedTab` 没有产品字段；
- PreparedTab 测试用不含 `..` 的穷尽解构证明 DTO 恰好只有
  `document` / `runtime` 两个字段；
- PreparedTab 安装与 `WorkspaceEffect` reconciliation 使 model/runtime store
  对所有生产生命周期保持双射；
- shell Workspace 不执行产品 I/O 或系统剪贴板；
- app adapters 保持恢复格式、typed untitled 和产品插件行为；
- `App` 仍将 Workspace/TabRuntimeStore 作为 sibling 字段，现有分拆借用不变；
- 可以开始独立的 `ShellRuntime` 字段迁移计划，并在该计划中把
  Workspace/TabRuntimeStore 作为同一 model/session 字段组迁移；本阶段本身未
  创建 ShellRuntime。
