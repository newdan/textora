# 阶段 10：TabBar 增强方案（基于 Zed 分析）

> 前置条件：阶段 9（多 buffer + 基础 Tab UI）已完成
> 分析来源：`plans_zed_menu_tabbar_analysis.md`
> 约束：每阶段改动 ≤ 3 个文件；每阶段独立可编译/可测试

---

## 概述

阶段 9 实现了基础 TabBar（GPU 矩形 + cosmic-text 标签 + 点击切换/关闭/拖拽）。
本阶段在 9 的基础上，借鉴 Zed 的设计模式，逐步增强 TabBar 的可用性和视觉表现。

**核心原则**：不引入 GPUI 依赖，所有增强沿用 edit+ 现有的 wgpu + cosmic-text 渲染管线。

---

## 10.1 Tab 状态指示器（脏/冲突圆点）

### 目的

当前脏标记用颜色区分（`dirty: bool`），不够醒目。改为圆点指示器（Zed 的 `render_item_indicator` 模式）。

### 设计

```rust
// tab_bar.rs 新增
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabIndicator {
    None,             // 干净文件
    Dirty,            // 有未保存修改 → 蓝色圆点 (Color::Accent)
    Conflict,         // 有冲突 → 黄色圆点 (Color::Warning)
}

impl TabIndicator {
    pub fn for_doc(dirty: bool, has_conflict: bool) -> Self {
        match (has_conflict, dirty) {
            (true, _) => Self::Conflict,
            (false, true) => Self::Dirty,
            (false, false) => Self::None,
        }
    }
}
```

### 波及文件

| 文件 | 改动 |
|---|---|
| `crates/app/src/tab_bar.rs` | 新增 `TabIndicator` 枚举 + 渲染逻辑：在 Tab 标题左侧 6px 处画 3×3px 圆点；`layout_tabs` 预留 indicator 空间（+8px） |
| `crates/app/src/app.rs` | 传递 `document.has_conflict()` 状态（当前仅 `dirty` 字段，需确认是否已有冲突检测） |

### 渲染细节

```
┌──────────────────────────────┐
│ ● main.rs              [×]  │  ← Dirty dot (blue)
│   README.md            [×]  │  ← Clean, no dot
│ ● Cargo.toml           [×]  │  ← Conflict dot (yellow)
└──────────────────────────────┘
```

- 圆点颜色：Dirty → `theme.cursor`（蓝色）；Conflict → `Color::Warning`（黄色）
- 圆点位置：Tab 内左侧 padding 区域，垂直居中

### 验收

- 修改文本后 → 蓝点出现
- 保存后 → 蓝点消失
- 有 git 冲突时 → 黄点（优先级高于蓝点）
- indicator 不影响其他 tab 的 hit-test 区域

---

## 10.2 同名文件消歧义

### 目的

当打开两个同名文件（如两个 `README.md`）时，无法从 Tab 标题区分。Zed 的做法：自动添加父目录名。

### 设计

```rust
// tab_bar.rs 新增
pub fn compute_disambiguation(titles: &[String]) -> Vec<usize> {
    // 算法：对每个 title，看有多少个同名的
    // 同名者返回 detail=1 → 显示 "parent_dir/title"
    // 仍冲突返回 detail=2 → 显示 "grandparent/parent_dir/title"
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for t in titles { *counts.entry(t.as_str()).or_default() += 1; }
    titles.iter().map(|t| {
        if counts[t.as_str()] > 1 { 1 } else { 0 }
    }).collect()
}
```

### 波及文件

| 文件 | 改动 |
|---|---|
| `crates/app/src/tab_bar.rs` | 新增 `compute_disambiguation()`；`TabEntry` 增加 `disambiguation: Option<String>`（父目录名）；`layout_tabs` 接受 disambiguation 列表 |
| `crates/app/src/app.rs` | 调用 `compute_disambiguation` 传入 `doc_views` 的文件路径列表 |

### 视觉效果

```
┌─────────────────────────────────┐
│  README.md  src/README.md       │  ← 同名文件自动显示父目录
│  main.rs                        │
└─────────────────────────────────┘
```

父目录文本用稍淡的颜色 / 更小的字号区分。

### 验收

- 打开 `a/README.md` 和 `b/README.md` → 分别显示 `a/README.md` 和 `b/README.md`
- 打开三个 `README.md`（a/b/c）→ 全部显示父目录
- 仅一个 `README.md` → 不显示父目录（正常行为）
- 关闭一个同名文件后 → 另一个恢复简名

---

## 10.3 导航历史按钮（前进/后退）

### 目的

edit+ 已有 `tab_history: Vec<usize>`（记录打开顺序），但没有可视化导航 UI。
Zed 的做法：TabBar 左侧放前进/后退箭头按钮。

### 设计

```rust
// tab_bar.rs — 在 TabBarLayout 中新增
pub struct TabBarLayout {
    pub tabs: Vec<TabEntry>,
    pub nav_buttons: NavButtonLayout,  // ← 新增
    // ...
}

pub struct NavButtonLayout {
    pub enabled: bool,
    pub back_rect: [f32; 4],    // ← 箭头区域 (NDC)
    pub forward_rect: [f32; 4], // → 箭头区域 (NDC)
    pub back_enabled: bool,     // 是否有后退历史
    pub forward_enabled: bool,  // 是否有前进历史
}
```

### 波及文件

| 文件 | 改动 |
|---|---|
| `crates/app/src/tab_bar.rs` | `layout_tabs` 左侧预留 56px 给导航按钮；`hit_test` 增加 `NavBack`/`NavForward` 变体；`tab_bar_vertices` 渲染箭头字形 |
| `crates/app/src/app.rs` | 新增 `nav_history_back: Vec<usize>` 和 `nav_history_forward: Vec<usize>`（双向栈）；`switch_to` 时将旧索引推入 back 栈；点击后退时当前索引入 forward 栈 |

### 视觉效果

```
┌──────────────────────────────────────────┐
│ ←  →  │  main.rs   Cargo.toml      [×]  │
└──────────────────────────────────────────┘
```

- `←` 禁用时灰色（无后退历史）
- `→` 禁用时灰色（无前进历史）
- 点击 `←` → 回到上一个活动 tab，当前索引入 forward 栈
- 点击 `→` → 回到 forward 栈顶，当前索引入 back 栈

### 验收

- 从 tab A → tab B → 点击 `←` → 回到 A
- 回到 A 后点击 `→` → 回到 B
- 从 A → B → C → 点击 `←` → 回到 B → 点击 `←` → 回到 A
- 在 A 时 `←` 禁用

---

## 10.4 位置感知 Tab 边框（选中融合）

### 目的

当前所有 Tab 边框相同，选中 Tab 与编辑器内容区有视觉割裂感。
Zed 的做法：选中 Tab 无下边框（与内容区融合），相邻 Tab 共享边框。

### 设计

edit+ 是 GPU quads 渲染，需要把 Zed 的 `TabPosition` 概念转化为绘制逻辑。

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabBorderPosition {
    First,           // 第一个 tab
    MiddleBefore,    // 在活动 tab 之前
    Active,          // 当前活动 tab
    MiddleAfter,     // 在活动 tab 之后
    Last,            // 最后一个 tab
}
```

**关键规则**（参考 Zed 的 `Tab::render`）：

- **选中 Tab**：无下边框 + 左右边框始终存在。底部与编辑区直接相连。
- **非选中 Tab（First）**：右下边框
- **非选中 Tab（Last）**：左下边框
- **非选中 Tab（Middle, 在选中左边）**：左下边框
- **非选中 Tab（Middle, 在选中右边）**：右下边框

每个 tab 只需要画**自己负责的那一侧边框**，相邻 tab 之间不重复画。

### 波及文件

| 文件 | 改动 |
|---|---|
| `crates/app/src/tab_bar.rs` | `TabEntry` 增加 `border_position: TabBorderPosition`；`layout_tabs` 根据 active_index 计算每个 tab 的 border_position；`tab_bar_vertices` 按位置绘制不同的边框 quads |
| `crates/app/src/app.rs` | 无改动（active_index 已传递） |

### 视觉效果

```
Before (当前):
┌──────────┬──────────┬──────────┐
│ main.rs  │ Cargo    │ README   │  ← 所有 Tab 有完整边框，与内容间有白线
└──────────┴──────────┴──────────┘

After (改进后):
┌──────────┬──────────┬──────────┐
│ main.rs  │ Cargo    │ README   │  ← 选中 Tab 无下边框
│          │          │          │
│  (此区域与内容区融为一体)         │
```

### 验收

- 选中 tab 底部无边框线，与编辑区视觉融合
- 非选中 tab 底部有 1px 边框线
- 相邻 tab 之间共享边框（无双倍粗细）
- 切换选中 tab 后边框正确重绘

---

## 10.5 TabBar 可滚动（溢出处理）

### 目的

打开大量文件时 Tab 被挤压到不可读。Zed 的做法：设置最小 Tab 宽度，超出部分水平滚动。

### 设计

```rust
pub const MIN_TAB_WIDTH: f32 = 60.0;   // tab 最小宽度（像素）
pub const TAB_SCROLL_SPEED: f32 = 40.0; // 鼠标滚轮每次滚动像素

pub struct TabBarLayout {
    pub tabs: Vec<TabEntry>,
    pub overflow_left: bool,     // 左侧是否有被剪切的 tab
    pub overflow_right: bool,    // 右侧是否有被剪切的 tab
    pub scroll_offset: f32,      // 当前水平滚动偏移
    pub total_width: f32,        // 所有 tab 总宽度
    pub visible_width: f32,      // tab 区域可见宽度
}
```

### 波及文件

| 文件 | 改动 |
|---|---|
| `crates/app/src/tab_bar.rs` | `layout_tabs`：当 `tab_width < MIN_TAB_WIDTH` 时固定宽度，tab 总宽超出区域；新增 `scroll_tabs(delta)` 修改 `scroll_offset`；`hit_test` 考虑 `scroll_offset`；渲染顶点时所有 tab X 坐标偏移 `-scroll_offset` |
| `crates/app/src/app.rs` | `MouseScrollDelta` 事件在 tab bar 区域时分发给 `scroll_tabs()`；可选：hover 时显示左右溢出箭头 |

### 视觉效果

```
┌──────────────────────────────────────────┐
│ ← main.rs  Cargo.toml  lib.rs  app.r... →│  ← 溢出箭头
└──────────────────────────────────────────┘
```

- 鼠标滚轮在 tab bar 区域 → 水平滚动 tab
- 左右溢出时在 tab bar 两侧画半透明箭头
- 活动 tab 始终可见（自动滚动到视野内）

### 验收

- 打开 3 个 tab → 正常显示，无溢出箭头
- 打开 20 个 tab → 部分 tab 被剪切，显示 `→` 箭头
- 滚动到最右端 → `→` 消失，显示 `←` 箭头
- 切换到被剪切的 tab → 自动滚动使其可见
- 最小 tab 宽度不低于 60px

---

## 10.6 Pin 标签

### 目的

固定常用文件，使其始终在标签栏左侧，关闭其他文件时不受影响。

### 设计

```rust
// app.rs — App 新增字段
pub struct App {
    // ... 现有 ...
    pub pinned_tab_count: usize,   // 前 N 个 tab 被固定
}

impl App {
    pub fn is_tab_pinned(&self, index: usize) -> bool {
        index < self.pinned_tab_count
    }
    pub fn pin_tab(&mut self, index: usize) {
        // 将该 tab 移到 pinned 区末尾
    }
    pub fn unpin_tab(&mut self, index: usize) {
        // 将该 tab 移到 unpinned 区开头
    }
}
```

### 波及文件

| 文件 | 改动 |
|---|---|
| `crates/app/src/tab_bar.rs` | `TabEntry` 增加 `pinned: bool`；pinned tab 渲染时高亮锁图标或背景色区分；`layout_tabs` 时 pinned/unpinned tab 之间添加隔线 |
| `crates/app/src/app.rs` | 新增 `pinned_tab_count`；`close_tab` 跳过 pinned tab（需 unpin 后才能关）；快捷键 `Cmd+Shift+P` 切换 pin 状态 |

### 视觉效果

```
┌──────────────────────────────────────────┐
│ 📌 main.rs  📌 Cargo.toml │ lib.rs  app.rs │  ← Pin 图标 + 隔线
└──────────────────────────────────────────┘
```

- Pinned tab 前显示 📌 小图标
- Pinned tab 宽度比其他 tab 稍窄（因为内容少一个 close 按钮，用 pin 图标替代）
- Pinned 区域和 unpinned 区域之间有 1px 隔线

### 验收

- `Cmd+Shift+P` 切换 pin
- Pinned tab 不能用 `Cmd+W` 关闭（需先 unpin）
- 关闭 unpinned tab 后 pinned tab 不受影响
- 重启后 pinned 状态保留

---

## 10.7 Tab 右键上下文菜单

### 目的

右键点击 Tab 弹出操作菜单（关闭、关闭其他、关闭右侧、复制路径等）。

### 设计

此功能涉及弹出式菜单 UI，在 wgpu 管线下需要自绘。先做最小可用的版本：

```rust
// tab_bar.rs 新增
#[derive(Debug, Clone)]
pub struct TabContextMenu {
    pub visible: bool,
    pub tab_index: usize,
    pub position: [f32; 2],  // 弹出位置 (NDC)
    pub items: Vec<ContextMenuItem>,
}

pub struct ContextMenuItem {
    pub label: String,
    pub action: ContextMenuAction,
}

pub enum ContextMenuAction {
    Close,
    CloseOthers,
    CloseRight,
    CloseAll,
    CopyPath,
    TogglePin,
}
```

### 波及文件

| 文件 | 改动 |
|---|---|
| `crates/app/src/tab_bar.rs` | 新增 `TabContextMenu` 结构和渲染/布局/hit-test 逻辑 |
| `crates/app/src/app.rs` | 右键事件 → 显示菜单；点击菜单项 → 执行对应操作 |

### 视觉效果

```
┌─────────────────────────────┐
│ main.rs              [×]   │
├─────────────┐               │
│ Close        │  ← 右键菜单  │
│ Close Others │              │
│ Close Right  │              │
│──────────────│              │
│ Copy Path    │              │
│ Pin Tab      │              │
└─────────────┘───────────────┘
```

- 菜单项可点击
- 点击菜单外部 → 关闭菜单

### 验收

- 右键 tab → 弹出菜单
- 点击 "Close" → 关闭该 tab
- 点击 "Close Others" → 保留该 tab，关闭其余
- 点击外部区域 → 菜单消失

---

## 10.8 Preview Tab 模式

### 目的

从文件浏览器单击文件时，临时打开文件而不污染 tab 历史。
再次单击其他文件时复用同一个 preview tab。

### 设计

```rust
// app.rs — App 新增字段
pub struct App {
    // ...
    pub preview_tab_index: Option<usize>,  // 预览 tab 的索引
}
```

**行为**：
- 从文件树/查找器单击时 → 复用 preview_tab_index（如果有）
- 双击或在 tab 内编辑后 → preview 升级为普通 tab，清除 preview 标记
- 切换到其他 tab → preview tab 自动关闭

### 波及文件

| 文件 | 改动 |
|---|---|
| `crates/app/src/tab_bar.rs` | `TabEntry` 增加 `preview: bool`；preview tab 标题用斜体或不同颜色渲染 |
| `crates/app/src/app.rs` | 新增 `preview_tab_index`；`open_file` 增加 `preview: bool` 参数；编辑操作检测 → 自动升级 preview |

### 验收

- 单击文件 A → preview tab 显示 A（斜体）
- 再单击文件 B → preview tab 替换为 B（A 关闭）
- 在 preview tab 编辑 → 变为普通 tab（无斜体）
- 切换到另一个 tab → preview tab 自动关闭

---

## 跨阶段依赖

```
10.1 状态指示器 ──┐
                 ├──→ 10.4 边框（共用 TabEntry 字段）
10.2 消歧义 ─────┘
                        │
10.3 导航历史 ─────────┤
                        ├──→ 10.5 可滚动（布局算法升级）
10.6 Pin 标签 ─────────┘
                              │
10.7 右键菜单 ←────────────────┘
10.8 Preview Tab ←─────────────┘（独立，可并行）
```

## 实施优先级

| 优先级 | 阶段 | 收益 | 改动量 | 依赖 |
|---|---|---|---|---|
| **P0** | 10.1 状态指示器 | 即时可见的 UX 提升 | ~30 行 | 无 |
| **P0** | 10.2 消歧义 | 修复同名文件不可区分 | ~50 行 | 无 |
| **P1** | 10.4 位置感知边框 | 视觉融合，接近原生 IDE | ~80 行 | 10.1/10.2（共享字段） |
| **P1** | 10.6 Pin 标签 | 常用文件固定 | ~100 行 | 10.1/10.2 |
| **P2** | 10.3 导航历史 | 常用导航操作 | ~80 行 | 无 |
| **P2** | 10.5 可滚动 | 大量文件时的可用性 | ~120 行 | 10.1—10.4 的布局算法 |
| **P3** | 10.7 右键菜单 | 快捷操作入口 | ~200 行 | 无 |
| **P3** | 10.8 Preview Tab | 文件浏览流畅体验 | ~150 行 | 无（需文件树配合） |

## 注意事项

1. **不改 3+ 文件原则**：每个子阶段最多改动 `tab_bar.rs` + `app.rs` 两个文件。如需要第三个文件（如新建 `context_menu.rs`），需单独拆阶段。
2. **AGENTS.md 规则**：写代码之前先说清楚打算怎么做，等确认后再动手。
3. **编译验证**：每个子阶段完成后 `cargo check -p edit-plus-app` 必须通过。
4. **现有测试不退化**：每个子阶段完成后 `cargo test -p edit-plus-app --lib` 保持全部通过。
