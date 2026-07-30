# 思维导图所见即所得编辑器设计

## 1. 目标

基于 MMF v0.1（Markdown Mindmap Format）实现所见即所得的思维导图编辑体验：

- **混合模式**：默认展示思维导图画布，可随时切换到 MMF 源码视图编辑
- **右分支树布局**：根节点最左，子节点逐级向右缩进，自上而下排列
- **纯标题卡片**：画布节点只显示标题文字，属性/备注在源码侧查看
- **大纲键盘模型 + 边界跨级**：Tab/Shift+Tab 调整层级，方向键在同级间移动，到达边界时跳转到父/子节点
- **点击定位 + 源码层文本编辑**：点击画布节点将光标映射到 MMF 源码对应位置，用户在源码层打字、IME 输入、删除，画布仅做布局响应
- **首版聚焦右分支树，布局层抽象预留放射状等扩展点**

## 2. 核心原则：源码是唯一真实状态，插件提供物理坐标

**文本编辑在源码层，画布是可视化投影。光标和 IME 位置由插件提供物理坐标给 app 层绘制。**

```
app 层 (大脑 + 文本编辑器 + 光标/IME 绘制)    MindmapView (布局/渲染专家)
──────────────────────────────────          ─────────────────────────────
winit → EditCommand                         PluginQuery::HitTestCanvas
    → DocumentView 执行文本编辑               PluginQuery::ContentHeight
    → Undo/Redo 逐字记录                     PluginQuery::CursorRect
    → IME 组合态、CJK 跳转                   PluginQuery::VisualMove
    → Dispatch to plugin
                                             PluginMessage::UpdateSource
    app ──── PluginMessage.InterceptKey ──→ plugin  (结构编辑按键)
    app ──── PluginMessage.UpdateSource ──→ plugin  (源码变更通知)
    app ←── PluginQuery.HitTestCanvas ──── plugin   (画布→源码映射)
    app ←── PluginQuery.CursorRect ─────── plugin   (源码字节→画布像素)
    app ←── PluginQuery.VisualMove ─────── plugin   (方向键画布导航)
```

- **文本输入走 app 层 DocumentView**——插件不接管字符输入、删除和 IME 组合
- **插件绝不生成 Patch 或序列化回写源码**——源码永远由 app 层通过 `DocumentView` 直接修改
- **光标和 IME 物理坐标由插件提供**——app 在每一帧通过 `CursorRect(byte_offset)` 查询当前光标在画布上的精确像素位置，app 在此位置绘制闪烁光标竖线，并设置系统 IME 候选窗位置
- **结构编辑通过 InterceptKey 拦截**——Tab/Enter/方向键等由插件处理，直接调用 `doc.replace_range()` 做精确局部修改
- `shows_cursor()` 返回 `false`，因为光标由 app 层绘制——但 app 使用插件提供的画布物理坐标而非源码层的行列坐标

## 3. 架构概览

```
crates/markdown/src/
  mmf/
    mod.rs        → MMF 模块入口
    parser.rs     → MMF 文本 → Tree
    model.rs      → Tree, Node, NodeProps 数据模型
    layout.rs     → 右分支树布局算法，Tree → LayoutTree (含坐标)
    canvas.rs     → 画布渲染：卡片、连线、选区高亮 → DrawList
    edit.rs       → 内部结构编辑：InterceptKey → 定位 source_range → doc.replace_range()
  mindmap_view.rs → MindmapView impl ViewPlugin (薄协调器)
```

`MindmapView` 作为独立 `ViewPlugin`，与 `MarkdownView` 平级。MMF 文件由 `MindmapPluginFactory` 接管，匹配 `.mmap.md` 扩展名。

## 4. MMF 源码 ↔ 思维导图 Tree 映射

```
MMF 源码 (DocumentView.doc.text())          Tree (内部 AST，只读投影)
─────────────────────────────              ─────────────────────────────
# 产品规划                    parse()       Node { title: "产品规划",
## 数据同步              ──────────→         children: [
### 本地文件                                    Node { title: "数据同步",
### 云端同步                                      children: [
## AI 生成                                          Node { title: "本地文件" },
                                                    Node { title: "云端同步" }
                                                  ] },
                                                Node { title: "AI 生成" }
                                              ] }
```

Tree 是 MMF 源码的**只读结构化投影**。插件不把 Tree 序列化回源码——源码修改永远由 app 层通过 `DocumentView` 执行。

## 5. 数据模型

```rust
// ── MMF 解析后的核心 AST（只读投影）──

pub struct Tree {
    pub version: u32,
    pub root: Node,
    pub global_props: HashMap<String, String>,  // layout, theme, direction
}

pub struct Node {
    pub title: String,                   // 标题文本（纯文本，不含 #）
    pub children: Vec<Node>,
    pub props: Option<NodeProps>,        // None = 无 toml node 块
    pub note: Option<String>,            // 正文备注
    pub source_range: Range<usize>,      // 此节点在 MMF 源码中的字节范围
    pub title_byte_range: Range<usize>,  // 标题文字在源码中的精确字节范围（不含 # 前缀）
    pub heading_level: u8,              // # 的个数 (1=根节点, 2=一级子, ...)
}

pub struct NodeProps {
    pub id: Option<String>,
    pub priority: Option<String>,        // P0/P1/P2/P3
    pub status: Option<String>,          // todo/doing/done/blocked/canceled
    pub owner: Option<String>,
    pub collapsed: bool,
    pub tags: Vec<String>,
    pub color: Option<String>,           // 语义颜色名
}

// ── 布局计算结果 ──

pub struct LayoutNode {
    pub x: f32,                          // 卡片左上角 x（画布坐标）
    pub y: f32,                          // 卡片左上角 y
    pub w: f32,                          // 卡片宽度（文本测量 + padding）
    pub h: f32,                          // 卡片高度
    pub node_idx: usize,                 // 对应 Tree 中节点的 DFS 索引
    pub depth: u8,                       // 层级深度
    pub connector_from: (f32, f32),      // 连线起点（父节点右边缘中点）
    pub connector_to: (f32, f32),        // 连线终点（当前节点左边缘中点）
}

pub struct LayoutTree {
    pub nodes: Vec<LayoutNode>,          // DFS 序扁平化
    pub total_w: f32,                    // 画布总宽度
    pub total_h: f32,                    // 画布总高度
}

// ── 交互层 HitMap（像素 → 源码字节映射）──

pub struct HitMap {
    pub node_rects: Vec<Rect>,           // node_rects[i] = 节点 i 的卡片命中区域
    pub title_char_edges: Vec<Vec<f32>>, // title_char_edges[i][j] = 节点 i 第 j 个字符
                                         //   的右边缘 x 坐标（用于像素→字节偏移）
}

/// Hit-test 结果：画布坐标 → 源码字节偏移
pub struct HitResult {
    pub byte_offset: usize,              // 在源码中的精确字节位置
    pub node_idx: usize,                 // 命中的节点索引
}

// ── 内部编辑命令（仅在 crates/markdown 内部使用，绝不暴露到 ui::plugin）──

enum MindmapEdit {
    Indent { node_idx: usize },          // Tab → 降级
    Outdent { node_idx: usize },         // Shift+Tab → 升级
    NewSibling { after_idx: usize },     // Enter → 创建同级空节点
    NewChild { parent_idx: usize },      // Ctrl+Enter → 创建子节点
    Delete { node_idx: usize },
    MoveUp { node_idx: usize },
    MoveDown { node_idx: usize },
}
```

`MindmapEdit` 是 `crates/markdown` 模块内部枚举，由 `MindmapView` 将 `PluginMessage::InterceptKey` 翻译而来。**绝对不暴露到 `ui::plugin` 层**。

## 6. 布局算法

右分支树，两层遍历：

```
第一遍：自底向上计算子树高度
  compute_subtree_height(node):
    如果 node 无子节点 → CARD_HEIGHT
    否则 → sum(child_subtree_heights) + gaps，取 max(CARD_HEIGHT, sum)

第二遍：自顶向下分配坐标
  assign_positions(node, depth, y_offset):
    x = depth * LEVEL_INDENT
    当前节点 y = y_offset + (subtree_h - CARD_HEIGHT) / 2  (垂直居中于子树)
    将当前节点加入结果
    cursor = y_offset
    遍历子节点:
      assign_positions(child, depth+1, cursor)
      cursor += child_subtree_h + SIBLING_GAP
```

**连线**：直角折线——从父节点右边缘中点水平出发，在父子间距一半处转折，垂直延伸到子节点左边缘中点。

**默认尺寸常量**（通过 Theme 读入，非硬编码）：

| 常量 | 默认值 |
|---|---|
| CARD_HEIGHT | 32px |
| CARD_PADDING_X | 16px |
| CARD_PADDING_Y | 6px |
| LEVEL_INDENT | 240px |
| SIBLING_GAP | 8px |
| CARD_RADIUS | 6px |
| CONNECTOR_WIDTH | 1.5px |

## 7. 画布渲染

### 7.1 渲染输出

`canvas::render()` 生成 `DrawList`（`ViewPlugin::render()` 的标准返回类型），包含：

1. 连线（直角折线，位于卡片下层）
2. 卡片背景（圆角矩形，按主题色）
3. 卡片边框
4. 标题文字（shaping → glyph placement）
5. 根节点特殊样式（加粗、强调色填充）
6. 选中态高亮（蓝色边框覆盖）

**光标不在 `canvas::render()` 中绘制**。app 层在插件 `render()` 返回后，通过 `PluginQuery::CursorRect` 查询当前光标字节偏移对应的画布像素坐标，由 app 层在画布坐标系中叠加绘制闪烁光标竖线。IME 候选窗也由 app 层通过同一查询定位到该物理坐标。

### 7.2 视口裁剪

利用 `LayoutTree.nodes` 按 y 坐标有序的特点，二分查找可见范围，只渲染视口内 ± 一个缓冲区高度的节点。

## 8. 编辑管线（两种编辑路径）

### 8.1 文本编辑路径（点击 → 定位 → 源码编辑 + 光标/IME 物理同步 → 刷新）

```
用户点击画布节点卡片上的文字
  │
  ├── 1. MindmapView.query(HitTestCanvas { x, y, ... })
  │       → HitMap 查找 pixel → char → title_byte_range 内的精确字节偏移
  │       → 返回 PluginResponse::HitResult(Some(HitResult { byte_offset, node_idx }))
  │
  ├── 2. app 层收到 byte_offset，设置 DocumentView 光标到该字节位置
  │
  ├── 3. 每帧渲染时，app 查询 CursorRect(byte_offset)
  │       → 插件返回 PluginResponse::CursorRect(Some((x, y, height)))
  │       → x, y 是光标在画布上的精确像素坐标（基于 title_char_edges 计算）
  │       → height 是当前节点的卡片字号行高
  │       → app 在该物理位置绘制闪烁光标竖线
  │       → IME 候选窗也被钉在这个物理坐标上
  │
  ├── 4. 用户敲键盘（中文 IME、英文、Backspace 等）
  │       → app 层 DocumentView 直接修改 MMF 源码
  │       → 光标字节偏移随编辑变化，下一帧 CursorRect 返回更新后的物理坐标
  │       → Undo 自动逐字记录（app 层标准行为）
  │
  └── 5. app 发送 PluginMessage::UpdateSource { text, generation }
          → MindmapView 收到，重新 parse → layout，卡片宽度自适应更新
```

**用户始终在画布卡片上看到闪烁光标，IME 候选窗也始终跟随光标位置——光标物理坐标由插件通过 CursorRect 提供，绘制和 IME 定位由 app 层执行。**

### 8.2 结构编辑路径（按键拦截 → 直接修改源码）

```
用户按下 Tab（对焦点所在节点）
  │
  ├── 1. app 层 dispatch 层发现 MindmapView 处于活跃态
  │       → 发送 PluginMessage::InterceptKey { key: Tab, modifiers: None }
  │
  ├── 2. MindmapView.handle_message(InterceptKey(Tab, None), doc)
  │       → 内部翻译为 MindmapEdit::Indent(node_idx)
  │       → 确定节点的 title_byte_range（如 "# 本地文件" 中标题文本在字节 72..86）
  │       →
  │
  ├── 3. 直接用 doc: &mut dyn DocViewMut 修改源码：
  │       doc.replace_range(title_byte_range.start..title_byte_range.start, "#")
  │       // 在标题前插入 "#"，使 ### → ####
  │       // 对此节点的所有子节点，同样各加一个 "#"
  │       → 返回 true（消费事件）
  │
  └── 4. app 层检测到源码被修改，自动触发 Undo 记录
          并发送 PluginMessage::UpdateSource，插件重新 parse+layout
```

**原则**：
- 结构编辑用 `doc.replace_range()` 做精确局部替换，**绝不**全量序列化 Tree → 覆盖源码
- 全量序列化会丢失用户手打的空行、空格格式，并产生巨大的 Undo 块
- 每次结构修改可能产生多个 `replace_range()` 调用（节点自身 + 所有子节点），通过 `doc.begin_edit() / end_edit()` 合并为一个 Undo 单元

## 9. 键盘交互

### 9.1 文本输入键（被 app 层文本编辑器消费，插件不拦截）

所有可打印字符、Backspace、Delete（当光标在标题文字内时）、IME 组合键、CJK 输入法。

### 9.2 结构编辑键（通过 InterceptKey 发送给插件）

| 按键 | 插件动作 |
|---|---|
| Tab | Indent——在节点标题前插入 "#" + 子节点各加 "#" |
| Shift+Tab | Outdent——在节点标题前删除一个 "#" + 子节点各删 "#" |
| Enter | NewSibling——在当前节点之后插入新的 `## title` 行 |
| Ctrl+Enter | NewChild——在当前节点子树末尾插入新子节点行 |
| Delete (选中节点时) | Delete——删除节点的源码范围 |
| Escape | 取消当前操作 |

### 9.3 画布导航键（通过 PluginQuery::VisualMove 查询新位置，然后 app 设置光标）

| 按键 | 插件响应 |
|---|---|
| ↑/↓ | 查询同级上/下一个节点的 title_byte_range，返回新光标位置 |
| ← | 光标在标题首位时：返回父节点的 title_byte_range。否则：返回 title_byte_range 前移一字 |
| → | 光标在标题末尾时：若有子节点，返回第一个子节点位置。否则：后移一字 |

### 9.4 光标与 IME 的物理定位

- 用户在画布卡片上看到闪烁光标（由 app 层在 `CursorRect` 返回的物理坐标处绘制）
- 中文输入法候选窗显示在光标下方（app 层通过同一 `CursorRect` 坐标调用系统 `set_ime_cursor_area`）
- 光标随编辑实时移动——每帧 app 查询 `CursorRect(byte_offset)`，坐标随布局变化自动更新
- 点击节点卡片文字 → HitTestCanvas → app 设置光标到对应源码字节偏移 → 下一帧光标出现在画布卡片上
- 点击画布空白区域 → 光标保持当前位置，不丢失

## 10. PluginMessage / PluginQuery 扩展

### 10.1 新增 PluginMessage 变体

```rust
// crates/ui/src/plugin.rs

pub enum PluginMessage {
    // ... 现有变体保持不变 ...

    /// 请求插件拦截处理结构编辑按键。
    /// 插件可调用 doc.replace_range() 直接修改源码。
    /// 返回 true 表示已消费，false 表示按键透传给源码编辑器。
    InterceptKey {
        key: Key,
        modifiers: Modifiers,
    },
}
```

`Key` 和 `Modifiers` 是 `crates/ui` 中已有的输入类型，不引入新依赖。

### 10.2 新增 PluginQuery 变体

```rust
pub enum PluginQuery {
    // ... 现有变体保持不变 ...

    /// 画布坐标 → 源码字节偏移。
    /// 用于点击画布节点时将光标定位到 MMF 源码的对应位置。
    HitTestCanvas { x: f32, y: f32, offset_x: f32, offset_y: f32 },
    // → PluginResponse::HitResult(Option<HitResult>)

    /// 源码字节偏移 → 画布精确像素坐标 (x, y, height)。
    /// 用于 app 层在画布上绘制文本光标 + 定位系统 IME 候选窗。
    /// height 为当前节点字号的行高，用于确定光标和 IME 候选窗的垂直高度。
    CursorRect(usize),
    // → PluginResponse::CursorRect(Option<(f32, f32, f32)>)

    /// 从当前源码位置执行画布导航（方向键移动焦点）。
    VisualMove {
        from_byte: usize,
        direction: Direction,  // Up, Down, Left, Right
    },
    // → PluginResponse::Position(Option<usize>)  —— 新光标在源码中的字节偏移
}
```

### 10.3 新增 PluginResponse 变体

```rust
pub enum PluginResponse {
    // ... 现有变体保持不变 ...

    HitResult(Option<HitResult>),
    /// 光标的画布物理坐标 (x, y, height)。None 表示字节偏移不在任何节点标题内。
    CursorRect(Option<(f32, f32, f32)>),
}
```

## 11. ViewPlugin 集成

```rust
impl ViewPlugin for MindmapView {
    fn name(&self) -> &str { "mindmap" }

    fn render(&mut self, doc: &dyn DocView, bounds: Rect, theme: &Theme,
              shaper: &mut Shaper, dpi: f32) -> DrawList {
        // 1. source_generation 比对，决定是否重新 parse+layout
        // 2. parse → Tree
        // 3. layout → LayoutTree + HitMap
        // 4. 视口裁剪 → 可见子集
        // 5. canvas::render() → DrawList (卡片 + 连线 + 选区高亮)
        // 6. 叠加选区高亮（从 PluginQuery::SelectionHighlights 查询）
    }

    fn handle_message(&mut self, msg: PluginMessage, doc: &mut dyn DocViewMut) -> bool {
        match msg {
            // 源码变更：清空缓存，下次 render 重新 parse+layout
            PluginMessage::UpdateSource { .. } => {
                self.tree = None;
                self.layout_tree = None;
                true
            }
            // 结构编辑拦截：翻译为 MindmapEdit，直接调用 doc.replace_range()
            PluginMessage::InterceptKey { key, modifiers } => {
                self.handle_intercept_key(key, modifiers, doc)
            }
            PluginMessage::SetScrollY(y) => {
                self.scroll_y = y;
                true
            }
            _ => false,
        }
    }

    fn query(&self, query: PluginQuery, doc: &dyn DocView) -> PluginResponse {
        match query {
            PluginQuery::HitTestCanvas { x, y, offset_x, offset_y } => {
                // 画布坐标 → char → title_byte_range 内的字节偏移
                PluginResponse::HitResult(self.hit_test(x - offset_x, y - offset_y))
            }
            PluginQuery::CursorRect(byte_offset) => {
                // 字节偏移 → HitMap 查字符位置 → 画布像素坐标 (x, y, height)
                PluginResponse::CursorRect(self.cursor_rect(byte_offset))
            }
            PluginQuery::VisualMove { from_byte, direction } => {
                // 在当前 Tree 中查找 from_byte 所属节点，返回导航后的新字节偏移
                PluginResponse::Position(self.visual_move(from_byte, direction))
            }
            PluginQuery::ContentHeight => {
                PluginResponse::Float(self.layout_tree.as_ref()
                    .map(|lt| lt.total_h).unwrap_or(0.0))
            }
            PluginQuery::ScrollY => PluginResponse::Float(self.scroll_y),
            _ => PluginResponse::None,
        }
    }

    fn shows_cursor(&self) -> bool { false }   // app 层绘制光标（使用插件提供的物理坐标）
    fn shows_gutter(&self) -> bool { false }   // 思维导图无行号
    fn allows_editing(&self) -> bool { true }  // 允许源码编辑
}
```

**缓存策略**：`source_generation` 相同时复用 `Tree + LayoutTree`（纯平移视口时）。收到 `UpdateSource` 后清空缓存，下一次 `render()` 重新 parse+layout。无"编辑态"特殊处理——编辑始终在源码层进行，画布被动响应。

**插件注册**：

```rust
registry.register(Box::new(MindmapPluginFactory));
// MindmapPluginFactory::can_handle() 匹配 *.mmap.md
```

## 12. 不在此版范围

- 放射状/鱼骨图布局（保留 `layout.rs` 抽象扩展点）
- 节点属性徽章/备注面板（纯标题卡片先行）
- 拖拽调整布局
- 节点折叠动画
- XMind/OPML 导入导出
- 协同编辑（MMF v0.2 的非树状关联）
