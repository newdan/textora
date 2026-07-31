# notora 首版执行计划

日期：2026-07-30

状态：待执行

基线提交：`f6c2e249a33c29ee2e82bba18f1e344080c915f8`

依据：

- [`docs/specs/2026-07-30-notora-product-design.md`](../specs/2026-07-30-notora-product-design.md)
- [`docs/specs/2026-07-30-minimal-editor-runtime-design.md`](../specs/2026-07-30-minimal-editor-runtime-design.md)
- [`docs/plans/2026-07-30-minimal-editor-runtime-execution-plan.md`](./2026-07-30-minimal-editor-runtime-execution-plan.md)

## 1. 目标

在不改变 textora 产品定位和现有持久化格式的前提下，交付独立桌面 binary
`notora`。

首版完成后：

- 一个窗口管理一个普通文件夹工作区；
- `.txt`、`.md`、`.mmap.md` 是正文内容源；
- notora 复用 `EditorRuntime`、Markdown WYSIWYG 和 Mindmap 插件；
- 左侧导航、中间虚拟化卡片、右侧编辑器形成稳定三栏交互；
- 工作区笔记在编辑停止 800ms 后自动保存；
- 工作区外文件只在显式命令后保存；
- 搜索、目录、星标、标签、回收站和临时文件入口可用；
- SQLite catalog 可以由正文文件重建，同时保护星标、标签和回收站 metadata；
- 外部文件变化、保存竞态、catalog 损坏和异常退出都有明确恢复路径；
- 10,000 篇笔记下，中栏布局不随总数量线性退化；
- `notora-core` 保持 headless，shared crates 不含 notora 产品语义；
- textora 全量回归和 `./scripts/verify.sh` 继续通过。

## 2. 当前基线

本计划以基线提交为起点：

- `crates/appkit-shell/src/editor_runtime/` 已存在，提供文档会话、输入、
  `EditorFrame`、异步保存和文件安全能力；
- `crates/appkit-shell/tests/editor_runtime_fake_product.rs` 已覆盖第二消费者在非零
  编辑器矩形中的基本生命周期；
- `ProductHost`、`ProductWakeHandle` 和无 payload 的
  `ShellEvent::ProductWake` 已存在；
- `.mmap.md`、`.md`、`.txt` 的优先级路由已能通过 `ViewRouteTable` 表达；
- Markdown 与 Mindmap 插件仍由产品层注册，shared runtime 不依赖
  `textora-markdown`；
- `ui` 已有基础 widget、form、popup、overlay 和绘制协议，但没有 tree list、
  virtual card list、split button 或 splitter；
- workspace 尚无 `notora-core`、`notora-app` crate；
- workspace 尚未统一声明 `rusqlite`、`uuid` 等 notora 新依赖；
- `scripts/check_architecture.sh` 已保护 appkit 边界，但尚未检查新增 notora crate
  的 headless 和反向依赖。

目录存在不等于 N0 已完成。只有本计划 N0 的命令和契约审查全部通过，才允许创建
notora crate。

## 3. 实施约束

- 每个实现任务最多修改 3 个逻辑文件；`Cargo.lock` 是依赖声明的生成伴随文件，
  仍需随对应提交一起审查。
- 每次提交前至少运行：

  ```bash
  cargo fmt --all -- --check
  cargo check --workspace
  bash scripts/check_architecture.sh
  ```

- 行为变更先写失败测试；纯移动在移动前后运行同一组测试。
- 同一个 Bug 连续修改两次仍未解决，停止叠加防御性补丁，重新审查领域状态、
  所有权和事件时序。
- 禁止把 `LibraryState`、`NoteId`、`NavigationScope` 或 catalog handle 传入
  `ui`。
- 禁止在 widget paint、layout 或 hit-test 中执行 SQL、文件 I/O、Markdown
  解析或调用 `EditorRuntime`。
- 禁止在 `notora-core` 引入 `ui`、`winit`、`wgpu`、`render`、`shaping`、
  `appkit-shell` 或 `textora-markdown`。
- 禁止在 `appkit-core`、`appkit-shell`、`ui` 中加入 notora 产品类型、路径或
  自动保存策略。
- 禁止使用字符串 action、`Box<dyn Any>`、全局可变 callback 表或多个 bool
  表达互斥状态。
- 主线程不执行全量扫描、FTS rebuild、大文件摘要解析、catalog backup 或磁盘
  保存。
- SQLite 查询全部参数绑定；schema 只通过单调递增 migration 变更。
- 文件操作先验证规范化路径位于工作区根内，并拒绝 `.notora` 保留目录。
- 产品层只决定保存策略。实际写盘复用 `EditorRuntime` 的
  `PreparedDocumentSave`、expected disk revision 和异步保存机制，不实现第二套
  原子写入器。
- N2、N4、N6、N7 阶段结束必须运行 `./scripts/verify.sh`。

## 4. 目标模块布局

```text
crates/notora-core/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── domain.rs
    ├── workspace.rs
    ├── summary_parser.rs
    ├── catalog/
    │   ├── mod.rs
    │   ├── migration.rs
    │   ├── note_repository.rs
    │   ├── search_repository.rs
    │   └── metadata_repository.rs
    ├── scan.rs
    ├── reconciliation.rs
    ├── file_monitor.rs
    ├── note_command.rs
    ├── trash.rs
    └── backup.rs

crates/notora-app/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── main.rs
│   ├── app.rs
│   ├── events.rs
│   ├── render.rs
│   ├── product.rs
│   ├── state.rs
│   ├── action.rs
│   ├── effect_executor.rs
│   ├── shell/
│   │   ├── mod.rs
│   │   ├── layout.rs
│   │   └── input.rs
│   ├── editor_adapter.rs
│   ├── document_registry.rs
│   ├── workspace_controller.rs
│   ├── external_files.rs
│   ├── autosave.rs
│   ├── search_controller.rs
│   ├── paths.rs
│   ├── settings.rs
│   ├── session.rs
│   └── runtime_lru.rs
└── tests/
    ├── smoke.rs
    ├── save_policy.rs
    ├── search_flow.rs
    └── trash_flow.rs

crates/ui/src/widgets/
├── tree_list/
├── virtual_card_list/
├── split_button.rs
├── splitter.rs
└── status_state.rs
```

这是职责布局，不要求一次创建空文件。模块只在对应任务开始时建立；如果编译 spike
证明更适合合并私有模块，可以调整文件名，但不得改变依赖方向或扩大公共 API。

## 5. 实现前冻结的产品契约

以下类型先作为领域或产品层契约实现，后续不得用裸字符串或布尔组合替代：

- `WorkspaceId`、`NoteId`、`TagId`、`ExternalFileId`；
- `DocumentKind::{Text, Markdown, Mindmap}`；
- `DocumentOrigin::{Note, ExternalFile, UntitledExternal}`；
- `DocumentIdentity`，用于稳定映射 `DocumentIdentity → TabId`；
- `NoteLifecycle::{Active, Trashed { .. }}`；
- `NavigationScope`；
- `FocusTarget`；
- `ResponsiveLayoutMode`；
- `AutoSaveState`；
- `NotoraAction`、`NotoraEffect`；
- `NotoraProductEvent`，后台 payload 只存在于 notora 自有 channel；
- `CardQuery`、`SearchGeneration` 和分页 cursor；
- `NoteCommand` 及其类型化 request。

固定语义：

1. `DocumentOrigin` 是保存策略的唯一来源；
2. 中栏卡片选择不是 tab，只有实际打开的文档进入 `EditorRuntime`；
3. 单击卡片使用 `OpenDisposition::Preview`，编辑或 Enter 后升级为 persistent；
4. 同一 `DocumentIdentity` 同时最多映射一个活动 `TabId`；
5. Trash 不参与普通搜索、星标或标签 badge；
6. 笔记保存成功后异步更新 catalog，catalog 更新失败不回滚已成功写入的正文；
7. 外部文件永不进入工作区 catalog 或 `.notora/trash`；
8. 关闭、回收或 LRU 淘汰后到达的旧 generation、save、scan、index 结果必须丢弃。

## 6. 阶段总览

| 阶段 | 交付结果 | 进入下一阶段的门槛 |
|---|---|---|
| N0 | EditorRuntime 前置验收 | runtime fake product、架构检查和 textora 回归通过 |
| N1 | headless notora-core 基础 | workspace、catalog、scan、reconcile、watcher 测试通过 |
| N2 | notora App 与通用 UI 骨架 | 三栏静态界面、焦点和自定义 editor rect smoke 通过 |
| N3 | 工作区、新建、打开与插件路由 | 三类笔记和外部文件可正确打开、preview、固定 |
| N4 | 保存与文件安全 | 自动/手动保存分流、冲突、snapshot 和退出测试通过 |
| N5 | 搜索与虚拟卡片 | 中文搜索、分页、generation 和 10,000 卡片基准通过 |
| N6 | 星标、标签与回收站 | metadata 和 trash 全生命周期集成测试通过 |
| N7 | 设置、恢复、性能和发布验收 | 全量自动化、手工验收和 textora 回归通过 |

---

## N0：EditorRuntime 前置门禁

### Task N0-1：验证 runtime 第二消费者契约

**文件：** 无修改。

**步骤：**

1. 检查 `EditorRuntimeConfig` 由产品注入插件表、路由、设置、主题和 snapshot
   目录。
2. 检查 `EditorInputContext` 接收产品计算出的 `editor_rect`、focus 和 modal
   状态。
3. 检查 `PreparedDocumentSave` 携带 expected disk revision，completion 可安全
   应用于已关闭或已继续编辑的 tab。
4. 检查 fake product 不依赖 `textora-app`，并覆盖非零矩形、保存和迟到结果。
5. 如果任何契约缺失，先回到 ER 计划补齐；不得在 notora 中复制 runtime 能力。

**验证：**

```bash
cargo test -p textora-appkit-shell --test editor_runtime_fake_product
cargo test -p textora-appkit-shell editor_runtime
cargo tree -p textora-appkit-shell
```

### Task N0-2：验证 textora 基线和 shared 边界

**文件：** 无修改。

**步骤：**

1. 运行架构守卫并确认 shared crate 不含产品语义。
2. 运行 textora lib、smoke 和 render smoke。
3. 记录基线失败；不得把既有失败归因给 notora。

**验证：**

```bash
bash scripts/check_architecture.sh
cargo test -p textora-app --lib
cargo test -p textora-app --test smoke
cargo test -p textora-app --test render_smoke
```

### Task N0-3：冻结 notora 依赖版本

**文件：**

- 修改：`Cargo.toml`

**步骤：**

1. 在 `[workspace.dependencies]` 统一加入 `rusqlite`、`uuid`、`notify`、
   `blake3`、`dirs`、`rfd` 和 `tempfile`；已有依赖改为 workspace 引用时不升级
   版本。
2. `rusqlite` 启用 bundled SQLite、FTS5 所需 feature 和 backup API；先用最小
   编译 spike 验证目标平台 feature 名。
3. `uuid` 只启用 v4、serde；不引入 Tokio。
4. `Cargo.lock` 在 N1 首个实际使用这些依赖的 crate 创建时生成；审查不得意外
   升级 `winit`、`wgpu` 或其他基础依赖。

**验证：**

```bash
git diff -- Cargo.toml
```

**提交：**

```bash
git add Cargo.toml
git commit -m "build(notora): declare workspace dependencies"
```

---

## N1：建立 notora-core

### Task N1-1：创建 headless crate 和领域标识

**文件：**

- 新增：`crates/notora-core/Cargo.toml`
- 新增：`crates/notora-core/src/lib.rs`
- 新增：`crates/notora-core/src/domain.rs`

**步骤：**

1. 创建 package `notora-core`，只依赖 serde、uuid 和领域所需的 headless
   依赖。
2. 在 `domain.rs` 实现不透明 ID newtype、`DocumentKind`、
   `DocumentOrigin`、`DocumentIdentity`、`NoteLifecycle`、
   `NavigationScope` 和 `NoteSummary`。
3. 为 `.mmap.md` 优先于 `.md` 的分类顺序写失败测试后实现。
4. ID 的解析、显示和 serde round-trip 都在领域边界测试；业务代码不使用裸
   `String` 代替 ID。

**验证：**

```bash
cargo test -p notora-core domain
cargo tree -p notora-core
```

**提交：**

```bash
git add crates/notora-core
git commit -m "feat(notora-core): define document domain"
```

### Task N1-2：建立架构守卫

**文件：**

- 修改：`scripts/check_architecture.sh`
- 修改：`crates/notora-core/src/lib.rs`

**步骤：**

1. 增加 `notora-core` 禁止依赖 UI、窗口、GPU、render、shaping、shell 和
   Markdown 插件的 dependency guard。
2. 增加 `appkit-core`、`appkit-shell`、`ui` 禁止依赖
   `notora-core/notora-app` 的 guard。
3. 增加 shared source 禁止 `Notora`、`NavigationScope`、`.notora` 等产品
   token；拆分 guard 自身的字符串，避免自匹配。
4. 增加 notora source 禁止 `.edit+` 路径。
5. 在 `notora-core` crate-level 文档写清 headless 边界。

**验证：**

```bash
bash scripts/check_architecture.sh
cargo tree -p notora-core
```

**提交：**

```bash
git add scripts/check_architecture.sh crates/notora-core/src/lib.rs
git commit -m "test(notora): guard product boundaries"
```

### Task N1-3：实现工作区身份和安全路径

**文件：**

- 新增：`crates/notora-core/src/workspace.rs`
- 修改：`crates/notora-core/src/lib.rs`

**步骤：**

1. 定义 `WorkspaceDescriptor`、`WorkspaceManifest` 和 schema version 常量。
2. 验证根目录存在且为目录；创建或读取 `.notora/workspace.toml`。
3. 实现相对路径规范化、根目录包含检查和保留目录拒绝。
4. 覆盖绝对路径、`..`、符号链接逃逸、重复初始化、未知 schema 和损坏 TOML。
5. manifest 使用原子写入，不在其中保存正文或 UI session。

**验证：**

```bash
cargo test -p notora-core workspace
```

**提交：**

```bash
git add crates/notora-core/src/workspace.rs crates/notora-core/src/lib.rs
git commit -m "feat(notora-core): initialize safe workspaces"
```

### Task N1-4：实现 Unicode 安全的摘要解析

**文件：**

- 新增：`crates/notora-core/src/summary_parser.rs`
- 修改：`crates/notora-core/src/lib.rs`
- 修改：`crates/notora-core/Cargo.toml`

**步骤：**

1. 先写 TXT、Markdown、Mindmap、空文件、CRLF、中文和组合字符测试。
2. Markdown/Mindmap 取首个一级标题，TXT 取首个非空行，均以 stem fallback。
3. excerpt 跳过标题和空段落，去除有限且明确的 Markdown 展示标记。
4. 以 grapheme 截断；上限使用语义常量。
5. parser 只接收文本和文件 stem，不读取磁盘。

**验证：**

```bash
cargo test -p notora-core summary_parser
```

**提交：**

```bash
git add crates/notora-core/Cargo.toml crates/notora-core/src/lib.rs \
  crates/notora-core/src/summary_parser.rs
git commit -m "feat(notora-core): parse note summaries"
```

### Task N1-5：建立 catalog migration

**文件：**

- 新增：`crates/notora-core/src/catalog/mod.rs`
- 新增：`crates/notora-core/src/catalog/migration.rs`
- 修改：`crates/notora-core/src/lib.rs`

**步骤：**

1. 建立 catalog open 配置：WAL、foreign keys、busy timeout。
2. migration v1 创建 `notes`、`tags`、`note_tags`、`trash_entries` 和必要索引；
   FTS 表在 N5 独立 migration 中加入。
3. migration 在单个事务中执行，并记录单调递增 user/schema version。
4. 覆盖全新库、重复打开、旧版本迁移失败回滚和未来版本拒绝。
5. 不通过运行时探测列拼补 schema。

**验证：**

```bash
cargo test -p notora-core catalog::migration
```

**提交：**

```bash
git add crates/notora-core/src/lib.rs crates/notora-core/src/catalog
git commit -m "feat(notora-core): create catalog schema"
```

### Task N1-6：实现基础 note repository

**文件：**

- 新增：`crates/notora-core/src/catalog/note_repository.rs`
- 修改：`crates/notora-core/src/catalog/mod.rs`

**步骤：**

1. 定义 catalog row DTO 与领域类型之间的单一映射。
2. 实现按相对路径、`NoteId` 查询和 batch upsert。
3. 实现工作区根、直接目录、星标、Trash 的稳定分页查询。
4. 路径、ID 和查询值全部参数绑定。
5. 覆盖 rename 后 ID 稳定、重复路径拒绝和 transaction rollback。

**验证：**

```bash
cargo test -p notora-core catalog::note_repository
```

**提交：**

```bash
git add crates/notora-core/src/catalog
git commit -m "feat(notora-core): persist note catalog"
```

### Task N1-7：实现增量扫描

**文件：**

- 新增：`crates/notora-core/src/scan.rs`
- 修改：`crates/notora-core/src/lib.rs`
- 修改：`crates/notora-core/src/catalog/note_repository.rs`

**步骤：**

1. 定义 `ScanRequest`、批次、候选文件和 completion，不暴露 SQLite connection
   给调用者。
2. 递归扫描支持的后缀，完整忽略 `.notora` 和系统垃圾文件。
3. 先比较相对路径、mtime 和 size，仅在需要时读取内容并计算 hash/摘要。
4. 以有界批次提交 catalog，使 UI 可渐进更新。
5. 覆盖不支持文件、隐藏保留目录、未变化文件不重读和扫描中单文件失败。

**验证：**

```bash
cargo test -p notora-core scan
```

**提交：**

```bash
git add crates/notora-core/src/lib.rs crates/notora-core/src/scan.rs \
  crates/notora-core/src/catalog/note_repository.rs
git commit -m "feat(notora-core): scan workspace incrementally"
```

### Task N1-8：实现 reconciliation

**文件：**

- 新增：`crates/notora-core/src/reconciliation.rs`
- 修改：`crates/notora-core/src/lib.rs`
- 修改：`crates/notora-core/src/catalog/note_repository.rs`

**步骤：**

1. 定义文件新增、变化、missing、移动候选和删除的类型化差异。
2. catalog 行对应文件消失时先标记 missing，不在单个 watcher event 后立即删除。
3. 使用 file identity 或 content hash 尝试保持移动后的 `NoteId`；不确定时记录
   可诊断结果并按删除加新增处理。
4. 覆盖写盘成功但 catalog 更新失败后的重建。
5. 不覆盖星标、标签或 Trash metadata。

**验证：**

```bash
cargo test -p notora-core reconciliation
```

**提交：**

```bash
git add crates/notora-core/src/lib.rs crates/notora-core/src/reconciliation.rs \
  crates/notora-core/src/catalog/note_repository.rs
git commit -m "feat(notora-core): reconcile catalog with files"
```

### Task N1-9：实现工作区文件监控

**文件：**

- 新增：`crates/notora-core/src/file_monitor.rs`
- 修改：`crates/notora-core/src/lib.rs`
- 修改：`crates/notora-core/Cargo.toml`

**步骤：**

1. 沿用 `RecommendedWatcher` 与专用线程、`std::sync::mpsc` 模式。
2. 200ms 合并窗口以语义常量表示；完整忽略 `.notora`、原子保存临时文件和系统
   垃圾文件。
3. 输出规范化的 `WorkspaceFileBatch`，不在 watcher callback 中执行 SQL。
4. 支持干净 shutdown，receiver 断开后线程必须退出。
5. 用可注入事件源测试 debounce、rename pairing、自写事件和 shutdown。

**验证：**

```bash
cargo test -p notora-core file_monitor
./scripts/verify.sh
```

**提交：**

```bash
git add crates/notora-core/Cargo.toml crates/notora-core/src/lib.rs \
  crates/notora-core/src/file_monitor.rs
git commit -m "feat(notora-core): monitor workspace changes"
```

---

## N2：建立通用 UI 和 notora App 骨架

### Task N2-1：实现通用 TreeListWidget

**文件：**

- 新增：`crates/ui/src/widgets/tree_list/mod.rs`
- 新增：`crates/ui/src/widgets/tree_list/layout.rs`
- 修改：`crates/ui/src/widgets/mod.rs`

**步骤：**

1. 定义纯 UI 的 row key、label、icon、depth、expansion、selection 和 badge。
2. 实现稳定布局、scroll、hover、展开点击和选择 action。
3. key 只在当帧有效，不包含 `NoteId`、`TagId` 或路径领域语义。
4. 覆盖 DPI、深层缩进、badge、空列表和 hit-test。

**验证：**

```bash
cargo test -p textora-ui tree_list
bash scripts/check_architecture.sh
```

**提交：**

```bash
git add crates/ui/src/widgets/mod.rs crates/ui/src/widgets/tree_list
git commit -m "feat(ui): add generic tree list"
```

### Task N2-2：实现通用 VirtualCardListWidget

**文件：**

- 新增：`crates/ui/src/widgets/virtual_card_list/mod.rs`
- 新增：`crates/ui/src/widgets/virtual_card_list/layout.rs`
- 修改：`crates/ui/src/widgets/mod.rs`

**步骤：**

1. 定义稳定 card key、纯展示输入、selection 和 scroll state。
2. 只布局 viewport 与语义化 overscan 范围。
3. 列表替换后通过稳定 key 保持选择，scroll 与 selection 分离。
4. paint 只消费预计算标题、简介、时间、图标和标签摘要。
5. 覆盖 0、1、10,000 项的可见范围和键盘选择。

**验证：**

```bash
cargo test -p textora-ui virtual_card_list
```

**提交：**

```bash
git add crates/ui/src/widgets/mod.rs crates/ui/src/widgets/virtual_card_list
git commit -m "feat(ui): add virtual card list"
```

### Task N2-3：实现 split button 和 splitter

**文件：**

- 新增：`crates/ui/src/widgets/split_button.rs`
- 新增：`crates/ui/src/widgets/splitter.rs`
- 修改：`crates/ui/src/widgets/mod.rs`

**步骤：**

1. `SplitButtonWidget` 区分主操作和菜单操作，返回类型化 UI action。
2. `SplitterWidget` 只报告逻辑宽度变化和 drag lifecycle。
3. pointer capture 后即使指针离开原 rect 也能正确完成 drag。
4. 覆盖 DPI、最小/最大值 clamp、键盘触发和 modal 阻断。

**验证：**

```bash
cargo test -p textora-ui split_button
cargo test -p textora-ui splitter
```

**提交：**

```bash
git add crates/ui/src/widgets/mod.rs crates/ui/src/widgets/split_button.rs \
  crates/ui/src/widgets/splitter.rs
git commit -m "feat(ui): add split controls"
```

### Task N2-4：实现通用空状态和错误状态

**文件：**

- 新增：`crates/ui/src/widgets/status_state.rs`
- 修改：`crates/ui/src/widgets/mod.rs`

**步骤：**

1. 定义只包含标题、说明、图标和可选按钮的纯 UI 输入。
2. 区分 empty、loading 和 recoverable error 的展示状态，不包含 notora 错误类型。
3. action 只返回当帧 UI key，由产品层映射为重试、重新定位或选择工作区。
4. 覆盖窄宽度、长错误文本、无 action 和 DPI 布局。

**验证：**

```bash
cargo test -p textora-ui status_state
bash scripts/check_architecture.sh
```

**提交：**

```bash
git add crates/ui/src/widgets/mod.rs crates/ui/src/widgets/status_state.rs
git commit -m "feat(ui): add generic status states"
```

### Task N2-5：创建 notora-app crate

**文件：**

- 新增：`crates/notora-app/Cargo.toml`
- 新增：`crates/notora-app/src/lib.rs`
- 新增：`crates/notora-app/src/main.rs`

**步骤：**

1. 创建 package `notora-app` 和 binary `notora`。
2. 依赖 `notora-core`、appkit、ui、render、shaping、winit、wgpu 和
   `textora-markdown`；不依赖 `textora-app` 或 `textora-sync`。
3. 建立 `ci-no-fonts` feature 透传。
4. main 只负责事件循环、产品路径解析和 `NotoraApp` 启动；临时入口必须仍可
   编译。

**验证：**

```bash
cargo check -p notora-app
cargo tree -p notora-app
```

**提交：**

```bash
git add crates/notora-app
git commit -m "feat(notora): create application crate"
```

### Task N2-6：实现独立产品路径

**文件：**

- 新增：`crates/notora-app/src/paths.rs`
- 修改：`crates/notora-app/src/lib.rs`

**步骤：**

1. 定义 `NotoraPaths`，包含 config、settings、session、snapshots 和 catalog
   backup 目录。
2. 只在产品启动边界解析平台配置根；shared crate 不推导路径。
3. 明确不读取、迁移或回退到 `.edit+`。
4. 目录创建失败返回带目标路径的错误，不在构造函数中静默忽略。
5. 覆盖自定义根目录、各子路径隔离和目录创建失败。

**验证：**

```bash
cargo test -p notora-app paths
rg -n '\\.edit\\+' crates/notora-core crates/notora-app
```

**提交：**

```bash
git add crates/notora-app/src/lib.rs crates/notora-app/src/paths.rs
git commit -m "feat(notora): isolate product paths"
```

### Task N2-7：建立产品状态和 reducer

**文件：**

- 新增：`crates/notora-app/src/state.rs`
- 新增：`crates/notora-app/src/action.rs`
- 修改：`crates/notora-app/src/lib.rs`

**步骤：**

1. 分离 `LibraryState`、`LayoutState`，编辑状态只留在 `EditorRuntime`。
2. 定义 `FocusTarget`、overlay、responsive layout、navigation 和 card
   selection 的互斥 enum。
3. 定义 `NotoraAction`、`NotoraEffect` 和纯 reducer。
4. reducer 不执行 I/O、SQL、dialog 或 runtime 调用。
5. 覆盖搜索前范围恢复、Trash 禁止新建、标签范围新建附加标签和 Esc 层级。

**验证：**

```bash
cargo test -p notora-app state
cargo test -p notora-app action
```

**提交：**

```bash
git add crates/notora-app/src/lib.rs crates/notora-app/src/state.rs \
  crates/notora-app/src/action.rs
git commit -m "feat(notora): define product state reducer"
```

### Task N2-8：实现三栏布局几何

**文件：**

- 新增：`crates/notora-app/src/shell/mod.rs`
- 新增：`crates/notora-app/src/shell/layout.rs`
- 修改：`crates/notora-app/src/lib.rs`

**步骤：**

1. 实现左 220、中 340、右侧填充的默认逻辑布局和两个独立 splitter。
2. 左栏 clamp 180–320，中栏 clamp 260–520，右栏保留最小编辑宽度。
3. 窗口变窄时先压缩中栏，再进入 `ResponsiveLayoutMode`，不生成负 Rect。
4. editor、overlay、menu、tooltip rect 分层明确。
5. 覆盖 880×600、HiDPI、极窄窗口和 splitter 往返持久化精度。

**验证：**

```bash
cargo test -p notora-app shell::layout
```

**提交：**

```bash
git add crates/notora-app/src/lib.rs crates/notora-app/src/shell
git commit -m "feat(notora): compute three-pane layout"
```

### Task N2-9：实现静态 NotoraShell

**文件：**

- 新增：`crates/notora-app/src/render.rs`
- 修改：`crates/notora-app/src/shell/mod.rs`
- 修改：`crates/notora-app/src/lib.rs`

**步骤：**

1. 把产品状态显式映射为 tree row 和 card DTO，不把整个 `LibraryState` 传给
   widget。
2. 绘制搜索框、固定设置按钮、中栏标题栏、空卡片和右侧空状态。
3. 维护当帧 UI key 到 `NotoraAction` 的映射。
4. 产品 chrome、编辑器和 overlay 使用同一个 `EditorFrame`，overlay 最后绘制。

**验证：**

```bash
cargo test -p notora-app render
bash scripts/check_architecture.sh
```

**提交：**

```bash
git add crates/notora-app/src/lib.rs crates/notora-app/src/render.rs \
  crates/notora-app/src/shell/mod.rs
git commit -m "feat(notora): render static product shell"
```

### Task N2-10：接入 NotoraProduct 和后台事件通道

**文件：**

- 新增：`crates/notora-app/src/product.rs`
- 新增：`crates/notora-app/src/effect_executor.rs`
- 修改：`crates/notora-app/src/lib.rs`

**步骤：**

1. 实现 `ProductHost`，持有 notora 自有 sender/receiver 和服务 shutdown
   handles。
2. 定义带 payload 的 `NotoraProductEvent`，后台只发送无 payload 的
   `ProductWake`。
3. `drain_product_events` 丢弃不匹配 workspace/generation 的结果并返回通用
   `ShellEffect`。
4. effect executor 成为 dialog、catalog、worker 和 runtime 调用的唯一入口。
5. 覆盖事件排空、receiver 断开、重复 shutdown 和迟到事件。

**验证：**

```bash
cargo test -p notora-app product
cargo test -p notora-app effect_executor
```

**提交：**

```bash
git add crates/notora-app/src/lib.rs crates/notora-app/src/product.rs \
  crates/notora-app/src/effect_executor.rs
git commit -m "feat(notora): host product services"
```

### Task N2-11：接入窗口生命周期、焦点和 EditorRuntime

**文件：**

- 新增：`crates/notora-app/src/app.rs`
- 新增：`crates/notora-app/src/events.rs`
- 修改：`crates/notora-app/src/main.rs`

**步骤：**

1. `NotoraApp` 持有 `EditorRuntime`、`NotoraProduct` 和产品状态。
2. 窗口创建时注入 notora 插件 registry、route table、主题、设置和独立 snapshot
   路径。
3. 产品先处理 overlay、搜索、tree、card 和 splitter；剩余事件才进入 runtime。
4. 焦点不为 Editor 或 modal 打开时，字符和 IME 不得修改文档。
5. resize、redraw、about-to-wait 和 shutdown 使用 runtime 公共 API。

**验证：**

```bash
cargo check -p notora-app
cargo test -p notora-app events
```

**提交：**

```bash
git add crates/notora-app/src/app.rs crates/notora-app/src/events.rs \
  crates/notora-app/src/main.rs
git commit -m "feat(notora): embed editor runtime"
```

### Task N2-12：建立三栏 render smoke

**文件：**

- 新增：`crates/notora-app/tests/smoke.rs`
- 修改：`crates/notora-app/Cargo.toml`

**步骤：**

1. 验证 binary 产品名和窗口标题是 notora。
2. 在非零右栏矩形绘制 editor，断言左/中栏不侵入 editor rect。
3. 验证 modal、menu、tooltip 最后绘制。
4. 验证 card/search 焦点和 IME preedit 不进入 editor。

**验证：**

```bash
cargo test -p notora-app --test smoke
./scripts/verify.sh
```

**提交：**

```bash
git add crates/notora-app/Cargo.toml crates/notora-app/tests/smoke.rs
git commit -m "test(notora): cover three-pane runtime shell"
```

---

## N3：工作区、新建、打开与插件路由

### Task N3-1：实现工作区选择和扫描协调

**文件：**

- 新增：`crates/notora-app/src/workspace_controller.rs`
- 修改：`crates/notora-app/src/product.rs`
- 修改：`crates/notora-app/src/lib.rs`

**步骤：**

1. 实现选择、创建、关闭活动工作区的类型化命令。
2. 打开时依次验证目录、初始化 identity、打开 catalog、启动扫描和 watcher。
3. 每批扫描结果通过产品 channel 更新 `LibraryState`，不阻塞首帧。
4. 切换工作区生成新 generation，旧扫描和 watcher 结果失效。
5. 覆盖取消选择、无权限、损坏 manifest 和切换中的迟到结果。

**验证：**

```bash
cargo test -p notora-app workspace_controller
```

**提交：**

```bash
git add crates/notora-app/src/lib.rs crates/notora-app/src/product.rs \
  crates/notora-app/src/workspace_controller.rs
git commit -m "feat(notora): open note workspaces"
```

### Task N3-2：实现新建笔记领域命令

**文件：**

- 新增：`crates/notora-core/src/note_command.rs`
- 修改：`crates/notora-core/src/lib.rs`
- 修改：`crates/notora-core/src/catalog/note_repository.rs`

**步骤：**

1. 定义 `CreateNoteRequest` 和 `NoteCommand::Create`。
2. 根据 scope 解析目标目录；拒绝 Trash 和 `.notora`。
3. 生成 `未命名 N` 的唯一文件名，不覆盖现有文件。
4. TXT/MD 内容为空，MMAP.MD 内容为 `#`。
5. 文件落盘成功后 transaction 插入 catalog；失败由 reconciliation 可恢复。
6. 覆盖根目录、子目录、标签自动附加和并发同名。

**验证：**

```bash
cargo test -p notora-core note_command::create
```

**提交：**

```bash
git add crates/notora-core/src/lib.rs crates/notora-core/src/note_command.rs \
  crates/notora-core/src/catalog/note_repository.rs
git commit -m "feat(notora-core): create notes safely"
```

### Task N3-3：实现重命名和移动领域命令

**文件：**

- 修改：`crates/notora-core/src/note_command.rs`
- 修改：`crates/notora-core/src/catalog/note_repository.rs`

**步骤：**

1. 定义 `RenameNoteRequest`、`MoveNoteRequest` 和对应 `NoteCommand` variant。
2. 命令只接收 `NoteId` 与已验证的目标名称/目录，源路径从 catalog 精确解析。
3. 预检查目标位于工作区且不是 `.notora`，并拒绝同名覆盖。
4. 同文件系统原子 rename 成功后 transaction 更新相对路径，保持 `NoteId`。
5. catalog transaction 失败时返回明确补偿状态，由 reconciliation 修复。
6. 覆盖扩展名保护、目标冲突、目录逃逸、移动中断和 ID 稳定。

**验证：**

```bash
cargo test -p notora-core note_command::rename
cargo test -p notora-core note_command::move_note
```

**提交：**

```bash
git add crates/notora-core/src/catalog/note_repository.rs \
  crates/notora-core/src/note_command.rs
git commit -m "feat(notora-core): rename and move notes"
```

### Task N3-4：接入新建、重命名和移动 effect

**文件：**

- 修改：`crates/notora-app/src/action.rs`
- 修改：`crates/notora-app/src/effect_executor.rs`
- 修改：`crates/notora-app/src/state.rs`

**步骤：**

1. `Cmd/Ctrl+N` 按当前 scope 创建默认 MD，`Cmd/Ctrl+Shift+N` 打开类型菜单。
2. 新建、重命名和移动都转换为 `ExecuteNoteCommand`，UI 不直接调用文件系统。
3. 成功后更新目录树、card query 和 selection；重命名/移动已打开 Note 时同步更新
   runtime path 和 `DocumentOrigin`。
4. 当前标签 scope 新建成功后 attach 标签；Trash 不提供新建。
5. 失败保持旧状态并显示可恢复错误，不先乐观改写路径。

**验证：**

```bash
cargo test -p notora-app note_commands
```

**提交：**

```bash
git add crates/notora-app/src/action.rs crates/notora-app/src/effect_executor.rs \
  crates/notora-app/src/state.rs
git commit -m "feat(notora): execute note file commands"
```

### Task N3-5：建立文档身份到 TabId 的注册表

**文件：**

- 新增：`crates/notora-app/src/document_registry.rs`
- 修改：`crates/notora-app/src/lib.rs`

**步骤：**

1. 维护双向 `DocumentIdentity ↔ TabId` 映射和最近使用时间。
2. 同一路径的 external file 先 canonicalize，再去重。
3. runtime 关闭通知后移除映射；迟到通知不能复活旧 entry。
4. 覆盖 note rename 后 ID 稳定、external 路径别名和 tab reuse。

**验证：**

```bash
cargo test -p notora-app document_registry
```

**提交：**

```bash
git add crates/notora-app/src/lib.rs crates/notora-app/src/document_registry.rs
git commit -m "feat(notora): map documents to editor tabs"
```

### Task N3-6：实现文档准备和插件注册

**文件：**

- 新增：`crates/notora-app/src/editor_adapter.rs`
- 修改：`crates/notora-app/src/app.rs`
- 修改：`crates/notora-app/src/lib.rs`

**步骤：**

1. 注册纯文本、Markdown WYSIWYG 和 Mindmap 插件。
2. 路由优先级固定为 `.mmap.md` 高于 `.md`，`.txt` 使用 editor。
3. 后台读取并解码文件，成功后在主线程构造 `PreparedTab`。
4. 加载或解析失败返回可恢复错误，不安装半初始化 tab。
5. 首次打开注册 `DocumentOrigin`；共享 runtime 不接触该领域类型。

**验证：**

```bash
cargo test -p notora-app editor_adapter
cargo test -p textora-appkit-shell view_route
```

**提交：**

```bash
git add crates/notora-app/src/app.rs crates/notora-app/src/editor_adapter.rs \
  crates/notora-app/src/lib.rs
git commit -m "feat(notora): prepare routed editor documents"
```

### Task N3-7：实现卡片 preview 和 persistent 生命周期

**文件：**

- 修改：`crates/notora-app/src/action.rs`
- 修改：`crates/notora-app/src/effect_executor.rs`
- 修改：`crates/notora-app/src/document_registry.rs`

**步骤：**

1. 单击或键盘选择以 `Preview` 打开；已 persistent 的文档直接激活。
2. Enter 将 preview 升级为 persistent。
3. 首次内容变更通知立即升级，防止切卡丢失 dirty 文档。
4. selection、active tab 和加载 generation 分离。
5. 覆盖快速连续选择导致的乱序完成和 preview reuse。

**验证：**

```bash
cargo test -p notora-app preview
```

**提交：**

```bash
git add crates/notora-app/src/action.rs crates/notora-app/src/document_registry.rs \
  crates/notora-app/src/effect_executor.rs
git commit -m "feat(notora): manage card preview lifecycle"
```

### Task N3-8：实现外部文件 session

**文件：**

- 新增：`crates/notora-app/src/external_files.rs`
- 修改：`crates/notora-app/src/state.rs`
- 修改：`crates/notora-app/src/lib.rs`

**步骤：**

1. 定义 existing、untitled 和 missing external entry 状态。
2. external file 不进入 catalog、搜索、星标、标签或 Trash。
3. 打开同一 canonical path 只建立一个 entry 和 tab。
4. 从列表移除只关闭 session，不删除磁盘文件。
5. missing entry 提供 relocate/remove action。

**验证：**

```bash
cargo test -p notora-app external_files
```

**提交：**

```bash
git add crates/notora-app/src/external_files.rs crates/notora-app/src/lib.rs \
  crates/notora-app/src/state.rs
git commit -m "feat(notora): manage external file sessions"
```

### Task N3-9：接入打开对话框和系统打开事件

**文件：**

- 修改：`crates/notora-app/src/events.rs`
- 修改：`crates/notora-app/src/effect_executor.rs`
- 修改：`crates/notora-app/src/external_files.rs`

**步骤：**

1. `Cmd/Ctrl+O`、中栏打开按钮和系统 open event 进入同一类型化 effect。
2. 限制支持的可解码文本文件；二进制或错误编码显示错误状态。
3. 系统打开成功后明确切换到“文件”入口并激活文档。
4. 拖入路径复用同一验证函数，不单独实现打开逻辑。

**验证：**

```bash
cargo test -p notora-app open_external
```

**提交：**

```bash
git add crates/notora-app/src/effect_executor.rs crates/notora-app/src/events.rs \
  crates/notora-app/src/external_files.rs
git commit -m "feat(notora): open external text files"
```

### Task N3-10：完成打开与新建集成测试

**文件：**

- 修改：`crates/notora-app/tests/smoke.rs`
- 新增：`crates/notora-app/tests/open_flow.rs`

**步骤：**

1. 覆盖 TXT、MD WYSIWYG、MMAP.MD 路由。
2. 覆盖工作区根、子目录、标签范围新建位置，以及重命名/移动后的稳定
   `NoteId`。
3. 覆盖卡片 preview、Enter 固定、编辑固定和外部文件去重。
4. 断言中栏卡片不是 runtime tab，未选中的 10,000 篇笔记不创建
   `DocumentModel`。

**验证：**

```bash
cargo test -p notora-app --test open_flow
cargo test -p notora-app --test smoke
```

**提交：**

```bash
git add crates/notora-app/tests/open_flow.rs crates/notora-app/tests/smoke.rs
git commit -m "test(notora): cover create and open flows"
```

---

## N4：保存、冲突与恢复

### Task N4-1：实现类型化 autosave scheduler

**文件：**

- 新增：`crates/notora-app/src/autosave.rs`
- 修改：`crates/notora-app/src/lib.rs`

**步骤：**

1. 实现 `AutoSaveState::{Idle, Scheduled, Saving, Failed}`。
2. Note 内容变化刷新 800ms deadline；external 和 untitled 不安排。
3. deadline 携带 content revision，旧 deadline 不得保存新状态。
4. IME preedit 不调度，commit 后正常调度。
5. 使用可注入 clock 覆盖刷新、取消、立即保存和失败重试。

**验证：**

```bash
cargo test -p notora-app autosave
```

**提交：**

```bash
git add crates/notora-app/src/autosave.rs crates/notora-app/src/lib.rs
git commit -m "feat(notora): schedule note autosave"
```

### Task N4-2：接入 runtime 保存通知和异步完成

**文件：**

- 修改：`crates/notora-app/src/app.rs`
- 修改：`crates/notora-app/src/effect_executor.rs`
- 修改：`crates/notora-app/src/product.rs`

**步骤：**

1. 统一处理 `ContentChanged`、`SaveCompleted`、`SaveFailed` 和
   `PathChanged`。
2. deadline 到期调用 `prepare_save`，把不可变 save 交给 runtime 共享保存
   worker。
3. 应用 completion 后，如果当前 revision 更高则保持 dirty 并重新调度。
4. 保存成功发送 catalog reindex event；catalog 失败只标记待 reconcile。
5. 已关闭或已切换工作区的 completion 必须被安全忽略。

**验证：**

```bash
cargo test -p notora-app save_completion
```

**提交：**

```bash
git add crates/notora-app/src/app.rs crates/notora-app/src/effect_executor.rs \
  crates/notora-app/src/product.rs
git commit -m "feat(notora): execute asynchronous saves"
```

### Task N4-3：实现外部文件手动保存和 Save As

**文件：**

- 修改：`crates/notora-app/src/events.rs`
- 修改：`crates/notora-app/src/effect_executor.rs`
- 修改：`crates/notora-app/src/external_files.rs`

**步骤：**

1. `Cmd/Ctrl+S` 对 Note 触发立即 autosave，对 ExternalFile 手动保存，对
   UntitledExternal 打开 Save As。
2. Save As 成功后 canonicalize 路径、更新 origin 和 registry。
3. 取消 Save As 不修改 dirty、origin 或 tab。
4. 禁止把 external Save As 到工作区内后静默转换为 Note；首版仍保持 external
   身份，重新扫描后提示重复来源。

**验证：**

```bash
cargo test -p notora-app manual_save
```

**提交：**

```bash
git add crates/notora-app/src/effect_executor.rs crates/notora-app/src/events.rs \
  crates/notora-app/src/external_files.rs
git commit -m "feat(notora): save external files explicitly"
```

### Task N4-4：实现外部修改冲突流程

**文件：**

- 修改：`crates/notora-app/src/product.rs`
- 修改：`crates/notora-app/src/action.rs`
- 修改：`crates/notora-app/src/effect_executor.rs`

**步骤：**

1. `ConcurrentModification` 暂停对应 Note autosave，不自动覆盖磁盘。
2. 提供 reload、另存副本、重试和取消的类型化 action。
3. 干净文档外部变化可 reload；dirty 文档必须显式决策。
4. watcher 自写事件可以触发一致性校验，但不得重复制造冲突。
5. 工作区移除后所有 Note autosave 停止。

**验证：**

```bash
cargo test -p notora-app conflict
```

**提交：**

```bash
git add crates/notora-app/src/action.rs crates/notora-app/src/effect_executor.rs \
  crates/notora-app/src/product.rs
git commit -m "feat(notora): resolve external modifications"
```

### Task N4-5：实现 dirty snapshot 和有界退出

**文件：**

- 新增：`crates/notora-app/src/dirty_snapshot.rs`
- 修改：`crates/notora-app/src/app.rs`
- 修改：`crates/notora-app/src/lib.rs`

**步骤：**

1. snapshot 只写入 `NotoraPaths::snapshots_directory`。
2. 复用 `appkit-core::snapshot` 的 diff 格式和 disk revision 语义，由产品 adapter
   在后台执行，不复制 snapshot 编解码。
3. 退出时提交到期和立即要求的 Note 保存，并在语义化超时内 drain completion。
4. 未完成、失败或 external dirty 内容写 snapshot；成功保存的 revision 不重复写。
5. 下次启动列出可恢复 snapshot，不静默覆盖当前磁盘文件。
6. shutdown 顺序固定为停止新任务、保存/snapshot、停止 watcher/index、关闭
   runtime。

**验证：**

```bash
cargo test -p notora-app dirty_snapshot
cargo test -p notora-app shutdown
```

**提交：**

```bash
git add crates/notora-app/src/app.rs crates/notora-app/src/dirty_snapshot.rs \
  crates/notora-app/src/lib.rs
git commit -m "feat(notora): recover unsaved documents"
```

### Task N4-6：完成保存策略集成测试

**文件：**

- 新增：`crates/notora-app/tests/save_policy.rs`
- 修改：`crates/notora-app/tests/smoke.rs`

**步骤：**

1. 覆盖三类 Note 在 800ms idle 后保存。
2. 覆盖 external 经过多个 idle 周期仍不写盘，显式保存后才写。
3. 覆盖编辑发生在 save in-flight、external conflict、Save As 取消。
4. 覆盖退出超时产生 snapshot 和迟到 completion。

**验证：**

```bash
cargo test -p notora-app --test save_policy
cargo test -p notora-app --test smoke
./scripts/verify.sh
```

**提交：**

```bash
git add crates/notora-app/tests/save_policy.rs crates/notora-app/tests/smoke.rs
git commit -m "test(notora): verify save policy boundaries"
```

---

## N5：全文搜索和虚拟卡片

### Task N5-1：增加 FTS migration 和索引写入

**文件：**

- 修改：`crates/notora-core/src/catalog/migration.rs`
- 新增：`crates/notora-core/src/catalog/search_repository.rs`
- 修改：`crates/notora-core/src/catalog/mod.rs`

**步骤：**

1. 增加独立 migration 创建 FTS5 trigram 表和同步所需索引。
2. 启动测试明确验证 bundled SQLite 支持 FTS5/trigram；不支持时返回可诊断错误。
3. 实现 batch index、remove 和 rebuild，全部在后台 connection 执行。
4. 正文、title、path、tags 的更新边界明确。
5. migration 失败保持旧 schema 可打开或进入恢复，不留下半迁移状态。

**验证：**

```bash
cargo test -p notora-core catalog::search_repository
cargo test -p notora-core catalog::migration
```

**提交：**

```bash
git add crates/notora-core/src/catalog
git commit -m "feat(notora-core): index notes with fts"
```

### Task N5-2：实现中文搜索和排序

**文件：**

- 修改：`crates/notora-core/src/catalog/search_repository.rs`

**步骤：**

1. 标题和路径执行规范化模糊匹配，标签执行精确/前缀匹配。
2. 3 字符以上正文使用 trigram；1–2 字符只在受限候选集 fallback。
3. 固定标题、路径、标签、正文、mtime 的排序权重为语义常量。
4. 默认排除 Trash，空查询不执行 FTS。
5. 覆盖中文、拉丁文、组合字符、SQL 特殊字符和稳定 tie-break。

**验证：**

```bash
cargo test -p notora-core catalog::search_repository
```

**提交：**

```bash
git add crates/notora-core/src/catalog/search_repository.rs
git commit -m "feat(notora-core): search multilingual notes"
```

### Task N5-3：实现 IndexWorker

**文件：**

- 新增：`crates/notora-app/src/index_worker.rs`
- 修改：`crates/notora-app/src/product.rs`
- 修改：`crates/notora-app/src/lib.rs`

**步骤：**

1. 专用线程拥有后台 catalog connection 和索引命令 receiver。
2. 支持 scan batch、save completion、metadata change 和 rebuild 命令。
3. completion 携带 workspace generation；主线程丢弃旧结果。
4. shutdown 后不再 wake，断开 channel 时自然退出。
5. 全量 rebuild 不阻塞输入或渲染。

**验证：**

```bash
cargo test -p notora-app index_worker
```

**提交：**

```bash
git add crates/notora-app/src/index_worker.rs crates/notora-app/src/lib.rs \
  crates/notora-app/src/product.rs
git commit -m "feat(notora): index notes in background"
```

### Task N5-4：实现搜索 generation 和 debounce

**文件：**

- 新增：`crates/notora-app/src/search_controller.rs`
- 修改：`crates/notora-app/src/action.rs`
- 修改：`crates/notora-app/src/lib.rs`

**步骤：**

1. commit 后 120ms debounce；IME preedit 不提交搜索。
2. `Cmd/Ctrl+F` 聚焦全局搜索；非空查询进入 Search，清空恢复搜索前 scope。
3. 每次查询递增 generation，旧 completion 丢弃。
4. Esc 先清空搜索，再归还导航焦点。
5. 覆盖快速输入、清空、workspace 切换和乱序完成。

**验证：**

```bash
cargo test -p notora-app search_controller
```

**提交：**

```bash
git add crates/notora-app/src/action.rs crates/notora-app/src/lib.rs \
  crates/notora-app/src/search_controller.rs
git commit -m "feat(notora): coordinate debounced search"
```

### Task N5-5：接入分页 card query

**文件：**

- 修改：`crates/notora-app/src/product.rs`
- 修改：`crates/notora-app/src/state.rs`
- 修改：`crates/notora-app/src/effect_executor.rs`

**步骤：**

1. `NavigationScope` 映射为类型化 `CardQuery`。
2. 分页 cursor 使用稳定排序键，不用易错的裸 offset 处理实时更新。
3. 结果批次合并时通过 `DocumentIdentity` 保持 selection。
4. 搜索、Trash、目录和 external 使用明确不同的数据源。
5. loading、empty、partial 和 failed 使用互斥状态。

**验证：**

```bash
cargo test -p notora-app card_query
```

**提交：**

```bash
git add crates/notora-app/src/effect_executor.rs crates/notora-app/src/product.rs \
  crates/notora-app/src/state.rs
git commit -m "feat(notora): page document cards"
```

### Task N5-6：完成真实虚拟卡片渲染

**文件：**

- 修改：`crates/notora-app/src/render.rs`
- 修改：`crates/notora-app/src/shell/mod.rs`
- 修改：`crates/notora-app/src/events.rs`

**步骤：**

1. 将 `NoteSummary`/external entry 显式映射为 `DocumentCardInput`。
2. 滚动接近尾部时触发下一页 effect，paint 不查询数据。
3. Up/Down 更新选择并打开 preview；Enter 固定并聚焦 editor。
4. 标题、excerpt、mtime、kind、star 和 tag 摘要都来自预计算 DTO。
5. 列表刷新不重置 scroll 或选择。

**验证：**

```bash
cargo test -p notora-app virtual_cards
```

**提交：**

```bash
git add crates/notora-app/src/events.rs crates/notora-app/src/render.rs \
  crates/notora-app/src/shell/mod.rs
git commit -m "feat(notora): render virtual note cards"
```

### Task N5-7：建立搜索和 10,000 卡片基准

**文件：**

- 新增：`crates/notora-app/tests/search_flow.rs`
- 新增：`crates/notora-app/benches/library_bench.rs`
- 修改：`crates/notora-app/Cargo.toml`

**步骤：**

1. 构造可重复的 10,000 篇中英文笔记数据集。
2. benchmark 记录索引后首批搜索、分页查询和可见卡片布局。
3. 测试断言布局数量受 viewport/overscan 限制，不对总数量线性分配。
4. 性能数值只作为可追踪基线；不要在无稳定 CI 环境时写脆弱毫秒断言。

**验证：**

```bash
cargo test -p notora-app --test search_flow
cargo bench -p notora-app --bench library_bench --no-run
```

**提交：**

```bash
git add crates/notora-app/Cargo.toml crates/notora-app/benches/library_bench.rs \
  crates/notora-app/tests/search_flow.rs
git commit -m "perf(notora): baseline large libraries"
```

---

## N6：星标、标签和回收站

### Task N6-1：实现星标和标签 repository

**文件：**

- 新增：`crates/notora-core/src/catalog/metadata_repository.rs`
- 修改：`crates/notora-core/src/catalog/mod.rs`

**步骤：**

1. 实现单 transaction 星标切换、标签创建/重命名/删除和 note-tag 关联。
2. 标签名称按 Unicode 规范化后唯一，展示名独立保存。
3. attach/detach 幂等，删除标签只删关联。
4. Trash 保留 metadata，但普通 badge 和查询排除 Trash。
5. 覆盖 transaction rollback、名称冲突和恢复后 metadata 保留。

**验证：**

```bash
cargo test -p notora-core catalog::metadata_repository
```

**提交：**

```bash
git add crates/notora-core/src/catalog
git commit -m "feat(notora-core): persist note metadata"
```

### Task N6-2：接入星标和标签 action/effect

**文件：**

- 修改：`crates/notora-app/src/action.rs`
- 修改：`crates/notora-app/src/effect_executor.rs`
- 修改：`crates/notora-app/src/state.rs`

**步骤：**

1. 卡片菜单和编辑器 metadata 区发出相同类型化 action。
2. 成功后更新当前 card、导航 badge 和相关 query；失败保留原状态。
3. 标签范围新建自动 attach 当前 `TagId`。
4. 星标入口新建不自动星标。
5. 删除标签必须经过产品确认 overlay。

**验证：**

```bash
cargo test -p notora-app metadata_actions
```

**提交：**

```bash
git add crates/notora-app/src/action.rs crates/notora-app/src/effect_executor.rs \
  crates/notora-app/src/state.rs
git commit -m "feat(notora): edit stars and tags"
```

### Task N6-3：完成目录和标签导航树

**文件：**

- 修改：`crates/notora-app/src/render.rs`
- 修改：`crates/notora-app/src/events.rs`
- 修改：`crates/notora-app/src/state.rs`

**步骤：**

1. 只显示包含支持笔记或子目录的目录，按名称稳定排序。
2. 展开状态以相对路径保存；标签以 `TagId` 保持改名后的选择。
3. 当帧 tree key 映射回 `NavigationScope`，不把领域 ID 放入 UI crate。
4. 设置按钮固定底部，不随 tree scroll。
5. 覆盖目录删除、标签改名和当前 scope 失效后的 fallback。

**验证：**

```bash
cargo test -p notora-app navigation_tree
bash scripts/check_architecture.sh
```

**提交：**

```bash
git add crates/notora-app/src/events.rs crates/notora-app/src/render.rs \
  crates/notora-app/src/state.rs
git commit -m "feat(notora): navigate directories and tags"
```

### Task N6-4：实现回收站领域操作

**文件：**

- 新增：`crates/notora-core/src/trash.rs`
- 修改：`crates/notora-core/src/lib.rs`
- 修改：`crates/notora-core/src/catalog/note_repository.rs`

**步骤：**

1. `move_to_trash` 只接收 `NoteId`，从 catalog 解析精确源路径。
2. 原子移动到 `.notora/trash/<note-id>/<file-name>` 后 transaction 记录
   original/trash path 和时间。
3. restore 检查原路径；冲突返回类型化结果，不覆盖。
4. permanent delete 只接受已在 Trash 的精确 entry。
5. 批量清空先解析固定目标列表，不对工作区根执行递归删除。
6. 覆盖中断补偿、路径逃逸、重复操作和 metadata 保留。

**验证：**

```bash
cargo test -p notora-core trash
```

**提交：**

```bash
git add crates/notora-core/src/catalog/note_repository.rs \
  crates/notora-core/src/lib.rs crates/notora-core/src/trash.rs
git commit -m "feat(notora-core): manage recoverable trash"
```

### Task N6-5：接入 dirty 文档回收流程

**文件：**

- 修改：`crates/notora-app/src/action.rs`
- 修改：`crates/notora-app/src/effect_executor.rs`
- 修改：`crates/notora-app/src/document_registry.rs`

**步骤：**

1. dirty Note 必须先保存成功，保存失败则取消移入 Trash。
2. 移动成功后关闭对应 runtime、移除映射并稳定更新中栏选择。
3. restore path conflict 打开重命名恢复/取消 overlay。
4. permanent delete 和清空 Trash 必须显式确认。
5. external file 永远不进入此流程。

**验证：**

```bash
cargo test -p notora-app trash_actions
```

**提交：**

```bash
git add crates/notora-app/src/action.rs crates/notora-app/src/document_registry.rs \
  crates/notora-app/src/effect_executor.rs
git commit -m "feat(notora): coordinate trash with editors"
```

### Task N6-6：建立 catalog metadata backup

**文件：**

- 新增：`crates/notora-core/src/backup.rs`
- 修改：`crates/notora-core/src/lib.rs`
- 修改：`crates/notora-core/src/catalog/mod.rs`

**步骤：**

1. 星标、标签、Trash metadata transaction 后调度一致性 backup。
2. 使用 SQLite backup API，不复制活动 WAL 文件集合。
3. backup 写入产品注入的目标目录，不在 core 推导用户 home。
4. migration 前创建可验证 backup；保留数量使用语义配置。
5. 覆盖 backup 中断、损坏源库和从最近有效 backup 读取 metadata。

**验证：**

```bash
cargo test -p notora-core backup
```

**提交：**

```bash
git add crates/notora-core/src/backup.rs crates/notora-core/src/catalog/mod.rs \
  crates/notora-core/src/lib.rs
git commit -m "feat(notora-core): back up catalog metadata"
```

### Task N6-7：接入 metadata backup 调度

**文件：**

- 修改：`crates/notora-app/src/product.rs`
- 修改：`crates/notora-app/src/effect_executor.rs`

**步骤：**

1. 产品层把 `NotoraPaths::catalog_backups_directory` 显式注入 backup service。
2. 星标、标签、Trash metadata transaction 成功后合并 backup 请求，禁止每次点击
   在 UI 线程同步复制。
3. migration 前 backup 不走 debounce，必须先完成或返回阻断性错误。
4. completion 经产品 channel 返回；失败显示可诊断状态，但不伪造成功备份。
5. shutdown 有界 drain 已提交 backup，线程随后干净退出。

**验证：**

```bash
cargo test -p notora-app catalog_backup
```

**提交：**

```bash
git add crates/notora-app/src/effect_executor.rs crates/notora-app/src/product.rs
git commit -m "feat(notora): schedule catalog backups"
```

### Task N6-8：完成 metadata 和 Trash 集成测试

**文件：**

- 新增：`crates/notora-app/tests/trash_flow.rs`
- 修改：`crates/notora-app/tests/search_flow.rs`

**步骤：**

1. 覆盖星标、标签创建/改名/删除和标签范围新建。
2. 覆盖 dirty save 后 Trash、restore、restore conflict 和 permanent delete。
3. 断言 Trash 从普通搜索、星标和 badge 排除，但恢复后 metadata 仍在。
4. 断言 external file 无法进入 Trash。

**验证：**

```bash
cargo test -p notora-app --test trash_flow
cargo test -p notora-app --test search_flow
./scripts/verify.sh
```

**提交：**

```bash
git add crates/notora-app/tests/search_flow.rs crates/notora-app/tests/trash_flow.rs
git commit -m "test(notora): verify metadata and trash flows"
```

---

## N7：设置、会话恢复、性能与发布验收

### Task N7-1：实现产品设置

**文件：**

- 新增：`crates/notora-app/src/settings.rs`
- 修改：`crates/notora-app/src/lib.rs`

**步骤：**

1. 使用 N2 已建立的 `NotoraPaths::settings_file`。
2. settings 使用带 default、deny unknown policy 和 schema version 的 serde
   DTO。
3. 支持 Appearance、Editor、Interface、Workspace；运行时设置显式映射到
   `ui::Settings`。
4. 使用原子写入，损坏设置 fallback 并保留可诊断错误。
5. settings 不包含 textora sync、library registry 或 `.edit+` 兼容字段。

**验证：**

```bash
cargo test -p notora-app settings
rg -n '\\.edit\\+' crates/notora-core crates/notora-app
```

**提交：**

```bash
git add crates/notora-app/src/lib.rs crates/notora-app/src/settings.rs
git commit -m "feat(notora): persist product settings"
```

### Task N7-2：实现 session 持久化

**文件：**

- 新增：`crates/notora-app/src/session.rs`
- 修改：`crates/notora-app/src/lib.rs`

**步骤：**

1. 保存工作区 path/ID、external paths、最后 scope/document、展开目录、逻辑栏宽
   和窗口几何。
2. session 不保存正文、SQLite connection、`TabId` 或 runtime 内部状态。
3. 写入使用 debounce 和原子替换；关闭时 flush。
4. 读取时逐项验证，单个失效 external path 不阻止其他状态恢复。
5. 覆盖版本迁移、损坏 TOML、DPI 变化和越界窗口位置。

**验证：**

```bash
cargo test -p notora-app session
```

**提交：**

```bash
git add crates/notora-app/src/lib.rs crates/notora-app/src/session.rs
git commit -m "feat(notora): persist product sessions"
```

### Task N7-3：实现渐进式启动恢复

**文件：**

- 修改：`crates/notora-app/src/app.rs`
- 修改：`crates/notora-app/src/workspace_controller.rs`
- 修改：`crates/notora-app/src/session.rs`

**步骤：**

1. 按设置、窗口/runtime、session、工作区/catalog、扫描、布局、external、scope、
   最后文档的顺序恢复。
2. 先提供可用首帧，再渐进展示 scan/index 结果。
3. 只按需打开最后文档，不为整个 catalog 创建 `DocumentModel`。
4. session 的 WorkspaceId 与磁盘不一致时进入明确的重新选择流程。
5. 最后文档消失时保留 scope，显示可恢复空状态。

**验证：**

```bash
cargo test -p notora-app session_restore
```

**提交：**

```bash
git add crates/notora-app/src/app.rs crates/notora-app/src/session.rs \
  crates/notora-app/src/workspace_controller.rs
git commit -m "feat(notora): restore sessions progressively"
```

### Task N7-4：实现 editor runtime LRU

**文件：**

- 新增：`crates/notora-app/src/runtime_lru.rs`
- 修改：`crates/notora-app/src/document_registry.rs`
- 修改：`crates/notora-app/src/lib.rs`

**步骤：**

1. 上限从语义配置读取，按最近激活顺序选择候选。
2. dirty、saving、active、preview 正在升级和 pinned tab 不参与淘汰。
3. 淘汰只关闭干净非活动 runtime；卡片和 catalog 不受影响。
4. 再次选择被淘汰文档时按正常 prepare 流程打开。
5. 覆盖所有 tab 都不可淘汰、迟到 save completion 和 registry 清理。

**验证：**

```bash
cargo test -p notora-app runtime_lru
```

**提交：**

```bash
git add crates/notora-app/src/document_registry.rs crates/notora-app/src/lib.rs \
  crates/notora-app/src/runtime_lru.rs
git commit -m "feat(notora): bound editor runtime usage"
```

### Task N7-5：完成设置 overlay 和响应式布局

**文件：**

- 新增：`crates/notora-app/src/settings_overlay.rs`
- 修改：`crates/notora-app/src/render.rs`
- 修改：`crates/notora-app/src/shell/layout.rs`

**步骤：**

1. 使用通用 form 控件映射 notora settings DTO。
2. `Cmd/Ctrl+,` 与左栏固定按钮进入同一个设置 action；设置 overlay 阻断 editor
   键盘、IME 和 pointer 输入。
3. 窄窗口使用互斥 layout mode：左栏 overlay、中栏保留、选择后显示 editor 和返回
   操作。
4. 恢复栏宽按逻辑像素 clamp，DPI 变化不累计误差。
5. 覆盖 880×600、低于阈值、overlay Esc 和 modal paint order。

**验证：**

```bash
cargo test -p notora-app settings_overlay
cargo test -p notora-app responsive_layout
```

**提交：**

```bash
git add crates/notora-app/src/render.rs crates/notora-app/src/settings_overlay.rs \
  crates/notora-app/src/shell/layout.rs
git commit -m "feat(notora): finish settings and compact layout"
```

### Task N7-6：实现 catalog 损坏恢复

**文件：**

- 修改：`crates/notora-core/src/backup.rs`
- 修改：`crates/notora-core/src/catalog/mod.rs`
- 修改：`crates/notora-core/src/reconciliation.rs`

**步骤：**

1. 打开 catalog 时运行一致性检查并分类 migration、I/O、corruption 错误。
2. 损坏时先从最近有效 backup 恢复用户 metadata，再从正文扫描重建派生字段和
   FTS。
3. 无有效 backup 时保留正文可访问，明确报告 metadata 可能丢失。
4. 恢复过程写入新数据库后原子替换，不在损坏库上原地拼补。
5. 覆盖损坏 WAL、截断数据库、无 backup 和恢复中断。

**验证：**

```bash
cargo test -p notora-core catalog_recovery
```

**提交：**

```bash
git add crates/notora-core/src/backup.rs crates/notora-core/src/catalog/mod.rs \
  crates/notora-core/src/reconciliation.rs
git commit -m "feat(notora-core): recover damaged catalogs"
```

### Task N7-7：完成性能和异常恢复验收

**文件：**

- 修改：`crates/notora-app/benches/library_bench.rs`
- 新增：`crates/notora-app/tests/recovery_flow.rs`
- 修改：`crates/notora-app/Cargo.toml`

**步骤：**

1. 记录 10,000 笔记扫描、已建索引搜索、分页、滚动和 tab 切换基线。
2. 验证后台 scan/rebuild 时输入、IME 和光标动画仍可调度。
3. 覆盖工作区被移除、catalog 损坏、watcher 断开、保存权限失败和异常退出。
4. 验证切换已打开文档不重新读取磁盘。
5. 把测量环境和结果记录到测试输出或后续 benchmark 基线文档，不写无依据承诺。

**验证：**

```bash
cargo test -p notora-app --test recovery_flow
cargo bench -p notora-app --bench library_bench
```

**提交：**

```bash
git add crates/notora-app/Cargo.toml crates/notora-app/benches/library_bench.rs \
  crates/notora-app/tests/recovery_flow.rs
git commit -m "perf(notora): validate large workspace recovery"
```

### Task N7-8：最终自动化、架构和手工验收

**文件：**

- 修改：`scripts/check_architecture.sh`
- 新增：`docs/plans/2026-07-30-notora-acceptance-results.md`

**步骤：**

1. 收紧最终 dependency/source guard：
   - `notora-core` 保持 headless；
   - shared crates 不含 notora 语义；
   - `ui` 不依赖 notora；
   - notora 不依赖 `textora-app`、`textora-sync` 或 `.edit+`。
2. 运行全部格式、clippy、单测、集成测试、render smoke 和 workspace 验证。
3. 在 macOS 至少完成首版验收清单；其他目标平台记录已自动化验证与待实机项。
4. 记录工作区选择、新建三类文件、WYSIWYG、Mindmap、自动保存、external 手动
   保存、搜索、metadata、Trash、冲突、重启恢复和 10,000 卡片结果。
5. 未通过项必须保留为未完成，不得用“已知问题”绕过首版完成定义。

**验证：**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p notora-core
cargo test -p notora-app
cargo test -p textora-app --lib
cargo test -p textora-app --test smoke
cargo test -p textora-app --test render_smoke
bash scripts/check_architecture.sh
./scripts/verify.sh
```

**提交：**

```bash
git add scripts/check_architecture.sh \
  docs/plans/2026-07-30-notora-acceptance-results.md
git commit -m "test(notora): record first release acceptance"
```

---

## 7. 每阶段统一审查清单

- [ ] 本阶段每个任务修改不超过 3 个逻辑文件。
- [ ] 新行为先有失败测试，纯移动前后运行同一测试。
- [ ] 所有新增名称精准表达领域含义，无宽泛 `data/info/temp/res/flag`。
- [ ] 互斥状态使用 enum，没有组合 bool。
- [ ] 函数职责单一，超过 50 行已审查拆分。
- [ ] 错误通过类型传播，没有无理由 `.unwrap()`。
- [ ] UI 输入是纯 DTO，不持有领域状态、SQL handle 或 runtime。
- [ ] 主线程没有扫描、SQL rebuild、文件读取或保存。
- [ ] 所有路径经过工作区包含检查，`.notora` 保留目录不可操作。
- [ ] 所有 SQL 使用参数绑定，catalog 写操作使用 transaction。
- [ ] generation/revision/workspace identity 能丢弃迟到结果。
- [ ] 后台线程可停止，shutdown 不泄漏 watcher 或 channel。
- [ ] `cargo fmt --all -- --check` 通过。
- [ ] `cargo check --workspace` 通过。
- [ ] `bash scripts/check_architecture.sh` 通过。
- [ ] 重大阶段 `./scripts/verify.sh` 通过。

## 8. 最终完成定义

只有同时满足以下条件，notora 首版才算完成：

1. `notora` 独立 binary 启动并展示三栏产品界面；
2. 可创建、选择、恢复单个普通文件夹工作区；
3. 可新建、打开和编辑 TXT、MD、MMAP.MD；
4. MD 使用现有 WYSIWYG，MMAP.MD 使用现有 Mindmap；
5. 中栏展示标题、简介、mtime、类型、星标和标签摘要；
6. preview、persistent、切换和 LRU 不丢失编辑状态；
7. Note 在 800ms idle 后自动保存；
8. ExternalFile 只在显式保存后写盘，UntitledExternal 正确执行 Save As；
9. expected disk revision 阻止自动覆盖外部修改；
10. 搜索支持标题、路径、正文、标签和中文短查询；
11. 星标、标签和 Trash 全生命周期可用；
12. Trash restore 不静默覆盖冲突路径，永久删除只作用于精确目标；
13. 重启恢复工作区、external 列表、scope、最后文档、展开状态、栏宽和窗口几何；
14. catalog 损坏不影响正文直接访问，metadata 有一致性 backup/恢复路径；
15. 10,000 笔记下只布局可见卡片，后台工作不阻塞编辑输入；
16. `notora-core` 保持 headless；
17. `ui/appkit` 不依赖或理解 notora；
18. notora 不读取 `.edit+`，textora 持久化格式和行为不变；
19. 自动化和手工验收结果已记录；
20. `./scripts/verify.sh` 通过。
