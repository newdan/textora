# 文件打开历史 (File Open History) 实施计划

## 1. 目标与背景

在现有 Hot Exit（退出恢复）的基础上，增加**跨会话的文件打开历史**。类似 VS Code 的 `File > Open Recent`，让用户可以快速回顾和打开之前操作过的文件。

**核心需求**（来自讨论确认）：

| # | 需求 | 说明 |
|---|------|------|
| 1 | 关闭时记录 | 关闭单个 Tab 时记录到 history（不记入 workspace）。退出程序时所有打开的 Tab 同时记入 workspace 和 history |
| 2 | 按 workspace 恢复 | history 条目关联 workspace，按工作上下文分组查询 |
| 3 | 上限 100 条 | 总容量 100，菜单展示最近 20 条 |
| 4 | 文件不存在则不显示 | 路径失效的条目跳过显示，但不立即删除（可定时清理） |
| 5 | 可删除单条 | 右键/菜单中移除某条历史记录 |
| 6 | 可排除目录 | 指定某些目录下的文件不进入历史 |
| 7 | 不区分打开来源 | CLI / 拖拽 / 文件对话框统一处理 |
| 8 | 退出时记 workspace | 退出程序时所有打开的 Tab 同时写入 workspace.yaml 和 history.yaml |

---

## 2. 数据模型

### 2.1 FileHistoryEntry（单条历史）

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FileHistoryEntry {
    /// 文件绝对路径
    pub(crate) file_path: PathBuf,
    /// 所属 workspace 根目录（用于按 workspace 分组恢复/展示）
    pub(crate) workspace_root: Option<PathBuf>,
    /// 最后一次关闭的时间戳（epoch millis）
    pub(crate) last_closed_at: u64,
    /// 最后一次关闭时的光标位置
    pub(crate) last_cursor_line: usize,
    pub(crate) last_cursor_col: usize,
}
```

### 2.2 FileHistory（全局单例）

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FileHistory {
    pub(crate) version: u32,
    /// 按 last_closed_at 降序排列的条目列表
    pub(crate) entries: Vec<FileHistoryEntry>,
    /// 排除目录列表（文件所属目录匹配任一即不记录）
    pub(crate) excluded_dirs: Vec<PathBuf>,
}
```

### 2.3 存储位置

```
~/.config/edit+/history.yaml
```

与 workspace 缓存（`workspace.yaml`）**独立存储**，原因：
- workspace 是瞬态快照，history 是长期累积
- 数据结构和生命周期不同
- 独立文件更易于调试和手动编辑

### 2.3.1 Workspace vs History 职责边界（重要）

| 事件 | workspace.yaml | history.yaml |
|------|:---:|:---:|
| 关闭单个 Tab | ❌ 不写 | ✅ 记录 |
| 退出程序 (Cmd+Q) | ✅ 全量写入所有 Tab | ✅ 为每个 Tab 记录一条 |
| 启动恢复 | ✅ 恢复所有 Tab | ✅ 加载用于菜单展示 |

**设计意图**：
- workspace = 「当前正在编辑什么」→ 退出时保存 → 启动时恢复
- history = 「用过哪些文件」→ 关闭 Tab 时记录 + 退出时记录 → 菜单中展示，方便重新打开
- 两者职责不同，不互相污染

### 2.4 YAML 格式示例

```yaml
version: 1
entries:
  - file_path: "/Users/dan/proj/edit+/crates/app/src/workspace.rs"
    workspace_root: "/Users/dan/proj/edit+"
    last_closed_at: 1680000000000
    last_cursor_line: 170
    last_cursor_col: 28
  - file_path: "/Users/dan/proj/edit+/crates/app/src/app.rs"
    workspace_root: "/Users/dan/proj/edit+"
    last_closed_at: 1680000001000
    last_cursor_line: 45
    last_cursor_col: 12
excluded_dirs:
  - "/Users/dan/proj/vendor"
  - "/Users/dan/.cargo"
```

---

## 3. 详细开发阶段 (Phases)

### Phase 1: FileHistory 模型 + 磁盘读写（底层基建）

**目的**：建立 history 的数据结构与持久化能力。与 Hot Exit Phase 1 同级，作为独立底层模块。

**具体改动**：

| 文件 | 改动 |
|------|------|
| `[NEW] crates/app/src/file_history.rs` | 定义 `FileHistory`, `FileHistoryEntry`，实现序列化 |
| `[MODIFY] crates/app/src/lib.rs` | 注册 `mod file_history` |

**核心接口**：

```rust
impl FileHistory {
    /// 从磁盘加载，不存在或损坏则返回空
    pub(crate) fn load(config_dir: &Path) -> Self;
    /// 写回磁盘
    pub(crate) fn save(&self, config_dir: &Path) -> io::Result<()>;
    /// 记录一条（关闭 Tab 时调用）
    pub(crate) fn record(&mut self, entry: FileHistoryEntry);
    /// 获取有效条目（排除文件不存在的 + 排除目录内的），最多返回 N 条
    pub(crate) fn get_valid_entries(&self, n: usize) -> Vec<&FileHistoryEntry>;
    /// 按 workspace 过滤
    pub(crate) fn get_by_workspace(&self, workspace_root: &Path, n: usize) -> Vec<&FileHistoryEntry>;
    /// 删除单条（按文件路径）
    pub(crate) fn remove_entry(&mut self, file_path: &Path);
}
```

**边界**：
- 磁盘写入出错（磁盘满 / 权限不足）→ 打印警告，不阻塞主流程
- YAML 被外部损坏 → 静默回退到空 history，覆盖写入
- entries 列表始终按 `last_closed_at` 降序维护

**验证**：
- 构造 150 条模拟数据，验证 `record()` 后自动截断为 100 条
- 在 excluded_dirs 中创建文件路径，验证 `get_valid_entries()` 正确过滤
- 写入 YAML 后手动损坏，重新加载验证不会 panic

---

### Phase 2: 关闭 Tab → 记录 History（记录点埋入）

**目的**：在 Tab 关闭时自动记录到 history。

**具体改动**：

| 文件 | 改动 |
|------|------|
| `[MODIFY] crates/app/src/workspace.rs` | `close_tab_inner()` 中，关闭前调用 `history.record()` |
| `[MODIFY] crates/app/src/app.rs` | App 持有 `FileHistory` 实例，传递到 Workspace 或通过回调 |

**架构选择**：FileHistory 的归属

```
选项 A：App 持有 FileHistory，Workspace 通过回调记录
  Workspace.close_tab() → 返回被关闭文件信息 → App 调 history.record()

选项 B：Workspace 直接持有 FileHistory
  workspace.history.record(entry)
```

推荐 **选项 B**：Workspace 已经管理所有 Tab 生命周期，由它直接持有 history 最自然。但需注意 save 操作通过 App 的退出流程统一触发。

**边界**：
- 临时文件（file_path == None）不记录
- 重复打开 → 关闭同一文件：更新 timestamp 和 cursor，移到最前，不重复
- App 退出/崩溃时 history 未保存 → 需在 App 退出流程中调用 `history.save()`

**验证**：
- 打开 A.rs → 编辑 → 关闭 → 检查 history 文件中是否有 A.rs 及光标位置
- 再次打开 A.rs → 编辑新位置 → 关闭 → 验证 timestamp 更新，条目数不增加
- 打开临时文件（无 file_path）→ 编辑 → 关闭 → 验证不记录

---

### Phase 3: File > Open Recent 菜单（UI 层）

**目的**：通过系统菜单呈现最近文件。

**具体改动**：

| 文件 | 改动 |
|------|------|
| `[MODIFY] crates/app/src/native_menu.rs` | 增加 `OpenRecent` 子菜单项 |
| `[MODIFY] crates/app/src/menu_handler.rs` | 处理 Open Recent 点击事件 |
| `[MODIFY] crates/app/src/app.rs` | 菜单构建时注入 history 数据 |

**菜单结构**：

```
File
├── New
├── Open...
├── Open Recent              ← 新增子菜单
│   ├── /path/to/file1.rs
│   ├── /path/to/file2.rs
│   ├── ...
│   ├── ─────────────
│   └── Clear Recently Opened
├── Save
├── Save As...
└── Quit
```

**菜单条目设计**：
- 每个文件显示文件名（不含路径），hover/tooltip 显示完整路径
- 最多 20 条（`get_valid_entries(20)`）
- "Clear Recently Opened" 清空全部历史（需二次确认）
- 历史为空时不显示子菜单

**边界**：
- 上限处理：history 存 100，菜单只展示 20
- 去重：如果文件已在 workspace 中打开，菜单中标记（✓ 或灰显）
- 性能：菜单构建在 UI 线程，`get_valid_entries()` 需要检查文件存在性 → 可能需要缓存

**验证**：
- 关闭几个文件后，检查菜单是否正确显示最近文件
- 点击菜单中的文件 → 验证文件在编辑器中打开
- "Clear Recently Opened" → 验证 history 清空

---

### Phase 4: Workspace 恢复 + History 联动

**目的**：启动恢复 workspace 时，同时加载对应的 history，实现按 workspace 的上下文感知。

**具体改动**：

| 文件 | 改动 |
|------|------|
| `[MODIFY] crates/app/src/app.rs` | 启动流程：加载 history → 恢复 workspace → 展示归属该 workspace 的历史 |
| `[MODIFY] crates/app/src/workspace.rs` | 恢复 workspace 时注入 history 信息 |
| `[MODIFY] crates/app/src/file_history.rs` | 增加按 workspace_root 过滤 + 路径存在性检查 |

**交互流程**：

```
App 启动
  ├── 1. 加载 history.yaml
  ├── 2. 加载 workspace.yaml（Hot Exit 恢复）
  ├── 3. 恢复所有 Tab（懒加载）
  │      ├── 仅 active tab 立即加载文件内容到内存
  │      ├── 其他 tab 只创建 stub（file_path + cursor_position + dirty flag）
  │      ├── 脏文件（无磁盘文件）的未保存内容从 workspace.yaml 恢复
  │      └── 用户切换到某 tab 时才真正读磁盘打开文件
  ├── 4. 确定当前 workspace_root
  │      ├── 如果有多个已打开文件，workspace_root = 公共祖先目录
  │      └── 如果仅有一个空文件，workspace_root = None
  └── 5. 菜单：按 workspace_root 过滤 history，展示最近文件
```

**Workspace 切换**（后续扩展）：
> 当未来支持"打开文件夹"功能时，workspace_root 自动切换，菜单中的 history 随之更换。

**边界**：
- 文件在 workspace 已打开 → 菜单中标记（不重复打开）
- 文件存在性在构建菜单时检查 → 不存在的静默跳过
- **懒加载恢复**：启动时仅 active tab 加载内容，其余 tab 为 stub。切换时才读磁盘，避免启动阻塞。
- workspace_root 为 None 时 → 展示全局 history（不按目录过滤）

**验证**：
- 在一个项目中退出 → 重启 → 验证 workspace 恢复 + 菜单显示该项目历史
- 切换到另一个目录启动 → 验证菜单切换为对应 workspace 的历史
- 外部删除 history 中的文件 → 验证菜单不再显示

---

### Phase 5: 排除目录 + 单条删除（用户控制）

**目的**：给用户细粒度控制 history 的能力。

**具体改动**：

| 文件 | 改动 |
|------|------|
| `[MODIFY] crates/app/src/file_history.rs` | 增加 `add_excluded_dir()`, `remove_excluded_dir()`, `is_excluded()` |
| `[MODIFY] crates/app/src/menu_handler.rs` | 右键历史条目 → "Remove from Recent"，增加 "Manage Excluded Directories..." |
| `[MODIFY] crates/app/src/native_menu.rs` | Open Recent 子菜单中嵌入右键/删除逻辑 |
| `[NEW] crates/app/src/history_settings.rs` | 可选：排除目录的管理界面 |

**功能实现**：

```rust
impl FileHistory {
    /// 添加排除目录
    pub(crate) fn add_excluded_dir(&mut self, dir: PathBuf);
    /// 移除排除目录
    pub(crate) fn remove_excluded_dir(&mut self, dir: &Path);
    /// 检查文件是否在排除目录中
    pub(crate) fn is_excluded(&self, file_path: &Path) -> bool;
}
```

**UI 交互**：
- 菜单中右键某条历史 → 弹出 "Remove from Recent" → 从 history 中移除该条
- 菜单底部 "Manage Excluded Directories..." → 打开设置面板（或弹窗），展示排除目录列表，支持添加/删除
- 排除目录规则：匹配前缀。`/Users/dan/.cargo` 会排除所有 `/Users/dan/.cargo/**` 下的文件

**边界**：
- 删除操作需即时生效并持久化
- 排除目录添加后，之前已记录的文件**不会被回溯删除**（只影响新记录），或者提供选项
- 排除目录路径需规范化（resolve symlinks, ~, ../）

**验证**：
- 添加排除目录后打开目录内文件 → 关闭 → 验证不出现
- 删除排除目录后 → 再次打开该目录文件 → 验证正常记录
- 右键删除某条 history → 验证从菜单消失

---

## 4. 开发依赖关系

```
Phase 1 (数据结构 + IO)
  └── Phase 2 (记录点) ── 依赖 Phase 1
        └── Phase 3 (菜单) ── 依赖 Phase 2
              └── Phase 4 (Workspace联动) ── 依赖 Phase 3 + Hot Exit Phase 3
                    └── Phase 5 (用户控制) ── 依赖 Phase 3
```

**与 Hot Exit 的阶段对应**：

| Hot Exit Phase | File History Phase | 关系 |
|----------------|-------------------|------|
| Phase 1 (Workspace 串行化) | Phase 1 (History 串行化) | 同级独立，可并行 |
| Phase 2 (Quit 拦截) | Phase 2 (关闭记录) | 同级独立 |
| Phase 3 (启动恢复) | Phase 4 (Workspace 联动) | **此处耦合**：恢复时加载 history |
| Phase 4 (Tab Close) | Phase 2 (关闭记录) | 同一个 close_tab 入口 |
| Phase 5 (命名) | - | 无直接关系 |

---

## 6. 确认结论

| # | 议题 | 结论 |
|---|------|------|
| 1 | workspace_root 确定方式 | **公共祖先目录** |
| 2 | 排除目录是否回溯删除 | **不回溯**，只影响新记录 |
| 3 | history 为空时菜单行为 | **灰显 "No Recent Files"** |
| 4 | 搜索/过滤功能 | **不需要** |
| 5 | 关闭 Tab vs 退出程序的记录策略 | **关闭 Tab → history；退出 → workspace**，职责分离 |
| 6 | 菜单中已打开文件 | 标记但不重复打开 |
