# mmap 主题体系设计

## 背景

当前 mmap 渲染已经接入现有 `Theme`，但颜色主要通过 `theme.scopes.get("mindmap.*")` 查询，布局尺寸也仍在 `MindmapView` 中按 DPI 写入若干常量。这种方式适合早期验证，但不适合形成长期可维护的主题体系：颜色 token 缺少结构，层级、状态、优先级与几何参数都靠字符串约定表达，主题文件也无法清晰描述 mmap 的视觉规则。

本设计将 mmap 主题纳入现有 `ui::theme` / `ThemeRegistry` / `ThemeFile` 体系：代码提供稳定的主题框架与渲染决策规则，多个配色只作为配置文件或内置 `ThemeDefinition` 字段存在。系列之间共享同一套语义角色，用户切换配色时不需要重新理解状态颜色。

## 目标

- 建立强类型 `MindmapTheme`，作为 `Theme` 与 `ThemeDefinition` 的正式字段。
- 支持 mmap 的画布、节点、层级、状态、优先级、命名颜色和几何 token。
- 内置两组有逻辑关系的配色系列：工作系与图谱系，各自提供深色和浅色版本。
- 保持现有主题注册、解析、继承与 gamma correction 流程，不新增 mmap 专用 registry。
- 让 mmap 渲染消费纯数据主题结构，避免 `ui` 访问 `app` 状态，遵守跨层解耦边界。
- 保留旧 `mindmap.*` scope 作为兼容 fallback，但新主题文件和内置主题均使用强类型字段。

## 非目标

- 不设计独立 mmap 主题选择 UI。主题选择仍跟随现有 app 主题机制。
- 不把主题系列建模为 Rust enum。系列关系属于主题文件、文档和未来 UI 分组信息。
- 不在第一阶段实现复杂视觉效果，例如阴影、渐变、连线动画或节点图标。
- 不改变 MMF 文件格式中的 `theme` 字段语义；MMF 内部主题字段可以后续单独设计。

## 架构

`ui::theme` 增加 `mindmap` 模块，并从 `Theme` 与 `ThemeDefinition` 暴露：

```rust
pub struct Theme {
    pub name: String,
    pub is_dark: bool,
    pub palette: ColorPalette,
    pub editor: EditorTheme,
    pub markdown: MarkdownTheme,
    pub novel: NovelTheme,
    pub mindmap: MindmapTheme,
    pub scopes: HashMap<String, [f32; 4]>,
}

pub struct ThemeDefinition {
    pub display_name: String,
    pub is_dark: bool,
    pub palette: ColorPalette,
    pub editor: EditorTheme,
    pub markdown: MarkdownTheme,
    pub novel: NovelTheme,
    pub mindmap: MindmapTheme,
    pub scopes: BTreeMap<String, [f32; 4]>,
}
```

职责边界：

- `ui::theme::mindmap` 定义纯数据结构、默认值和 gamma correction。
- `ui::theme_file` 解析 `[mindmap.*]` TOML 覆盖，并合并到 `ThemeDefinition`。
- `Theme::from_definition()` 复制 `MindmapTheme` 并执行 gamma correction。
- `crates/markdown/src/mmf/canvas.rs` 使用 `theme.mindmap` 渲染节点、连线和文本。
- `crates/markdown/src/mindmap_view.rs` 将 `theme.mindmap.geometry` 乘以 DPI，生成现有 `LayoutConstants`。
- `theme.scope_color("mindmap.*")` 仅作为旧主题 fallback，不作为新代码首选路径。

## 数据结构

核心结构按画布、节点、语义和几何拆分：

```rust
pub struct MindmapTheme {
    pub canvas: MindmapCanvasTheme,
    pub node: MindmapNodeTheme,
    pub semantic: MindmapSemanticTheme,
    pub geometry: MindmapGeometry,
}

pub struct MindmapCanvasTheme {
    pub background: [f32; 4],
    pub connector: [f32; 4],
    pub connector_hover: [f32; 4],
    pub selection: [f32; 4],
    pub focus_ring: [f32; 4],
}

pub struct MindmapNodeStyle {
    pub fill: [f32; 4],
    pub border: [f32; 4],
    pub text: [f32; 4],
    pub accent: [f32; 4],
}

pub struct MindmapNodeTheme {
    pub default: MindmapNodeStyle,
    pub root: MindmapNodeStyle,
    pub depth: Vec<MindmapNodeStyle>,
}

pub struct MindmapSemanticTheme {
    pub status: MindmapStatusTheme,
    pub priority: MindmapPriorityTheme,
    pub named: BTreeMap<String, MindmapNodeStyle>,
}

pub struct MindmapStatusTheme {
    pub todo: MindmapNodeStyle,
    pub doing: MindmapNodeStyle,
    pub done: MindmapNodeStyle,
    pub blocked: MindmapNodeStyle,
    pub canceled: MindmapNodeStyle,
}

pub struct MindmapPriorityTheme {
    pub p0: MindmapNodeStyle,
    pub p1: MindmapNodeStyle,
    pub p2: MindmapNodeStyle,
    pub p3: MindmapNodeStyle,
}

pub struct MindmapGeometry {
    pub card_height: f32,
    pub card_padding_x: f32,
    pub card_padding_y: f32,
    pub level_indent: f32,
    pub sibling_gap: f32,
    pub card_radius: f32,
    pub connector_width: f32,
}
```

`MindmapNodeStyle` 是复用单元。根节点、默认节点、深度节点、状态节点、优先级节点和命名颜色都使用同一种结构，避免为每类样式设计不同字段。

## 主题文件形态

主题文件新增可选 `[mindmap]` 配置段，所有字段都是局部覆盖，未配置时继承 base theme：

```toml
[mindmap.canvas]
background = "#F6F3EC"
connector = "#C4B9AA"
connector_hover = "#AFA391"
selection = "#4F8FCF33"
focus_ring = "#4F8FCF"

[mindmap.node.default]
fill = "#FFFFFF"
border = "#DED7CE"
text = "#36424C"
accent = "#9AA6B2"

[mindmap.node.root]
fill = "#27313A"
border = "#27313A"
text = "#FFFFFF"
accent = "#F2A65A"

[[mindmap.node.depth]]
fill = "#E9F1F7"
border = "#9EB7C9"
text = "#24313D"
accent = "#4F8FCF"

[mindmap.semantic.status.done]
fill = "#EAF5ED"
border = "#97C8A3"
text = "#24382A"
accent = "#2F9E58"

[mindmap.semantic.priority.p0]
fill = "#FFE9E7"
border = "#D97474"
text = "#5E2728"
accent = "#CC3D3D"

[mindmap.semantic.named.blue]
fill = "#E9F1F7"
border = "#9EB7C9"
text = "#24313D"
accent = "#4F8FCF"

[mindmap.geometry]
card_height = 32.0
card_padding_x = 16.0
card_padding_y = 6.0
level_indent = 240.0
sibling_gap = 8.0
card_radius = 6.0
connector_width = 1.5
```

解析规则：

- `ThemeFile` 增加 `mindmap: Option<MindmapFile>`。
- 每个颜色字段使用现有 `hex_color::parse_hex()`，字段名带完整路径，例如 `mindmap.semantic.priority.p0.accent`。
- `depth` 使用数组表，允许主题定义 1 到 N 个层级样式。
- 若 `depth` 为空，渲染时回退到 `node.default`。
- 几何字段允许局部覆盖，默认主题可以完全继承，展示型主题可调整间距与圆角。

## 渲染决策规则

mmap 渲染时先计算结构样式，再叠加语义样式：

1. 根节点使用 `node.root`。
2. 非根节点按 `depth` 取结构样式：`depth_styles[(depth - 1) % depth_styles.len()]`。若没有 depth 样式，使用 `node.default`。
3. 若节点有 `status`，使用对应 status style 作为主体样式。
4. 若节点有 `priority`，priority style 的 `accent` 与 `border` 覆盖当前样式，用于表达紧急程度。
5. 若节点有 `color` 且命中 `semantic.named[color]`，named style 最高优先级，覆盖主体样式。
6. 若多个语义字段同时存在，优先级为 `color > priority accent/border > status body > depth/root/default`。
7. 选中态与焦点态使用 `canvas.selection` 与 `canvas.focus_ring` 叠加，不改变节点自身语义。

这套规则固定在代码中。主题文件只提供 token，不提供条件逻辑。

## 内置配色系列

第一版内置四个 mmap 配色，归为两个系列。所有系列共享同一套角色语义：

```text
primary  -> 根节点 / 当前焦点 / 主要路径
info     -> 普通层级里的冷色分支
success  -> done / positive
warning  -> doing / P2 / attention
danger   -> blocked / P0 / destructive
muted    -> todo / canceled / secondary
```

### Workbench 系

默认工作系，贴近主编辑器气质，低干扰、高可读。节点主体低饱和，状态主要通过 `accent`、描边和小圆点表达。

深色默认：`mmap-workbench-dark`

```text
canvas      #121416
surface     #202831
primary     #F2A65A
info        #5DA9E9
success     #62C370
warning     #F2A65A
danger      #E65F5C
muted       #8B949E
```

浅色默认：`mmap-whiteboard-light`

```text
canvas      #F6F3EC
surface     #FFFFFF
primary     #27313A
info        #4F8FCF
success     #2F9E58
warning     #D9822B
danger      #CC3D3D
muted       #8A8176
```

### Atlas 系

知识图谱系，更有 mmap 独立识别度。它和 Workbench 不共享具体颜色，但共享角色语义。

深色变体：`mmap-atlas-dark`

```text
canvas      #181713
surface     #24221D
primary     #86D1C2
info        #7AA7D9
success     #8FD694
warning     #F6BD60
danger      #FF6B6B
muted       #9B927F
```

浅色变体：`mmap-paper-light`

```text
canvas      #FBFBF8
surface     #FFFFFF
primary     #4C6F91
info        #5B8CC0
success     #6B9F72
warning     #C8904D
danger      #C45D5D
muted       #8C877D
```

主题系列关系不进入 Rust 类型系统。它可以体现在主题 id、display name、文档和未来设置 UI 分组中。

## 兼容策略

现有 `mindmap.*` scopes 保留为 fallback：

- `mindmap.node_bg` -> `node.default.fill`
- `mindmap.node_border` -> `node.default.border`
- `mindmap.text` -> `node.default.text`
- `mindmap.root_bg` -> `node.root.fill`
- `mindmap.root_border` -> `node.root.border`
- `mindmap.root_text` -> `node.root.text`
- `mindmap.connector` -> `canvas.connector`

新字段存在时优先使用 `theme.mindmap`。旧 scopes 只在强类型字段保持默认值或迁移期间用于兼容，不鼓励新主题继续写 scopes。

## 实施阶段

### 阶段 1：主题结构与默认值

- 新增 `crates/ui/src/theme/mindmap.rs`。
- 将 `MindmapTheme` 加入 `Theme` 与 `ThemeDefinition`。
- 在 `Theme::from_definition()` 和 `gamma_correct()` 中处理 mmap 颜色。
- 给 `default_dark()` / `default_light()` 填入 Workbench Dark 与 Whiteboard Light。

### 阶段 2：主题文件解析

- 在 `crates/ui/src/theme_file.rs` 增加 `MindmapFile`、`MindmapCanvasFile`、`MindmapNodeStyleFile`、`MindmapGeometryFile` 等 partial 结构。
- 支持 `[mindmap.canvas]`、`[mindmap.node.*]`、`[mindmap.semantic.*]`、`[mindmap.geometry]`。
- 增加解析、继承、未知字段拒绝和非法颜色错误测试。

### 阶段 3：mmap 渲染迁移

- 在 `crates/markdown/src/mmf/canvas.rs` 引入样式解析函数，使用 `theme.mindmap`。
- 移除直接 `scopes.get("mindmap.*")` 的主路径，只保留显式 fallback helper。
- 在 `MindmapView::render()` 中从 `theme.mindmap.geometry` 派生 `LayoutConstants`，并按 DPI 缩放。
- 保持 `LayoutConstants` 仍属于 markdown/mmf 布局输入，不让 `ui` 依赖 markdown。

### 阶段 4：内置系列与文档

- 补齐 Atlas Dark 与 Paper Light 的内置配置或示例主题文件。
- 在主题配置文档中说明 mmap token、语义角色和 fallback 策略。
- 可选：提供 `docs/specs` 或 README 中的主题文件示例。

## 测试策略

- `edit-plus-ui` 单元测试：
  - `Theme::from_definition()` 会 gamma-correct mmap 颜色。
  - 默认深/浅主题包含非空 `MindmapTheme`。
  - `ThemeFile` 能解析局部 `[mindmap]` 覆盖。
  - 非法 hex 返回带完整字段路径的错误。
  - 未知 mmap 字段被 `deny_unknown_fields` 拒绝。

- `edit-plus-markdown` 或相关模块测试：
  - 根节点使用 root 样式。
  - 普通节点按 depth 循环取样式。
  - status 覆盖主体样式。
  - priority 覆盖 accent/border。
  - color 命中 named 时拥有最高优先级。

- 集成验证：
  - `cargo fmt`
  - `cargo test -p edit-plus-ui theme_file`
  - `cargo test -p edit-plus-ui theme`
  - `cargo test -p edit-plus-markdown mmf`
  - 修改超过多模块时运行 `./scripts/verify.sh`

## 风险与取舍

- 强类型字段会让 `ThemeDefinition` 扩大，但换来配置可读性和编译期约束，优于继续扩展字符串 scopes。
- `depth: Vec<MindmapNodeStyle>` 比固定 `level_1` 到 `level_5` 更灵活，但解析与 fallback 要测试清楚。
- 几何 token 开放给主题文件可能导致主题切换时布局变化。默认主题保持一致几何，展示型主题才调整，避免日常使用中跳动过大。
- 兼容 fallback 会短期增加渲染路径复杂度。迁移完成后可在后续版本移除旧 `mindmap.*` scopes。

## 后续扩展

- mmap 文件内 `theme = "..."` 是否覆盖 app 当前主题。
- 主题选择 UI 如何显示系列分组。
- 节点图标、状态徽标和折叠态样式是否进入 `MindmapTheme`。
