# notora 笔记 App 完整设计

日期：2026-07-30

状态：方案初稿，待评审；实施依赖 EditorRuntime

依赖设计：
[`2026-07-30-minimal-editor-runtime-design.md`](./2026-07-30-minimal-editor-runtime-design.md)

## 1. 产品定义

notora 是基于 textora 编辑框架开发的桌面笔记 App。

核心能力：

- 以普通文件夹作为笔记工作区；
- 支持 `.txt`、`.md` 和 `.mmap.md` 笔记；
- Markdown 预览与编辑复用 textora 现有 WYSIWYG 模式；
- 复用 textora 的纯文本和 Mindmap 编辑能力；
- 使用左侧导航、中间文件卡片、右侧编辑器的三栏布局；
- 笔记自动保存；
- 工作区之外的外部文件手动保存；
- 支持搜索、目录、星标、回收站、标签和临时文件列表。

产品名和 binary 名统一为 `notora`。

## 2. 已确认决策

| 事项 | 决策 |
|---|---|
| 产品名 | `notora` |
| 产品形态 | 独立 binary，不改变现有 textora 产品定位 |
| Markdown 体验 | 复用现有 WYSIWYG 编辑/预览 |
| 笔记保存 | 编辑停止后自动保存 |
| 外部文件保存 | 手动保存 |
| 笔记内容 | 普通 `.txt/.md/.mmap.md` 文件是内容源 |
| 星标和标签 | notora metadata，不修改正文/front matter |
| 回收站 | 工作区内部可恢复回收站 |
| 编辑状态 | 复用 `EditorRuntime`、`Workspace`、`TabId` 和插件 runtime |

## 3. 范围

### 3.1 首版包含

- 创建、选择和恢复一个活动工作区；
- 递归展示工作区子目录；
- 新建 TXT、MD、MMAP.MD 笔记；
- 打开、编辑、自动保存笔记；
- 打开、编辑、手动保存外部文本文件；
- 搜索标题、路径、正文和标签；
- 星标；
- 标签的创建、分配、移除和按标签筛选；
- 移入回收站、恢复、永久删除；
- 文件卡片标题、简介和最后修改时间；
- 会话恢复、窗口/栏宽/展开状态持久化；
- 文件系统外部变更检测；
- 深浅主题和基础编辑设置。

### 3.2 首版不包含

- 云同步、Syncthing 产品集成或多人协作；
- Wikilink、反向链接、关系图谱；
- 所见即所得富文本以外的块数据库；
- 图片/PDF/音视频资源管理；
- 移动端；
- 同一窗口同时聚合多个工作区；
- Git 管理界面；
- Markdown front matter 标签双向同步；
- 非文本二进制文件预览；
- 插件市场或动态产品插件。

## 4. 目标架构

```text
crates/core
  TextBuffer、编码、语法基础

crates/appkit-core
  DocumentModel、TabId、文件安全、持久化基础

crates/ui
  纯 widget、布局、主题、绘制协议
  ├── tree_list
  ├── virtual_card_list
  ├── split_button
  └── splitter

crates/appkit-shell
  EditorRuntime、插件会话、窗口、GPU、输入、IME、reshape

crates/markdown
  Markdown WYSIWYG、Mindmap 插件

crates/notora-core
  无窗口笔记领域
  ├── workspace
  ├── catalog
  ├── search
  ├── metadata
  ├── trash
  ├── file_monitor
  └── summary_parser

crates/notora-app
  notora 产品
  ├── NotoraApp / NotoraProduct
  ├── NotoraShell 三栏组合
  ├── action/effect reducer
  ├── EditorRuntime adapter
  ├── autosave scheduler
  ├── settings/session
  └── main
```

依赖方向：

```text
core ← appkit-core ← appkit-shell
ui ──────────────────┘

notora-core ───────────────┐
appkit-core + appkit-shell ├─→ notora-app
ui + textora-markdown ─────┘
```

硬性边界：

- `notora-core` 不依赖 `ui`、`winit`、`wgpu`、`render` 或 Markdown 插件；
- `ui` 不依赖 `notora-core` 或 `notora-app`；
- `appkit-core/appkit-shell` 不依赖 notora；
- notora 领域类型只存在于 `notora-core/notora-app`；
- `notora-app` 负责把领域状态映射为 UI 纯数据输入。

### 4.1 技术选型

- Rust workspace crate，沿用仓库当前 Rust edition 和 MSRV；
- SQLite 通过 `rusqlite` 接入，使用 bundled SQLite 以保证 FTS5/trigram
  能力在各平台一致；
- 稳定领域 ID 使用 `uuid`，只在领域边界解析和格式化；
- 文件监控沿用 `notify::RecommendedWatcher`；
- 后台扫描、索引和保存沿用当前项目的专用线程 + `std::sync::mpsc` 模式，
  首版不引入 Tokio runtime；
- 配置和轻量 session 使用 serde + TOML；
- catalog 一致性备份使用 SQLite backup API，不复制正在运行中的 WAL 文件集合。

具体依赖版本在实施阶段基于 workspace 现有依赖统一选择，不在产品代码内产生多份
版本。

## 5. 三类状态

notora 明确分离：

### 5.1 LibraryState

负责：

- 当前工作区；
- 目录树；
- 笔记元数据；
- 搜索结果；
- 星标、标签和回收站；
- 外部文件列表；
- 后台扫描、索引和 watcher 状态。

不持有光标、Viewport、插件或 GPU 状态。

### 5.2 EditorState

由 `EditorRuntime` 持有：

- 打开的 `DocumentModel`；
- `TabId`；
- dirty、光标、选区和 undo；
- Markdown/Mindmap 插件；
- Viewport、reshape 和渲染状态。

中间文件卡片不是 tab。notora 在产品层维护：

```text
DocumentIdentity → TabId
```

### 5.3 LayoutState

负责：

- 当前 `NavigationScope`；
- 中间列表选择；
- 左栏和中栏宽度；
- 展开的工作区目录和标签组；
- 顶层焦点；
- overlay、菜单和拖动分隔条状态。

不直接修改笔记文件。

## 6. 领域类型

### 6.1 稳定标识

```rust
pub struct WorkspaceId(Uuid);
pub struct NoteId(Uuid);
pub struct TagId(Uuid);
pub struct ExternalFileId(Uuid);
```

- `WorkspaceId` 在工作区首次初始化时生成；
- `NoteId` 存在 catalog 中，应用内重命名或移动不改变；
- `TagId` 与展示名称分离，标签改名不需要重写关联；
- `ExternalFileId` 属于产品会话，不进入笔记 catalog。

所有 ID 均为不透明 newtype，禁止跨领域直接使用裸 `String`。

### 6.2 文件类型

```rust
pub enum DocumentKind {
    Text,
    Markdown,
    Mindmap,
}
```

匹配顺序：

1. 完整后缀 `.mmap.md` → `Mindmap`；
2. 扩展名 `.md` → `Markdown`；
3. 扩展名 `.txt` → `Text`。

工作区扫描忽略其他文件。外部打开首版同样限制为可解码文本文件；不支持二进制
预览。

### 6.3 文档来源

```rust
pub enum DocumentOrigin {
    Note {
        workspace_id: WorkspaceId,
        note_id: NoteId,
        relative_path: PathBuf,
    },
    ExternalFile {
        external_file_id: ExternalFileId,
        canonical_path: PathBuf,
    },
    UntitledExternal {
        external_file_id: ExternalFileId,
        kind: DocumentKind,
    },
}
```

该 enum 决定保存策略，禁止组合 `is_note/is_external/is_untitled/auto_save`
等多个 bool。

### 6.4 导航范围

```rust
pub enum NavigationScope {
    Search { query: String },
    WorkspaceRoot,
    Directory { relative_path: PathBuf },
    Starred,
    Trash,
    Tag { tag_id: TagId },
    ExternalFiles,
}
```

### 6.5 卡片摘要

```rust
pub struct NoteSummary {
    pub note_id: NoteId,
    pub relative_path: PathBuf,
    pub kind: DocumentKind,
    pub title: String,
    pub excerpt: String,
    pub modified_at: SystemTime,
    pub starred: bool,
    pub tags: Vec<TagSummary>,
    pub lifecycle: NoteLifecycle,
}

pub enum NoteLifecycle {
    Active,
    Trashed {
        original_relative_path: PathBuf,
        deleted_at: SystemTime,
    },
}
```

## 7. 工作区与磁盘结构

一个窗口对应一个活动工作区。用户选择的目录保持普通可读：

```text
MyNotes/
├── welcome.md
├── inbox/
│   ├── idea.txt
│   └── architecture.mmap.md
└── .notora/
    ├── workspace.toml
    ├── catalog.sqlite3
    └── trash/
        └── <note-id>/
            └── <original-file-name>
```

### 7.1 workspace.toml

只保存稳定的工作区身份和 schema 版本：

```toml
schema_version = 1
workspace_id = "..."
```

不保存笔记正文。

### 7.2 catalog.sqlite3

保存：

- 相对路径与稳定 `NoteId`；
- 文件类型、大小、mtime、content hash；
- 标题与简介；
- 星标；
- 标签及关联；
- 回收站清单；
- 全文索引；
- schema migration 版本。

正文文件仍是内容源。catalog 的索引部分可以重建；星标、标签和回收站清单是
用户数据，修改必须使用事务并定期创建一致性备份。

SQLite 开启 WAL。watcher 必须完整忽略 `.notora/`，避免数据库 WAL 触发索引
循环。

### 7.3 产品配置目录

notora 产品配置不复用 `~/.edit+`。产品层解析并注入：

```rust
pub struct NotoraPaths {
    pub config_directory: PathBuf,
    pub settings_file: PathBuf,
    pub session_file: PathBuf,
    pub snapshots_directory: PathBuf,
    pub catalog_backups_directory: PathBuf,
}
```

共享层不推导这些路径。

`session.toml` 保存：

- 上次工作区路径和 `WorkspaceId`；
- 外部文件列表；
- 最后选中的导航范围和文档；
- 目录展开状态；
- 左栏和中栏逻辑宽度；
- 窗口位置与大小。

## 8. 左侧导航设计

布局顺序固定：

```text
[ 搜索框                         ]

工作区
  ▾ 子目录
    ▸ 二级目录

星标
回收站
标签
  # 工作
  # 灵感
文件

[ 设置                           ]
```

### 8.1 通用 UI 输入

`ui` 新增通用 `TreeListWidget`，只接收纯展示数据：

```rust
pub struct NavigationRowInput {
    pub key: NavigationRowKey,
    pub label: String,
    pub icon: IconName,
    pub depth: usize,
    pub expansion: RowExpansion,
    pub selection: RowSelection,
    pub badge: Option<u32>,
}
```

`NavigationRowKey` 是 UI 当帧键，不包含 `NoteId` 或 `TagId`。notora-app 维护
当帧 key 到领域 action 的映射。

### 8.2 搜索框

- 输入后 120ms debounce；
- 非空时自动切换到 `NavigationScope::Search`；
- 清空后恢复输入搜索前的导航范围；
- `Esc` 先清空搜索，再把焦点交回导航；
- 中文输入过程中 IME preedit 不触发搜索，commit 后才发起查询；
- 每次查询带 generation，过期后台结果必须丢弃。

### 8.3 工作区目录

- 根节点下只展示包含受支持笔记或子目录的目录；
- `.notora` 和隐藏系统文件默认不展示；
- 目录按名称稳定排序；
- 展开状态按相对路径持久化；
- 单击目录在中栏展示该目录的直接笔记；
- “包含所有后代笔记”可作为后续筛选设置，首版不默认递归混排。

### 8.4 标签

- 标签入口可展开；
- 标签名称唯一性按 Unicode 规范化后的值比较；
- 标签重命名不改变 `TagId`；
- 删除标签只删除关联，不删除笔记；
- Trash 中笔记的标签不计入普通 badge。

### 8.5 设置按钮

设置固定在左栏底部，不随导航列表滚动。打开产品设置 overlay：

- Appearance；
- Editor；
- Interface；
- Workspace。

复用 `ui::widgets::form` 等通用控件，不把 notora 设置 DTO 放入 `ui`。

## 9. 中间文件卡片区

### 9.1 标题栏

不同入口的标题和操作：

| 当前入口 | 标题 | 主操作 | 次操作 |
|---|---|---|---|
| 工作区根 | 工作区名称 | 新建下拉 | 无 |
| 子目录 | 目录名 | 新建下拉 | 无 |
| 星标 | 星标 | 新建下拉 | 无 |
| 标签 | 标签名 | 新建下拉 | 无 |
| 搜索 | 搜索结果 | 无 | 清空搜索 |
| 回收站 | 回收站 | 恢复所选 | 清空回收站 |
| 文件 | 文件 | 打开 | 新建下拉 |

新建下拉顺序：

1. 新建 TXT；
2. 新建 MD；
3. 新建 MMAP.MD。

### 9.2 新建位置

- 当前范围是目录：创建到该目录；
- 当前范围是工作区根、星标、标签或搜索：创建到工作区根；
- 当前范围是回收站：不显示新建；
- 当前范围是文件：创建 `UntitledExternal`，首次保存时弹出 Save As；
- 从标签范围新建成功后，自动分配当前标签；
- 从星标范围新建成功后不自动星标，避免入口语义隐式修改数据。

工作区笔记创建后立即落盘，以便获得稳定路径并进入 watcher/catalog：

- TXT 初始内容为空；
- MD 初始内容为空；
- MMAP.MD 初始内容为 `#`，光标位于其后；
- 使用语义化的唯一名称，如 `未命名 1.md`；
- 不覆盖已有同名文件。

### 9.3 卡片内容

每张卡片展示：

- 标题；
- 简介；
- 最后修改时间；
- 文件类型图标；
- 星标状态；
- 可选标签摘要。

标题解析：

- Markdown/Mindmap：首个一级标题，否则文件 stem；
- TXT：首个非空行，否则文件 stem；
- 空文档：文件 stem。

简介解析：

- 取首个非标题、非空有效段落；
- 去除常见 Markdown 展示标记；
- 以 grapheme 为单位截断，禁止切断 UTF-8 或组合字符；
- 首版上限由语义常量定义，不在绘制函数中硬编码。

### 9.4 虚拟化

中栏使用 `VirtualCardListWidget`：

- 只布局可见卡片和少量 overscan；
- scroll offset 与选择状态分离；
- 查询分页或分段获取；
- 列表更新通过稳定 `DocumentCardKey` 保持选择；
- 不在 paint 阶段读取文件、执行 SQL 或解析 Markdown。

### 9.5 选择行为

- 单击卡片：用 `OpenDisposition::Preview` 打开或激活；
- 同一笔记已在 persistent runtime 中时直接激活，不降级为 preview；
- 键盘上下移动选择并更新右侧 preview；
- `Enter` 将当前 preview 转为 persistent；
- 编辑一旦发生，preview 自动转为 persistent，避免切换卡片丢失 dirty 文档；
- 删除、移动或回收当前笔记时，按 typed effect 更新 runtime 和选择。

## 10. 右侧编辑区

路由：

```text
*.mmap.md → Mindmap plugin
*.md      → Markdown WYSIWYG plugin
*.txt     → Editor plugin
```

右侧不理解导航范围和卡片查询，只接收：

- 活动 `TabId`；
- `editor_rect`；
- 编辑设置和主题；
- 已经过产品焦点过滤的输入事件。

空状态：

- 没有工作区：显示“选择或创建工作区”；
- 当前范围没有选择：显示“选择一篇笔记”；
- 外部文件失效：显示路径和重新定位/移除操作；
- 加载或解析失败：显示可恢复错误，不创建半初始化 runtime。

## 11. 三栏布局

默认逻辑宽度：

```text
左栏 220px | 中栏 340px | 右栏填充
```

约束：

- 左栏范围 180–320px；
- 中栏范围 260–520px；
- 右栏保留编辑器最小可用宽度；
- 两个 splitter 独立拖动并按 DPI 使用逻辑尺寸持久化；
- 窗口缩小时先压缩中栏，再进入折叠模式，不能产生负 Rect；
- modal、菜单和 tooltip 最后绘制；
- 左栏和中栏都不侵入 editor rect；
- 编辑器的 gutter、光标、IME 和点击坐标统一基于最终 editor rect。

首版桌面最小窗口尺寸建议为 880×600 逻辑像素。低于可用宽度时：

1. 左栏切换为 overlay；
2. 中栏保留；
3. 选择卡片后可临时显示编辑器并提供返回按钮。

响应式折叠可以在桌面主流程完成后作为独立阶段实现，但布局类型应预留互斥 enum，
禁止用多个 `collapsed` bool 组合。

## 12. UI 组件边界

建议在 `ui` 中新增以下业务无关组件：

- `TreeListWidget`：分层列表、展开、选中、badge；
- `VirtualCardListWidget`：虚拟化卡片列表；
- `SplitButtonWidget`：主按钮与下拉按钮；
- `SplitterWidget`：拖动调整相邻区域尺寸；
- 通用空状态和错误状态组件。

notora-app 定义：

- `NotoraShell`；
- `NavigationPanelInput` 到通用 tree rows 的映射；
- `DocumentCardInput`；
- 产品标题栏；
- `NotoraAction`；
- 产品 overlay 和菜单。

绝对禁止：

- `ui` 依赖 `LibraryState`、`NoteId`、`NavigationScope`；
- widget 直接执行 SQL、文件 I/O 或访问 `EditorRuntime`；
- notora-app 把整个 `LibraryState` 传给 widget；
- 使用字符串动作名或 `Any` 向上冒泡产品动作。

## 13. 数据库设计

首版 schema 概念模型：

```text
notes
  note_id PK
  relative_path UNIQUE
  kind
  title
  excerpt
  modified_ns
  file_size
  content_hash
  starred
  lifecycle

tags
  tag_id PK
  normalized_name UNIQUE
  display_name

note_tags
  note_id FK
  tag_id FK
  PK(note_id, tag_id)

trash_entries
  note_id PK/FK
  original_relative_path
  trash_relative_path
  deleted_at

notes_fts
  note_id
  title
  relative_path
  body
  tags
```

所有 schema 变更使用单调递增 migration，不允许运行时临时探测列然后拼补。

### 13.1 中文搜索

SQLite FTS5 默认分词不能满足所有中文子串查询。首版采用：

- 标题和路径：规范化后的模糊匹配；
- 正文：FTS5 trigram tokenizer；
- 1–2 字符查询：受限候选集上的 fallback 扫描；
- 标签：精确/前缀匹配；
- 最终按标题命中、路径命中、标签命中、正文命中和修改时间混合排序。

查询字符串必须通过参数绑定，不能拼接 SQL。

### 13.2 一致性

- catalog 操作使用事务；
- 内容写盘成功而 catalog 更新失败时，启动 reconciliation 能重新索引；
- catalog 行存在但文件消失时，先标记 missing，等待 watcher 合并窗口后再清理；
- 星标/标签变更后创建 catalog 一致性备份；
- 数据库损坏恢复时优先保留用户 metadata，再重建派生索引；
- 禁止在 UI 主线程执行全量扫描、FTS rebuild 或大文件解析。

## 14. 后台服务

`NotoraProduct` 实现 `ProductHost`，持有：

```text
NotoraProduct
├── CatalogService
├── WorkspaceFileMonitor
├── IndexWorker
├── SaveWorker
├── product event sender/receiver
└── shutdown handles
```

后台 payload 只进入 notora 自有 channel：

```rust
enum NotoraProductEvent {
    ScanCompleted(ScanCompletion),
    FileChangesIndexed(IndexBatchCompletion),
    SearchCompleted(SearchCompletion),
    SaveCompleted(SaveCompletion),
    CatalogFailed(CatalogFailure),
}
```

后台线程随后发送无 payload 的 `ShellEvent::ProductWake`。主线程在
`drain_product_events` 中排空产品 channel，并返回通用 `ShellEffect`。

## 15. 扫描与文件监控

### 15.1 初次扫描

1. 验证工作区存在且为目录；
2. 创建或读取 `.notora/workspace.toml`；
3. 打开并迁移 catalog；
4. 后台递归扫描受支持文件；
5. 比较相对路径、mtime、size 和 content hash；
6. 只解析新增或变化文件；
7. 分批提交 catalog；
8. 每批唤醒 UI，卡片可以渐进出现。

### 15.2 watcher

- 使用平台 `RecommendedWatcher`；
- 递归监听工作区；
- 忽略 `.notora/`、notora 原子保存临时文件和系统垃圾文件；
- 200ms debounce 合并路径；
- rename 尽量使用平台事件配对；
- 无法配对时通过 file identity/content hash 识别移动；
- 找不到稳定匹配时按删除 + 新增处理，并保留可诊断日志；
- notora 自己保存产生的事件仍可进入一致性检查，但不能重复建立编辑冲突。

## 16. 搜索流程

```text
SearchBox commit
  -> SearchRequested { generation, query }
  -> IndexWorker / CatalogService
  -> SearchCompleted { generation, summaries }
  -> 丢弃 generation 过期结果
  -> 更新中栏 VirtualCardList
```

搜索默认排除 Trash。Trash 入口使用独立查询，不与普通全文搜索混合。

空查询不执行 FTS；恢复搜索前的导航范围。

## 17. 笔记自动保存

### 17.1 调度

notora 收到：

```rust
EditorNotification::ContentChanged {
    tab_id,
    content_revision,
}
```

根据 `DocumentOrigin` 处理：

- `Note`：设置或刷新该 `TabId` 的 800ms idle deadline；
- `ExternalFile`：只标记 dirty，不安排保存；
- `UntitledExternal`：只标记 dirty，首次显式保存走 Save As。

autosave 状态使用 enum：

```rust
pub enum AutoSaveState {
    Idle,
    Scheduled { deadline: Instant, content_revision: u64 },
    Saving { content_revision: u64 },
    Failed { content_revision: u64, message: String },
}
```

不使用 `scheduled/saving/failed` 多 bool。

### 17.2 保存执行

1. deadline 到期；
2. 调用 `EditorRuntime::prepare_save(tab_id)`；
3. 将不可变 `PreparedDocumentSave` 发送给 SaveWorker；
4. worker 使用 expected `DiskRevision` 原子写盘；
5. 返回 `SaveCompletion`；
6. runtime 应用 completion；
7. 若当前 revision 已更新，保持 dirty 并重新调度；
8. catalog 异步更新标题、简介、mtime 和全文索引。

IME preedit 不触发保存；IME commit 产生正常内容变更后才调度。

### 17.3 错误

- `ConcurrentModification`：暂停自动保存并显示冲突提示；
- 权限/磁盘错误：保持 dirty，显示失败状态，允许重试或另存；
- 工作区被移除：禁止继续自动写入，切换到恢复流程；
- 退出时仍有 dirty 笔记：等待有界时间完成已提交保存，未完成内容保留 dirty snapshot。

## 18. 外部文件

“文件”入口是临时文件工作区，与笔记完全分离。

进入方式：

- 中栏“打开”按钮；
- 系统双击/打开方式；
- Finder/Explorer 拖入；
- “文件”入口的新建下拉。

行为：

- 打开后加入 `ExternalFileSession` 并在右侧激活；
- 不进入工作区目录、搜索、星标、标签和回收站；
- 手动 `Cmd/Ctrl+S` 保存；
- untitled 外部文件首次保存弹出 Save As；
- 从列表移除只关闭 session，不删除磁盘文件；
- 外部文件路径在 `session.toml` 中恢复；
- 恢复时路径不存在则显示 missing，提供重新定位或移除；
- 同一路径只建立一个 external entry 和一个 `TabId`。

系统双击外部文件时，即使当前导航不在“文件”，也要加入该入口并激活；是否自动
切换左侧选择到“文件”由产品 action 明确执行，首版采用自动切换。

## 19. 星标与标签

### 19.1 星标

- 星标只适用于 Active 笔记；
- 切换星标使用单条 catalog transaction；
- 星标入口按最近修改时间排序；
- 移入 Trash 后不出现在星标列表，但保留 starred metadata；
- 恢复后恢复原星标状态。

### 19.2 标签

- 标签存在 catalog，不写入正文；
- 支持从卡片 context menu 和编辑器顶部 metadata 区分配；
- 同一笔记和标签关联幂等；
- 标签重命名不修改正文；
- 标签删除需确认，只删除关联；
- 从标签入口新建自动附加当前标签；
- Trash 笔记保留标签但不参与普通标签计数。

## 20. 回收站

移入回收站：

1. 如果当前笔记 dirty，先完成自动保存；保存失败则取消回收；
2. 将文件原子移动到 `.notora/trash/<note-id>/<file-name>`；
3. catalog 记录原相对路径和删除时间；
4. 从普通目录、搜索、星标和标签查询中移除；
5. 关闭或切换对应 editor runtime；
6. 更新中栏选择。

恢复：

- 原目录存在且原路径空闲：恢复原路径；
- 原路径被占用：弹出选择，支持重命名恢复或取消；
- 不静默覆盖现有文件；
- 恢复后保留星标和标签。

永久删除：

- 只允许在 Trash 入口；
- 必须显式确认；
- 删除文件与 metadata 后不可由 notora 恢复；
- 批量清空回收站要先解析精确目标列表，禁止对工作区根执行宽泛递归删除。

## 21. 文件操作一致性

创建、重命名、移动和回收采用领域命令：

```rust
pub enum NoteCommand {
    Create(CreateNoteRequest),
    Rename(RenameNoteRequest),
    Move(MoveNoteRequest),
    MoveToTrash(NoteId),
    Restore(RestoreNoteRequest),
    DeletePermanently(NoteId),
    SetStarred { note_id: NoteId, starred: bool },
    AttachTag { note_id: NoteId, tag_id: TagId },
    DetachTag { note_id: NoteId, tag_id: TagId },
}
```

UI 不直接调用 `std::fs`。

文件操作和 catalog 无法形成真正的跨系统原子事务，因此采用：

1. 预检查精确目标；
2. 同文件系统原子 rename/atomic write；
3. catalog transaction；
4. 失败补偿；
5. 启动 reconciliation 修复中断状态。

## 22. 产品动作与 effect

notora-app 定义类型化动作：

```rust
pub enum NotoraAction {
    NavigationSelected(NavigationScope),
    SearchCommitted(String),
    CardSelected(DocumentIdentity),
    CreateRequested(DocumentKind),
    OpenExternalRequested,
    ToggleStar(NoteId),
    MoveToTrash(NoteId),
    Restore(NoteId),
    AttachTag { note_id: NoteId, tag_id: TagId },
    SplitterDragged { pane: Pane, logical_width: f32 },
    OpenSettings,
}
```

Reducer 只决定状态与 effect：

```rust
pub enum NotoraEffect {
    QueryCards(CardQuery),
    ExecuteNoteCommand(NoteCommand),
    PrepareDocument(DocumentIdentity),
    ActivateEditor(TabId),
    ScheduleAutoSave { tab_id: TabId, revision: u64 },
    PersistSession,
    Redraw,
}
```

文件 I/O、SQL、对话框和 runtime 调用在 effect executor 执行，不散落在 widget。

## 23. 焦点和快捷键

```rust
pub enum FocusTarget {
    NavigationSearch,
    NavigationTree,
    CardList,
    Editor,
    Overlay,
}
```

建议快捷键：

- `Cmd/Ctrl+N`：按当前入口创建默认 MD；
- `Cmd/Ctrl+Shift+N`：打开新建类型菜单；
- `Cmd/Ctrl+O`：打开外部文件；
- `Cmd/Ctrl+F`：焦点进入全局笔记搜索；
- `Cmd/Ctrl+S`：外部文件保存；笔记则立即执行一次 autosave；
- `Cmd/Ctrl+,`：设置；
- `Up/Down`：卡片列表移动；
- `Enter`：固定当前 preview 并进入编辑器；
- `Esc`：按 overlay → 搜索 → 编辑器焦点的层级退出。

产品焦点不属于 Editor 时，不能把字符输入或 IME 交给 `EditorRuntime`。

## 24. 会话恢复

启动顺序：

1. 读取 notora settings；
2. 创建窗口和 `EditorRuntime`；
3. 读取 `session.toml`；
4. 验证上次工作区；
5. 打开 catalog，启动增量扫描；
6. 恢复左栏/中栏宽度和目录展开状态；
7. 恢复外部文件路径列表；
8. 恢复最后选中的导航范围；
9. 如果最后文档仍存在，按需打开，不一次恢复全部笔记 runtime；
10. 首帧可用后渐进更新卡片和索引状态。

notora 不应为整个工作区每篇笔记创建 `DocumentModel`。只有右侧实际打开的文档
进入 `EditorRuntime`。

## 25. 大文件与性能

- 扫描只读取元数据和需要变化校验的内容；
- 摘要和全文索引在后台生成；
- 中栏虚拟化；
- SQL 查询分页；
- 搜索 generation 丢弃过期结果；
- 活跃 editor runtime 数量采用 LRU 上限，关闭干净且非活动的 runtime；
- dirty、正在保存或 pinned 的 runtime 不参与自动回收；
- 大文件继续复用 textora 的 viewport/reshape 优化；
- UI 主线程不执行 catalog vacuum、rebuild 或目录全量遍历。

首版性能目标：

- 10,000 篇普通笔记下，已建索引搜索结果首批在 100ms 级返回；
- 中栏滚动不随总笔记数线性退化；
- 切换已打开笔记不重新读取磁盘；
- 后台扫描不阻塞输入和光标动画。

性能目标是验收方向，不作为无测量依据的承诺；实施时必须建立基准数据集。

## 26. 安全与恢复

- 所有工作区路径在执行前规范化并验证仍位于工作区根；
- 禁止 `..` 逃逸；
- `.notora` 是保留目录，用户操作不能把笔记创建或移动到其中；
- 删除操作只接受解析后的 `NoteId` 和 catalog 精确路径；
- 外部文件绝不进入 notora 内部回收站；
- 保存使用 expected disk revision，禁止自动覆盖外部修改；
- dirty snapshot 使用 notora 独立路径；
- catalog migration 前备份；
- catalog 损坏不影响正文文件可直接访问。

## 27. 实施阶段

### N0：EditorRuntime 前置

- 完成依赖设计中的最小 runtime；
- textora 全量回归通过；
- fake product 能在自定义矩形嵌入编辑器。

### N1：notora-core 基础

- 建立 workspace identity、领域 ID 和 catalog migration；
- 完成扫描、摘要解析和基本查询；
- 完成 watcher 与 reconciliation。

### N2：notora App 骨架

- 创建 `notora` binary；
- 创建 NotoraProduct、NotoraShell 和三栏静态布局；
- 接入 EditorRuntime；
- 用内存卡片数据验证布局、焦点和事件边界。

### N3：打开与新建

- 工作区选择和恢复；
- 新建 TXT/MD/MMAP.MD；
- 卡片 preview/open；
- 外部文件打开与 session；
- 插件路由和 WYSIWYG/Mindmap 验证。

### N4：保存与文件安全

- 笔记 800ms 自动保存；
- 外部文件手动保存；
- Save As；
- 外部修改冲突；
- dirty snapshot 和退出恢复。

### N5：搜索与卡片

- 后台全文索引；
- 中文搜索；
- 虚拟化卡片列表；
- 标题、简介、mtime 增量更新。

### N6：星标、标签和回收站

- metadata 操作；
- 标签导航；
- move/restore/permanent delete；
- 路径冲突和恢复测试。

### N7：设置、恢复和性能

- 产品设置；
- session restore；
- runtime LRU；
- 大工作区性能与异常恢复；
- 完整手工验收。

每个阶段都必须继续拆成最多修改 3 个逻辑文件的实施任务。重大阶段完成后运行
`./scripts/verify.sh`。

## 28. 测试策略

### 28.1 notora-core

- 支持后缀分类，特别是 `.mmap.md` specificity；
- 路径规范化和根目录逃逸；
- scan/reconcile；
- 标题和 excerpt 的 Unicode/grapheme 行为；
- catalog migration；
- 搜索 ranking 和中文短查询；
- 星标/标签事务；
- trash/restore/path conflict；
- watcher debounce 与自写事件过滤；
- 数据库损坏和备份恢复。

### 28.2 notora-app

- 导航范围到查询映射；
- UI DTO 不泄漏领域状态；
- 当前目录新建位置；
- 标签入口新建自动附加标签；
- 卡片 preview 转 persistent；
- `DocumentIdentity ↔ TabId` 稳定映射；
- 焦点不在 Editor 时输入不修改文档；
- modal 阻断编辑器事件；
- 笔记和外部文件保存策略不同；
- external missing 恢复；
- 三栏 splitter 和 DPI 持久化。

### 28.3 集成与 smoke

- TXT 打开、编辑、自动保存；
- MD WYSIWYG 打开、编辑、自动保存；
- MMAP.MD 路由、编辑、自动保存；
- 外部 TXT/MD 手动保存；
- 系统双击加入“文件”入口；
- 外部修改冲突；
- move to trash/restore；
- 搜索后打开结果；
- 10,000 卡片虚拟化；
- headless/render smoke。

## 29. 架构与质量门槛

新增检查：

```bash
# notora-core 必须保持 headless
cargo tree -p notora-core

# shared crate 不能出现 notora 产品语义
rg -n 'Notora|NoteId|NavigationScope|notora' \
  crates/appkit-core crates/appkit-shell crates/ui

# notora 不复用 textora 产品路径
rg -n '\\.edit\\+' crates/notora-core crates/notora-app

# ui 不依赖 notora
cargo tree -p textora-ui
```

阶段验证：

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test -p notora-core
cargo test -p notora-app
bash scripts/check_architecture.sh
./scripts/verify.sh
```

## 30. 首版验收标准

- 启动显示 notora 产品身份和三栏界面；
- 可创建/选择一个工作区并浏览子目录；
- 可新建 TXT、MD、MMAP.MD；
- MD 使用现有 WYSIWYG，MMAP 使用现有 Mindmap；
- 中栏显示标题、简介和最后修改时间；
- 点击卡片在右侧打开，切换不丢失编辑状态；
- 笔记在 800ms idle 后自动保存；
- 外部文件只在显式保存时写盘；
- 系统双击打开的文件进入“文件”入口；
- 搜索、星标、标签和回收站完整可用；
- 外部修改不会被自动覆盖；
- 重启能恢复工作区、外部文件列表、栏宽和最后选择；
- 10,000 笔记规模下卡片列表保持虚拟化；
- textora 原产品行为不回归；
- shared crates 不依赖 notora；
- `./scripts/verify.sh` 通过。
