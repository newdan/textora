# Sidebar 体验问题修订方案 v2

> 基于实际试用反馈整理。**不动手实施**，先对齐设计。
>
> 与之前 `plans_sidebar_modern_soft.md` 的关系：本方案是该方案落地后的二次打磨；
> 它们触碰相同的代码区，但具体修复都集中在小范围、可独立提交的颗粒度。

## 0. 问题与根因速览

| # | 问题 | 根因（代码定位） |
|---|---|---|
| 1 | 超长文件名溢出 | `widgets/list.rs::paint` 直接 `ctx.list.text(label)`，从未做截断；`tab_bar/text.rs::truncate_title_by_width` 已有可用工具 |
| 2 | hover 不即时重绘，要等下一次鼠标移动 | `events.rs::handle_cursor_moved` 只在"overlay 存在但未消费"时 push `RequestRedraw`（181 行）；sidebar `Hovered` 路径走 `consumed` 直接 return，没有 redraw |
| 3 | 圆角丑、不平滑 | (a) sidebar 圆角值偏大（`pill_radius = 4dp`、按钮 `6dp`）；(b) `paint_backend.rs::push_fill::segments = 8` 固定 8 段，DPI≥2 时小半径下肉眼可见折线；popup_menu 用 `8dp` 也是 8 段，但因为半径绝对值大、相对误差小看起来还行 |
| 4 | sidebar 缺浅灰背景 | 当前 `theme.rs` `sidebar_bg = [0.145, 0.145, 0.149, 1.0]`（dark）= 几乎和 `background = [0.157, 0.173, 0.200]` 等亮度，对比度不足；light 主题更糟（`[0.95]` 与 `[0.98]` 差 0.03 看不出层次） |
| 5 | 文件区缺标题 | `sidebar.rs::compute_layout` 直接从 `new_btn_rect` 跳到 `list_clip`，无中间标题块；layout & paint 都缺 |
| 6 | 设置按钮无 hover 效果 | `sidebar.rs::paint` 第 749-754 行已有 hover bg 绘制；但 `state::on_mouse_move` 的 `hovered_button` 在 HoverPeek/Pinned "鼠标在 sidebar 内" 分支会被赋值；问题不是缺代码，而是**问题 #2 的副作用**——hover 状态变了但没触发重绘，肉眼看不到变化。需要修 #2 才能验证 #6 |

## 1. 修改清单（按文件 / 阶段拆分）

### Phase P1 — Hover 立即重绘（修问题 #2、解锁 #6 的可见性）

**File:** `crates/app/src/events.rs`

**根因深挖：**
当前 `handle_cursor_moved` 第 99-113 行：
```rust
let (widget_actions, consumed, cursor_hint) = dispatch_mouse(...);
actions.extend(widget_actions);
if let Some(icon) = cursor_hint { actions.push(SetCursor); return; }
if consumed { return actions; }   // ← sidebar hover 走这里直接 return
```
sidebar widget 在 `MouseMove` 处理结束时返回 `WidgetAction::Sidebar(SidebarAction::Hovered)`，被 `dispatch_mouse` 标为 `consumed=true`。`Hovered` 翻译成空 actions（`events.rs:404-407`）。所以 hover 状态机改了 `hovered_button` 字段，但当前帧没 `RequestRedraw`，画面要等下一帧（下次鼠标移动或 60Hz tick）才更新。

**修法：**
`SidebarAction::Hovered` 翻译时 push `AppAction::RequestRedraw`：

```rust
S::Hovered => {
    // Hover 状态可能改变了 hovered_button 或 hover_peek_start，需要重绘
    actions.push(AppAction::RequestRedraw);
}
```

**风险：** 鼠标在 sidebar 内每次 MouseMove 都触发 redraw——但事件本身已经按帧节流（winit 的 `CursorMoved` 实际频率约等于鼠标 polling rate ≤ 屏幕刷新率），等同于"sidebar 内鼠标移动期间持续 60fps 重绘"。可接受。

**不需要新测试**——视觉行为，单元测试覆盖不到。手测确认 hover 即时显示即可。

---

### Phase P2 — 圆角平滑度修复（修问题 #3）

**两路并修：**

#### P2.1 `paint_backend.rs::push_fill` segments 自适应

```rust
// 当前
let segments = 8;

// 改为
let segments = ((radius * 1.5).ceil() as usize).clamp(6, 16);
```

数值依据：
- `radius = 4` (sidebar pill) → 6 段，**仍偏少**——但配合 P2.2 把半径调小后会进一步看不出
- `radius = 8` (popup menu) → 12 段，曲线肉眼平滑
- `radius = 16` 极少出现，封顶 16

⚠️ 注意 `push_fill` 把 radius 作为像素值，未乘 dpi。所有调用处已经做了 `* ctx.dpi`，所以这里直接读到的是物理像素值。

#### P2.2 sidebar 圆角值收紧 + 改 1px 描边风格

用户需求 "改成 1px 线宽" — 当前没有描边能力，按 §0 审查里 #4 笔记，DrawCmd 不支持 stroke。可行的路径有两条：

| 路径 | 描述 | 工程量 |
|---|---|---|
| **P2.2.a** 减小圆角值 | `pill_radius` 4→2dp，`new_btn_radius` 6→3dp；保持纯填充 | 极小 |
| **P2.2.b** 给 DrawCmd 加 `StrokeRect` | 真正的描边路径；需扩 `DrawCmd` 枚举、`paint_backend.rs::push_stroke`、`DrawList::stroke_rounded` | 中（~50 行） |

**推荐 P2.2.a**：

理由——用户说"改成 1px 线宽"，但 sidebar 实际的 hover/active 视觉是**整块底色 pill**（不是边框）。把 1px 线宽理解成 stroke 不符合现状美学；他真正在意的是"圆角不要那么夸张、不要那么糊"。同时可以**新增右边 border 那种"窄长矩形伪 1px 线条"已经在用**（`sidebar.rs:663-668`）。

具体改：
- `widgets/list.rs::paint`：`pill_radius = 4.0 * dpi` → `pill_radius = 2.0 * dpi`
- `sidebar.rs::paint` hamburger hover bg：`4.0 * dpi` → `3.0 * dpi`
- new button：`6.0 * dpi` → `4.0 * dpi`
- settings hover bg：`4.0 * dpi` → `3.0 * dpi`
- accent line 圆角 `bar_w * 0.5` 保持（直径=2px 端帽自然圆）

如果用户**确实想要边框样式**而不是填充：补一条决策问询，再走 P2.2.b。本方案默认 P2.2.a。

---

### Phase P3 — Sidebar 浅灰背景（修问题 #4）

**File:** `crates/ui/src/theme.rs`

**当前值：**

| 主题 | sidebar_bg | window background |
|---|---|---|
| dark | `[0.145, 0.145, 0.149]` | `[0.157, 0.173, 0.200]` |
| light | `[0.95, 0.95, 0.95]` | `[0.98, 0.98, 0.98]` |

dark 主题 sidebar 比 background **更暗**且色相偏冷不一致；light 主题差值仅 0.03。

**目标：让 sidebar 比编辑区"略浅"一档，呈现微微抬起的层次感（VS Code、Zed 都是这种处理）。**

**改：**

```rust
// dark
sidebar_bg: [0.180, 0.195, 0.222, 1.0],   // 比 background 略浅，色相对齐
sidebar_border: [0.090, 0.105, 0.130, 1.0], // 比 sidebar_bg 略深的 1px 分割

// light
sidebar_bg: [0.965, 0.965, 0.965, 1.0],   // 比 background 略深
sidebar_border: [0.870, 0.870, 0.875, 1.0],
```

⚠️ 这违背"修问题 #4 时不动 dark 颜色"的直觉——但 dark 主题 sidebar 现在比 background 暗看起来"压下去"，与 #4 诉求"应该有浅灰背景"相反。需要让 sidebar 比 background **浅**。

**测试同步：** `theme.rs::tests::dark_and_light_have_different_backgrounds` 不影响。无新测试。

---

### Phase P4 — "文件" 区域标题（修问题 #5）

**Files:**
- `crates/ui/src/sidebar.rs`（layout & paint）
- `widgets/sidebar.rs`（list_clip 起点上移）

**Layout 改动：**

`SidebarLayout` 新增字段：

```rust
pub struct SidebarLayout {
    // ... 现有字段
    pub files_header_rect: Rect,   // "文件" 标题矩形
}
```

`compute_layout` 在 `new_btn_rect` 之后、`list_clip` 之前插入：

```rust
let files_header_h = 24.0 * dpi;
let files_header_y = new_y + new_h + pad * 2.0;   // 与 new 按钮间距 = 2*pad = 12dp
let files_header_rect = Rect::new(12.0 * dpi, files_header_y, w - 24.0 * dpi, files_header_h);

let list_top_px = files_header_y + files_header_h + pad * 0.5;  // 标题与列表小间距
```

**Paint 改动（`sidebar.rs::paint`）：**

在 New button 之后、Settings button 之前加：

```rust
// 4.5) Files section header
{
    let font_size = 11.0 * ctx.dpi;
    let baseline = layout.files_header_rect.y + layout.files_header_rect.h * 0.5 + font_size * 0.35;
    let mut fg = ctx.theme.sidebar_item_fg;
    fg[3] *= 0.5 * alpha;   // 半透明，作为 caption 弱化
    ctx.list.text(
        layout.files_header_rect.x,
        baseline,
        font_size, fg,
        "\u{6587}\u{4ef6}",  // "文件"
    );
}
```

**布局影响：**
- list_clip y 起点抬高 ~30dp，长文件列表可见行数 -1 行；可接受。
- HoverPeek / Pinned 共享同一 layout，标题在两种状态下都会显示。
- Hidden 态早 return（`paint:670-691`），不画标题。

**测试同步：**
- `crates/ui/src/widgets/sidebar.rs::tests::widget_paint_emits_background_and_header` 命令计数会再变（多 1 个 text）。改 `>= N` 断言。

---

### Phase P5 — 长文件名截断（修问题 #1）

**File:** `crates/ui/src/widgets/list.rs`

**集成 tab_bar 已有工具：**

`tab_bar/text.rs::truncate_title_by_width` 当前是 `pub(crate)`，需要：

1. 提到 `crates/ui/src/lib.rs` 或新建 `crates/ui/src/text_util.rs`，对 list 也可见。
   - 推荐：把 `tab_bar/text.rs` 中的 `truncate_title_by_width` / `char_width` / `estimate_text_width_px` 改为 `pub(crate)` → `pub`，并在 `lib.rs::pub use crate::tab_bar::text::*;` 或更干净地：抽到 `crate::core::text_util` 模块。
   - 工程量小：一次模块迁移，tab_bar 自身保留 `pub(crate) use`。

2. `list.rs::paint` 在 emit text 前调用：

```rust
let label_max_w = row_rect.w - pad_x * 2.0 - dot_extra_w;
// dot_extra_w = if has dot indicator { dot_r * 2.0 + 4dp } else { 0.0 }

let label = truncate_title_by_width(&item.label, label_max_w, font_size);
ctx.list.text(row_rect.x + pad_x, baseline, font_size, fg, &label);
```

3. ⚠️ `truncate_title_by_width` 用的是 `estimate_text_width_px`（ASCII 0.5em / 非 ASCII 1em 的硬编码估算），文件名常含 CJK + ASCII 混合，估算偏保守，不会越界。可以接受。

**测试新增：**

```rust
#[test]
fn long_label_is_truncated_with_ellipsis() {
    let theme = Theme::dark();
    let mut m = NoopMeasure;
    let mut layout = layout_ctx(&theme, &mut m);
    let mut w = VerticalListWidget::new(style());
    w.set_items(vec![item("very_long_filename_that_definitely_exceeds_row_width.rs")]);
    w.set_rect(Rect::new(0.0, 0.0, 120.0, 100.0), &mut layout);

    let mut list = DrawList::new();
    let mut paint = PaintCtx { list: &mut list, theme: &theme, dpi: 1.0,
        offset: (0.0, 0.0), global_alpha: 1.0 };
    w.paint(&mut paint);

    let text_cmd = list.cmds.iter().find_map(|c| match c {
        DrawCmd::Text { content, .. } => Some(content),
        _ => None,
    }).unwrap();
    assert!(text_cmd.contains('…'), "Expected ellipsis in truncated label, got: {text_cmd}");
    assert!(text_cmd.len() < "very_long_filename_that_definitely_exceeds_row_width.rs".len());
}
```

---

### Phase P6 — Settings hover 验证（修问题 #6）

**无代码改动。** P1 落地后，settings 的 hover bg fill（`sidebar.rs:751-754`）会即时显示。

**手测：** sidebar pin 状态下，鼠标缓慢移到 settings 按钮上方静止——应立即看到圆角浅底；移开立即消失。

如果 P1 完成后 settings hover **仍然不显示**，需要进一步排查 `state::on_mouse_move` 中 `hovered_button` 的赋值条件——但代码扫读没有发现问题，相信 P1 就够。

---

## 2. 阶段切分与提交粒度

| Phase | 文件数 | 难度 | 单独提交 |
|---|---|---|---|
| P1 redraw | 1 | 低 | ✅ |
| P2.1 segments 自适应 | 1 | 低 | ✅ |
| P2.2.a 圆角值收紧 | 2 | 低 | ✅ |
| P3 主题色调整 | 1 | 低 | ✅ |
| P4 文件标题 | 2 | 中 | ✅ |
| P5 长文件名截断 | 2-3 | 中（含模块迁移） | ✅ |
| P6 settings hover | 0 | 0 | 不提交，手测验证 |

**推荐顺序：** P1 → P3 → P2.2.a → P2.1 → P4 → P5 → P6 验收

理由：先修最高频可感知的问题（hover 即时反馈、整体背景层次），其次小颗粒视觉打磨（圆角），最后较重的功能补充（标题块、截断）。

## 3. 决策点（动手前确认）

1. **P2.2 用填充还是描边？** 
   -  给 `DrawCmd` 加 `StrokeRect`，sidebar hover/active 改为细线条边框（"1px 线宽"原意可能在此），参考菜单的实现

2. **P3 dark 主题 sidebar 比 background 浅 vs 深？**
   - 白色主题时， sidebar 应该比 background 灰
   - 现状 dark 主题 改成略浅（VS Code/Zed 风格） 

3. **P4 标题文字。** "文件" 

4. **P5 模块迁移：** `tab_bar::text` 抽到 `core::text_util`  
 
