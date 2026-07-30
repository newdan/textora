# mmap 所见即所得编辑设计

日期：2026-07-12
状态：已确认

## 1. 背景与根因

当前 `MindmapView` 已能解析、布局和渲染 `.mmap.md`，但仍接入旧画布协议：

- `allows_editing()` 返回 `false`，app 将 mmap 归类为阅读/预览视图，编辑命令会在修改 `DocumentView` 前被拦截。
- mmap 使用 `HitTestCanvas`、`CursorRect`、`CanvasMove`，而 app 的可编辑自绘视图已经迁移到 `HitTestByte`、`CursorScreenPos`、`VisualMove`、`EditPolicy` 和统一编辑事务。
- 旧画布查询在 app 层没有完整消费者，导致点击、光标、输入、IME 和导航链路断裂。
- 继续为 mmap 恢复独立事件分支会形成第二套 WYSIWYG 管线，并重复解决光标、IME、选择、Undo 和事务一致性问题。

本设计将 mmap 迁入现有统一 WYSIWYG 编辑协议，并增加可供其他结构化视图复用的“语义命中目标”。源码始终是唯一真实状态，画布是结构化投影。

## 2. 目标

首版形成标题编辑与结构编辑闭环：

- 点击标题文字直接放置光标，支持英文、中文 IME、选择、删除、粘贴和撤销重做。
- 点击卡片留白、边框或状态区域选中整个节点。
- 整节点选中后直接输入或启动 IME，以新输入替换原标题并进入标题编辑。
- `Enter` 新建同级节点，`Tab` 新建子节点。
- `Cmd+[` / `Cmd+]` 调整当前节点及整棵子树的层级；Windows/Linux 使用主修饰键对应快捷键。
- 整节点选中时 `Backspace` / `Delete` 删除节点及整棵子树。
- 空标题是合法状态，画布显示不落盘的“输入主题”占位符。
- 非法 MMF 显示明确错误态，引导切换源码修复，不显示过期画布。

## 3. 非目标

首版不包含：

- 多节点选择、框选和拖拽排序。
- 节点属性或备注的画布编辑。
- 容错解析画布。
- 放射状、鱼骨图等新布局。
- 增量 AST 或增量布局优化。

## 4. 核心原则

1. `DocumentView` 中的 MMF 源码是唯一可写状态。
2. Tree、布局、命中几何和预编辑文本都是可丢弃投影。
3. 所有文本及结构修改先生成 `EditPlan`，再由 app 统一执行事务。
4. mmap 不直接调用 `DocViewMut::replace_range()`，不维护独立 Undo 栈，也不全量序列化 Tree 覆盖源码。
5. `ui` 只定义通用纯数据协议；`app` 只处理文档光标、选择和事务；`markdown` 负责 MMF 语义。
6. 插件协议不得让 `ui` 依赖 `DocumentView`、Workspace 或 app 状态结构。

## 5. 架构边界

### 5.1 `ui::plugin`

定义通用编辑命中、事务与选择结果，不认识 mmap 或节点：

```rust
pub enum EditHitTarget {
    TextCaret {
        byte_offset: usize,
    },
    SourceObject {
        source_range: Range<usize>,
    },
    ClearFocus,
}
```

`TextCaret` 表示可编辑文本中的精确光标位置。`SourceObject` 表示由一段源码支撑的结构化对象；app 只把该范围设置为文档选择，不解释对象语义。`ClearFocus` 将文档光标移动到不属于任何标题的文档末尾并清除选择，用于点击画布空白。整节点选中态不额外保存一个位于选择内部的光标；进入标题编辑时，由 mmap 根据最新 `title_byte_range` 生成明确的选择更新计划。

### 5.2 `app`

app 负责：

- 将 `EditHitTarget` 映射为 `DocumentView` 光标和选择区间。
- 构造带 `source_generation` 的 `EditRequest`。
- 校验并原子执行插件返回的 `EditPlan`。
- 统一维护 dirty 状态、Undo/Redo、保存、IME 生命周期和系统候选窗位置。
- 将源码、光标、选择、预编辑文字和闪烁相位同步给活动插件。

app 不解析 MMF，不保存节点索引，也不包含 mmap 专用编辑分支。

### 5.3 `markdown::MindmapView`

`MindmapView` 负责：

- MMF 解析、源码范围计算和诊断。
- Tree 到布局与命中几何的投影。
- 根据文档光标和选择区间派生交互焦点。
- 将通用编辑意图翻译为 MMF 局部源码事务。
- 渲染节点、整卡选中、标题选择、光标、占位符和 IME 预编辑文字。

插件能力声明为：

```rust
allows_editing()            -> true
handles_own_rendering()     -> true
shows_cursor()              -> false
needs_cursor_blink_wakeup() -> true
edit_policy()               -> MindmapEditPolicy
```

## 6. 状态模型

交互状态由 `DocumentView` 光标/选择与当前 AST 范围派生，不另设多个布尔字段：

```rust
enum MindmapFocus {
    None,
    NodeSelected {
        node_index: usize,
    },
    TitleEditing {
        node_index: usize,
        cursor_byte: usize,
    },
    TitleTextSelected {
        node_index: usize,
        range: Range<usize>,
    },
}
```

判定规则：

- 折叠光标位于 `title_byte_range`：`TitleEditing`。
- 非空选择完全位于同一 `title_byte_range`：`TitleTextSelected`。
- 选择区间等于节点 `subtree_source_range`：`NodeSelected`。
- 其他光标或选择：`None`。

Undo、Redo、源码视图切换或外部文件更新后，焦点会从新源码范围重新派生，不依赖旧节点索引。

## 7. 源码范围

每个节点明确维护以下范围：

- `title_byte_range`：标题正文，不包含 `#` 和标题前空格。
- `heading_marker_range`：标题行开头的连续 `#`，只用于层级调整。
- `child_insertion_byte`：当前节点自身标题、属性和备注结束后，首个直接子节点之前的字节位置；没有子节点时等于 `subtree_source_range.end`。
- `subtree_source_range`：从当前标题行开头到下一个层级小于等于当前节点的标题之前，包含节点属性、备注和全部后代。

删除使用独立的 `subtree_delete_range()` 计算需要吸收的相邻换行，确保不吞掉相邻节点，也不留下损坏结构的分隔空行。

层级调整只修改解析器确认的 `heading_marker_range`，不得扫描字符串替换 `#`，以免修改备注、代码块或属性值。

## 8. 命中与焦点切换

布局为每个节点生成：

```rust
struct NodeHitGeometry {
    card_rect: Rect,
    title_rect: Rect,
    grapheme_edges: Vec<f32>,
    title_byte_range: Range<usize>,
    subtree_source_range: Range<usize>,
}
```

所有文字几何使用 Unicode grapheme 边界，不以 Rust `char` 数量近似光标位置。

命中规则：

- 标题文字：返回 `TextCaret`，根据 grapheme 边缘选择最近的源码字节。
- 卡片留白、边框或状态区域：返回 `SourceObject`，范围为 `subtree_source_range`。
- 画布空白：返回 `ClearFocus`，清除当前焦点，不修改源码。
- 标题内拖动：只选择当前标题文字；首版不允许跨节点文字选择。

状态切换：

- 点击标题文字立即进入标题编辑。
- 点击卡片非文字区域选中整节点。
- 标题编辑时按 `Escape`，转为整节点选中。
- 整节点选中时按 `Enter`，进入标题编辑并把光标放在标题末尾。
- 整节点选中时直接输入或开始 IME，以输入内容替换原标题。

## 9. 统一编辑事务

事务支持多个非重叠 replacement：

```rust
pub struct EditTransaction {
    pub source_generation: u32,
    pub replacements: Vec<TextReplacement>,
    pub selection_after: EditSelection,
}
```

app 执行前必须验证：

- 事务 `source_generation` 与执行时的 DocumentView generation 一致。
- 所有范围处于文档边界内并落在 UTF-8 字节边界。
- replacement 互不重叠。
- `selection_after` 在事务后的文档边界内。

不修改源码的焦点转换使用 `EditPlan::SetSelection(EditSelection)`，复用同一套选择边界校验。标题编辑态按 `Escape` 返回当前节点的 `SourceObject` 范围；整节点选中态按 `Enter` 返回标题末尾的折叠光标。两者都不产生 Undo。

验证成功后，app 按范围起点从后向前执行 replacement，并把整个事务记录为一个 Undo 单元。任一验证失败则整笔拒绝，不允许部分写入。

### 9.1 标题编辑

- 折叠光标输入：在光标位置插入。
- 标题文字选择后输入：替换选择区。
- `Backspace` / `Delete`：只删除标题文字，不越过 `title_byte_range` 修改标题标记或相邻结构。
- 整节点选中后输入：忽略对象选择的完整范围，只替换 `title_byte_range`；完成后清除对象选择并进入标题编辑。

### 9.2 创建节点

- 编辑态 `Enter`：在当前 `subtree_source_range.end` 插入同级空标题。
- `Tab`：在 `child_insertion_byte` 插入子节点空标题，即当前节点自身内容之后、首个子节点之前。
- 新标题继承当前文档换行风格，只插入必要标题行。
- 新节点标题为空，事务后光标位于标题起点。
- 根节点不允许通过 `Enter` 创建同级节点，但允许通过 `Tab` 创建子节点。

### 9.3 调整层级

- `Cmd+]`：当前节点降级为前一个同级节点的最后一个子节点。
- `Cmd+[`：当前节点升级为原父节点之后的同级节点。
- Windows/Linux 使用主修饰键对应快捷键。
- 当前节点全部后代与其一起移动，每个标题标记增加或删除一个 `#`。
- 没有前一个同级节点时不能降级。
- 根节点不能升级或降级。
- 不满足前置条件时返回 `Consume`，不修改源码，不产生 Undo。

层级调整通过同一事务中的多个 `TextReplacement` 修改标题标记，保留属性、备注、空行及其他手工格式。

### 9.4 删除节点

- 标题编辑态的 `Backspace` / `Delete` 只删除文字。
- 整节点选中态的 `Backspace` / `Delete` 删除 `subtree_delete_range`，即节点及全部后代。
- 根节点不能删除。
- 删除后优先选择删除位置之前的可见节点；没有则选择父节点，再没有则选择后一个节点。
- 删除与焦点移动属于同一个 Undo 单元。

## 10. 键盘导航

整节点选中态：

- `↑/↓`：前一个或后一个可见节点。
- `←`：父节点。
- `→`：第一个子节点。
- `Enter`：进入标题编辑，光标位于标题末尾。
- `Tab`：创建子节点并进入编辑。
- `Backspace/Delete`：删除整棵子树。

标题编辑态：

- `←/→/Home/End`：只移动标题文本光标。
- `↑/↓`：退出编辑并选择相邻可见节点。
- `Enter`：创建同级空节点并进入编辑。
- `Tab`：创建子节点并进入编辑。
- `Escape`：退出编辑，保留整节点选中。

## 11. 光标与 IME

mmap 自己绘制光标、标题文字选择和预编辑文字；app 继续提供闪烁时钟并设置系统 IME 候选窗。

- app 通过 `SetPreedit` 同步组合文字和组合光标。
- 标题编辑态将预编辑文字投影到当前光标位置。
- 整节点选中态用预编辑文字临时替代原标题，表达重命名预览。
- 预编辑期间不修改 DocumentView，不产生 Undo。
- 活动卡片以预编辑投影重新测量宽度，避免文字溢出。
- IME Commit 后才生成一次标题替换事务。
- `CursorScreenPos` 返回预编辑光标的实际屏幕矩形，系统候选窗随组合光标移动。

滚动、DPI 和画布偏移统一通过同一视口变换作用于渲染、命中、光标和 IME 矩形，禁止调用方与插件重复减 offset。

## 12. 空标题

空标题行是合法 MMF 节点：

```markdown
##
```

标题标记后可以带可选空格；解析器必须保留该节点和零长度 `title_byte_range`。画布显示灰色“输入主题”占位符：

- 占位符不写入源码。
- 占位符不增加标题源码长度。
- 点击占位符可在空标题位置放置光标。
- 保存时允许空标题继续存在。

## 13. 解析状态与错误处理

解析和布局状态使用互斥枚举：

```rust
enum MindmapDocumentState {
    Ready {
        generation: u32,
        tree: Tree,
        layout: LayoutTree,
    },
    Invalid {
        generation: u32,
        diagnostic: MmfDiagnostic,
    },
}
```

行为：

- `UpdateSource` generation 变化后立即重新解析。
- 解析成功才构建布局与命中几何。
- 解析失败进入 `Invalid`，丢弃旧布局，禁止命中和编辑。
- 错误画布显示错误摘要、源码行列和切换源码修复入口。
- 不显示最后一次成功画布，避免视觉内容与真实源码不一致。
- Undo、Redo 或外部更新恢复合法源码后自动回到 `Ready`。

## 14. 缓存与性能

- generation、DPI、主题几何和预编辑投影未变化时复用布局。
- 普通滚动只更新视口变换，不重新解析或布局。
- 标题提交后首版允许全量 parse/layout，优先保证范围和事务正确性。
- LayoutTree 维护按 y 排序的可见节点索引，恢复真正的视口裁剪；不得继续全量提交所有节点的绘制命令。
- 增加大节点数文档基准，后续是否做增量解析或布局以测量结果为准。

## 15. 测试策略

### 15.1 解析与范围

- 空标题生成合法节点和零长度标题范围。
- 属性、备注、代码块、空行和嵌套后代正确纳入各自范围。
- 备注或代码块中的 `#` 不会成为标题标记。
- 删除范围不吞相邻节点、不破坏换行。
- 非法 MMF 诊断包含源码行列。

### 15.2 编辑策略与事务

- 整节点选中后字符与 IME 只替换标题。
- 编辑态删除文字，整节点选中态删除子树。
- Enter 创建同级，Tab 创建子节点。
- 升降级修改整棵子树的标题标记。
- 根节点限制和无前序同级限制不产生源码修改。
- 多 replacement 原子执行；generation、边界、UTF-8 或重叠校验失败时整笔拒绝。
- 每个结构操作只生成一个 Undo 单元，Redo 精确恢复。

### 15.3 几何与 Unicode

- ASCII、CJK、emoji 和组合字符的点击字节、光标位置一致。
- 文字、留白和边框分别产生正确语义命中。
- DPI、滚动和画布偏移变化后，命中、光标和 IME 共用坐标系。
- 空标题占位符可点击但不污染源码范围。

### 15.4 app 集成与回归

- mmap 同时满足可编辑与自绘插件能力。
- 点击、Escape、方向导航、输入和删除的状态转换正确。
- 英文输入、粘贴、中文 IME、Undo/Redo、保存闭环通过。
- Markdown WYSIWYG 原有编辑、IME、光标和选择测试继续通过。

## 16. 验收

实现完成必须满足：

- `cargo fmt` 无差异。
- 相关 crate 单元测试和集成测试通过。
- `cargo check` 通过。
- 执行 `./scripts/verify.sh`，因为修改跨越 `ui`、`app`、`markdown` 多模块协议。
- 手工验证 `.mmap.md` 的点击编辑、IME、同级/子级创建、升降级、子树删除、Undo/Redo、保存和非法源码修复流程。
