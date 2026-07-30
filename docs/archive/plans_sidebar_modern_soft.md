# Sidebar Modern & Soft 美化执行方案

> **实施进度审查（2026-06-13）：**
>
> | 阶段 | 状态 | 审查备注 |
> |---|---|---|
> | A. Theme + 修 hover bug | ✅ 完成 | `theme.rs` 已加 `sidebar_accent`/`sidebar_border`；`make_style_from_theme` 已挂回 `theme.sidebar_item_hover_bg`；hover/active 已改为透明叠加（dark 0.06 / 0.10）。 |
> | B. List pill + accent | ✅ 完成 | `widgets/list.rs::paint` 已实现内缩 pill（4px pad、4dp radius）+ active 左侧 2px accent line（圆角竖线）；`row_h_logical = 28.0`、`pad_x_logical = 12.0`；`ListStyle` 增 `item_accent` 字段。 |
> | C. Sidebar 框架按钮自绘 | ✅ 完成 | hamburger 三横线、+ 自绘、settings 仍保留 `\u{2699}`（按 C.4 接受）；右 1px border；`SidebarHoverButton` 状态枚举已加；hover 背景圆角浅底已生效；C.1 取消了独立 header_bg fill。 |
> | D. Resize cursor | ✅ 完成 | `widgets/sidebar.rs:354-358` 写入 `ctx.cursor_hint = ColResize`；`events.rs::dispatch_mouse` 返回 `cursor_hint`，`handle_cursor_moved` 读取并 push `SetCursor`。**注：用了 `ColResize` 而非方案里写的 `EwResize`（winit 文档建议 ColResize 用于水平拖拽改列宽——sidebar 右边缘性质等同列宽，选择正确）**。 |
> | E. HoverPeek 滑入动画 | ⚠️ 部分完成 | 见下文"E 阶段缺口"。 |
> | F. Pinned 滑动 | ⏸ 未启动（按方案推迟） | — |
>
> **E 阶段缺口（仍需补完）：**
> 1. **只做了 fade，没做宽度滑入。** 当前 `widgets/sidebar.rs:277-286` 用 `hover_peek_start.elapsed() / 0.15` 算 alpha，乘进 `ctx.global_alpha`。但 `set_rect` / `current_width` 仍按 `cfg.width` 整宽给值——HoverPeek 的 sidebar 一开始就占满目标宽度，只是 0→1 透明渐显。方案 §E.3 要求宽度也要随 progress 滑入（`bg_rect.w = cfg.width * progress`）。
> 2. **缺反向动画。** 进入 HoverPeek 有 fade in；离开（HoverPeek → Hidden）瞬间消失，无 fade out。`hover_peek_start` 只在 enter 时记录，没有 leave 端的对偶字段。
> 3. **没有持续 redraw 触发。** 动画期间没有显式 `request_redraw`——靠现有事件循环（鼠标移动、tick）刷新，肉眼可见时通常正常，但当鼠标静止时 fade 可能"卡帧"。`SidebarWidget::tick` 当前返回的 bool 含义是 visibility 变化而非"动画进行中"，app.rs 主循环没有为动画专门 redraw。
> 4. **`SidebarPersistent` 缺方案中规定的 `anim_progress` / `anim_target`。** 当前用 elapsed-based 计算（每次 paint 都从 `hover_peek_start` 重算），优点是无状态，缺点是无法做反向动画与中断恢复（鼠标进出 200ms 内来回切换会突变）。
> 5. **DrawList helper 已自动读 `self.offset`，没读 `global_alpha`。** widget 仍需手动 `color[3] *= alpha`——这是当前所有 sidebar/list 内 fill 调用的写法。可接受（方案 §E.1.a 路径），但要意识到所有未来新增 fill 的地方都要记得乘 alpha，否则该处不会跟着 fade（编辑器、tab_bar 都不需要 fade，所以只在 sidebar 子树里有这个心智成本）。
>
> **建议补完顺序：**
> - E.1（小）：把 `SidebarWidget::tick` 返回值语义改清（`bool animating`），app.rs 在动画期间 request_redraw。
> - E.2（中）：补 `hover_peek_leave_start: Option<Instant>` + 反向 fade。
> - E.3（中）：HoverPeek 状态下 `set_rect` 把 `bg_rect.w` 等子矩形按 progress 缩放。
> - E.4（小）：测试新增 — 反向 fade、tick→redraw 路径。
>
> **总验证清单进度：**
> - ✅ DPI 圆角（push_fill segments=8 默认未改，目测够用，未做正式 DPI=2.0 验证）
> - ✅ hamburger / new / settings hover 圆角（C 阶段验证过）
> - ⚠️ HoverPeek 进出动画（仅 enter 端 fade，无宽度滑入、无反向）
> - ✅ resize 光标
> - ✅ Pinned 切换瞬变
> - ⚠️ ESC 收起 HoverPeek 是否 "瞬变" — 由于无反向动画，目前实际就是瞬变，"满足"但不是按方案的设计满足
>
> ---

> **前置条件：** 本方案的所有阶段（除阶段 A）必须在 `docs/plans_ui_refactor_v2.md` 全部落地之后开始。
> v2 重构会重写 `widgets/list.rs`、`widgets/sidebar.rs`、`sidebar.rs::SidebarLayout` 三处 paint / 坐标算式，
> 美化阶段 B/C 触碰的就是同一段代码，先美化后重构等于改两遍。
>
> v2 已经为美化预留两个协议槽位：
> - `PaintCtx::global_alpha: f32`（默认 1.0）—— 用于阶段 E 的 HoverPeek 淡入淡出
> - `EventCtx::cursor_hint: Option<winit::window::CursorIcon>`（默认 None）—— 用于阶段 D 的 resize cursor
>
> 这两个字段在 v2 阶段 1 已经加好，但**没有接线**（helper 不读 alpha；events.rs 不读 hint）。
> 美化阶段会负责把它们真正接通。

## 1. 整体目标与风格界定

**风格定位：** Modern & Soft（现代柔和）

**核心特征：**
- 放弃大面积深色生硬色块，采用极简的留白和细微对比。
- 引入大量圆角（Rounded Corners）与微悬浮（Hover overlay）效果。
- 活动状态（Active）采用强调色细线条（Accent Bar）指示，而非整行颜色高亮。
- 用 `FillRect` 拼几何图标代替原生 Unicode 字符（`☰`、`+`、`⚙`）。

**例外：** 新建按钮作为 primary action，允许使用 `sidebar_accent` 透明填充作为常驻底色——这与"active 用细线条"的总纲不冲突，因为 primary button 不是 list item active 语义。

## 2. 涉及修改的文件模块

| 文件 | 阶段 | 改动性质 |
|---|---|---|
| `crates/ui/src/theme.rs` | A | 新增 2 个颜色字段、调整现有 hover/active 为透明叠加 |
| `crates/ui/src/widgets/sidebar.rs` | A | 修 `make_style_from_theme` 的 hover 错挂（旧 bug） |
| `crates/ui/src/widgets/list.rs` | B | 重写行 hover/active 几何（pill + accent line） |
| `crates/ui/src/sidebar.rs` | C | 重写 `state.paint`：去常驻底色、自绘几何图标、border |
| `crates/ui/src/widgets/sidebar.rs` | C/E | paint 接线 global_alpha；on_event 写 cursor_hint |
| `crates/app/src/events.rs` | D | 读 `EventCtx::cursor_hint`，转 `AppAction::SetCursor` |
| `crates/ui/src/core/paint.rs` | E | helper 读 `ctx.global_alpha` 把 alpha 乘进 color |
| `crates/app/src/paint_backend.rs` | E（可选） | 验证 `push_fill::segments` 在 DPI 2.0 下足够 |

## 3. 阶段切分（原子化、独立可提交）

每个阶段一个 commit，跑 `cargo test --workspace` 全绿才进下一阶段。

### 阶段 A：Theme 字段补齐 + 修旧 bug

**前置：** 无（与 v2 解耦，可在 v2 任何阶段并行；建议放在 v2 完成后再做以避免 merge 冲突）。

**目标：** 把"列表项 hover 不响应主题字段"的旧 bug 修掉，并加进美化所需的两个新字段。本阶段视觉效果几乎不变，纯数据 + 1 行接线。

**改动：**
1. `crates/ui/src/theme.rs` 在 `Theme` 结构体追加：
   ```rust
   pub sidebar_accent: [f32; 4],  // 强调色（active 左竖线、primary button 底色）
   pub sidebar_border: [f32; 4],  // 极细右边线，1px
   ```
   `dark()` / `light()` 各自填值；`gamma_correct()` 数组追加这两个字段。

2. `dark()` / `light()` 中：
   - `sidebar_item_hover_bg` 改为低 alpha 透明叠加（dark：`[1.0, 1.0, 1.0, 0.06]`；light：`[0.0, 0.0, 0.0, 0.06]`）。
   - `sidebar_item_active_bg` 同理改为透明（alpha ~0.10）；选中态视觉重心后续阶段交给 accent line。

3. `crates/ui/src/widgets/sidebar.rs::make_style_from_theme` 当前用 `theme.sidebar_item_active_bg` 派生 hover（alpha×0.5），**完全没用 `theme.sidebar_item_hover_bg`**——直接挂回去：
   ```rust
   item_hover_bg: theme.sidebar_item_hover_bg,
   ```

**测试同步：**
- `crates/ui/src/widgets/sidebar.rs::tests::test_theme()` 函数追加 `sidebar_accent` / `sidebar_border` 字段。
- `crates/ui/src/theme.rs::tests` 已有 `dark_and_light_have_same_scopes`，不受影响；`dark_and_light_have_different_backgrounds` 不受影响。

**验证：** `cargo test --workspace` 全绿。视觉上 hover 颜色会略变浅——可接受。

**Commit：** `feat(theme): 加 sidebar_accent/border 字段，修 list hover 颜色错挂`

---

### 阶段 B：ListWidget pill hover + accent line active

**前置：** v2 阶段 4a（list 已迁移到相对坐标）+ 阶段 A。

**目标：** 列表项视觉柔和化——内缩圆角 hover、active 用左侧 2px 竖线代替整行高亮。

**改动 `crates/ui/src/widgets/list.rs::paint`：**

1. **行高加大：** `ListStyle::row_h_logical` 默认值由 `24.0` 改为 `28.0`。`make_style_from_theme` 中同步。
2. **Pill hover/active：**
   - 不再调 `ctx.list.fill(row_rect, ...)`。
   - 改成内缩圆角矩形：
     ```rust
     let pill_pad_x = 4.0 * dpi;
     let pill_pad_y = 2.0 * dpi;
     let pill_radius = 4.0 * dpi;
     let pill = Rect::new(
         row_rect.x + pill_pad_x,
         row_rect.y + pill_pad_y,
         row_rect.w - pill_pad_x * 2.0,
         row_rect.h - pill_pad_y * 2.0,
     );
     ctx.list.fill_rounded_with_offset(pill, color, pill_radius, ctx.offset);
     ```
3. **Active 左侧竖线：**
   - 在 `pill` 之后追加：
     ```rust
     let bar_w = 2.0 * dpi;
     let bar_h = row_rect.h * 0.6;
     let bar_x = 2.0 * dpi;  // 局部 x，pill 起点 4px，accent 在 2px，二者并存
     let bar_y = row_rect.y + (row_rect.h - bar_h) * 0.5;
     ctx.list.fill_rounded_with_offset(
         Rect::new(bar_x, bar_y, bar_w, bar_h),
         ctx.theme.sidebar_accent,
         bar_w * 0.5, ctx.offset,
     );
     ```
   - 注：accent 与 pill 同时绘制时视觉上 pill 先底色、accent 再叠在最左——构图正确。
4. **左 padding 加大：** `pad_x_logical` 由 `8.0` 改 `12.0`，给 accent line 留呼吸空间（accent 占 2+2=4px，文字基线左移到 12px）。

**ListStyle 增字段：**
```rust
pub item_accent: [f32; 4],   // 选中态左侧竖线颜色
```
`make_style_from_theme` 用 `theme.sidebar_accent`。

**测试同步（必须更新断言数）：**

| 测试名 | 旧断言 | 新断言原因 |
|---|---|---|
| `paint_emits_bg_plus_text_per_visible_row` | `cmds.len() == 4`（bg+3text） | bg 透明跳过：`3` |
| `rows_overflowing_rect_are_truncated` | `cmds.len() == 3` | bg 透明：`2`（2 个 text） |
| `active_row_paints_active_bg` | `cmds.len() == 4` | active 现在是 pill+accent 两个 fill：`5` |
| `dot_indicator_emits_extra_fill` | `cmds.len() == 3` | bg 透明：`2`（text+dot） |
| `empty_items_paint_emits_only_bg` | `cmds.len() == 1` | bg 透明：`0`；测试改名为 `empty_items_paint_emits_nothing` |

如果 `style()` test fixture 用的是不透明 bg（`[0.1, 0.1, 0.1, 1.0]`），那以上变化不会触发——但 `transparent_bg_emits_no_bg_fill` 的逻辑需要扩展验证 hover/active pill 计数。

**验证：**
1. `cargo test -p ui list`
2. `cargo run -p app`：选中文件应在最左有一根强调色竖线，hover 时整行内缩圆角变淡。

**Commit：** `feat(list): pill hover + accent line active`

---

### 阶段 C：Sidebar 框架按钮自绘 + border

**前置：** v2 阶段 4b（sidebar 已迁移到相对坐标）+ 阶段 B。

**目标：** 去掉 hamburger / new / settings 按钮的常驻深色块，hover 时显示圆角浅底；用 `FillRect` 拼几何图标代替 `\u{2630}` / `\u{2699}` Unicode 字符；右侧 1px 边线。

**改动 `crates/ui/src/sidebar.rs::state::paint`：**

#### C.1 背景统一与右边框

```rust
ctx.list.fill_with_offset(layout.bg_rect, ctx.theme.sidebar_bg, ctx.offset);
// 取消 header 单独色块：移除 fill(header_rect, sidebar_header_bg)
// 右侧 1px border
let border_w = 1.0 * dpi;
ctx.list.fill_with_offset(
    Rect::new(layout.bg_rect.w - border_w, layout.bg_rect.y, border_w, layout.bg_rect.h),
    ctx.theme.sidebar_border, ctx.offset,
);
// Hidden 态不画 border：bg_rect 此时是 menu_btn 的小矩形，逻辑上跳过
```

#### C.2 Hamburger 自绘

去掉 `\u{2630}` 文本，用 3 条横线：

```rust
let icon_color = ctx.theme.sidebar_item_fg;
let line_w = 1.5 * dpi;
let line_len = 12.0 * dpi;
let cx = menu_btn_rect.x + menu_btn_rect.w * 0.5;
let cy = menu_btn_rect.y + menu_btn_rect.h * 0.5;
let gap = 4.0 * dpi;
for i in [-1.0, 0.0, 1.0] {
    let y = cy + i * gap - line_w * 0.5;
    ctx.list.fill_rounded_with_offset(
        Rect::new(cx - line_len * 0.5, y, line_len, line_w),
        icon_color, line_w * 0.5, ctx.offset,
    );
}
```

Hover 时（hovered_index == HamburgerHover 之类的状态——见 C.5 hover state 扩展）画一个 `4px` 圆角浅底覆盖 menu_btn_rect。

#### C.3 New button：去常驻底 + accent 透明填充 + 自绘 +

```rust
// 去掉原来的 fill(new_btn_rect, sidebar_button_bg)
// 改为 accent 透明
let primary_bg = {
    let mut c = ctx.theme.sidebar_accent;
    c[3] *= 0.18;  // 极淡
    c
};
ctx.list.fill_rounded_with_offset(layout.new_btn_rect, primary_bg, 6.0 * dpi, ctx.offset);
// + 字符自绘：横竖两个 1.5px 矩形
let plus_size = 10.0 * dpi;
let plus_w = 1.5 * dpi;
let pcx = layout.new_btn_rect.x + 12.0 * dpi + plus_size * 0.5;
let pcy = layout.new_btn_rect.y + layout.new_btn_rect.h * 0.5;
ctx.list.fill_with_offset(
    Rect::new(pcx - plus_size * 0.5, pcy - plus_w * 0.5, plus_size, plus_w),
    icon_color, ctx.offset,
);
ctx.list.fill_with_offset(
    Rect::new(pcx - plus_w * 0.5, pcy - plus_size * 0.5, plus_w, plus_size),
    icon_color, ctx.offset,
);
// 文字"新建"放在 + 右侧
ctx.list.text_with_offset(
    pcx + plus_size * 0.5 + 6.0 * dpi,
    pcy + font_size * 0.35,
    font_size, icon_color, "新建", ctx.offset,
);
```

#### C.4 Settings button：去常驻底 + 保留 ⚙ 文字（短期）

⚙ 自绘成本高（齿轮要圆心 + 八齿），本阶段**保留 `\u{2699}` 文本**，只去掉常驻 `sidebar_header_bg` 底色。后续如要彻底自绘再起新阶段。

```rust
// 去掉 fill(settings_btn_rect, sidebar_header_bg)
// hover 时画圆角浅底（C.5 接入 hovered button state）
ctx.list.text_with_offset(
    layout.settings_btn_rect.x + 12.0 * dpi,
    layout.settings_btn_rect.y + layout.settings_btn_rect.h * 0.65,
    font_size, icon_color,
    "\u{2699} 设置", ctx.offset,
);
```

#### C.5 Hover state 扩展

`SidebarState` 新增字段记录哪个按钮在 hover：

```rust
#[derive(Copy, Clone, Eq, PartialEq, Debug, Default)]
pub enum SidebarHoverButton {
    #[default]
    None,
    Hamburger,
    NewDoc,
    Settings,
}
pub struct SidebarState {
    // ... 现有字段
    pub(crate) hovered_button: SidebarHoverButton,
}
```

`SidebarWidget::on_event` 处理 `MouseMove` 时更新 `hovered_button`（与 list hover 互斥）；`SidebarPersistent` 跨帧保留这个字段。

paint 时按 hovered_button 决定哪个按钮画圆角 hover 底色。

**测试同步：**
- `widgets/sidebar.rs::tests::widget_paint_emits_background_and_header` 当前断言 `cmds.len() >= 8`——本阶段构图变更后命令数会变（去掉 header_bg fill、去掉 button bg fill、加 border、加 + 字符两个 fill、加 hamburger 三横线）。建议改成精确断言或检查关键 cmd 存在性（按颜色匹配 sidebar_accent / sidebar_border）。
- `crates/ui/src/sidebar.rs::tests::sidebar_hidden_offsets_zero` 等结构性测试不受影响。

**验证：**
1. `cargo test --workspace`
2. `cargo run -p app`：
   - hamburger / new / settings 默认无底色，hover 才出现圆角浅底
   - new 按钮常驻 accent 透明底色，比其他按钮"权重高"
   - sidebar 右侧有 1px 暗色细线（border）
   - Retina 屏（dpi=2.0）下圆角无锯齿、无重影

**Commit：** `feat(sidebar): 框架按钮自绘 + 去常驻底色 + 右侧 border`

---

### 阶段 D：Resize cursor（接通 EventCtx::cursor_hint）

**前置：** 阶段 C。也可以独立于 C，但放在一起视觉/交互闭环更自然。

**目标：** 鼠标靠近 sidebar 右侧边缘 resize 区时，光标变 `ColResize`。

**改动 `crates/ui/src/widgets/sidebar.rs::on_event`：**

`MouseMove` 分支末尾、设置 hover state 之后：

```rust
let band = 4.0 * ctx.dpi;
let edge = self.cfg.width;
if (*px - edge).abs() <= band || self.dragging {
    ctx.cursor_hint = Some(winit::window::CursorIcon::EwResize);
}
```

**改动 `crates/app/src/events.rs::handle_cursor_moved`：**

在 `dispatch_mouse(app, Event::MouseMove { px, py })` 之后、push 默认 `SetCursor` 之前：

```rust
let dpi = Settings::with(|s| s.dpi_scale);
let mut ctx = EventCtx { theme: &app.current_theme, dpi, cursor_hint: None };
let _ = app.ui_shell.dispatch(&Event::MouseMove { px, py }, &mut ctx);
if let Some(icon) = ctx.cursor_hint {
    actions.push(AppAction::SetCursor(icon));
    return actions;  // 跳过编辑器默认 Text
}
```

⚠️ 这里有个**架构问题**：当前 `dispatch_mouse` 内部自己构造 `EventCtx`，外部读不到 `cursor_hint`。改法：把 `EventCtx` 在 `handle_cursor_moved` 内构造、传给 `dispatch_mouse`，dispatch 完成后回读 `ctx.cursor_hint`。这要求 `dispatch_mouse` 函数签名加一个 `&mut EventCtx` 参数。

签名变化：
```rust
fn dispatch_mouse(app: &mut App, ev: Event, ctx: &mut EventCtx) -> (Vec<AppAction>, bool)
```

调用方 `handle_cursor_moved` / `handle_mouse_input_left` / `handle_mouse_input_right` / `handle_scroll` 各自构造 ctx 并传入。

**测试新增：**
```rust
// crates/app/src/events.rs::tests
#[test]
fn cursor_moves_to_sidebar_right_edge_emits_col_resize() {
    // 构造一个 sidebar pinned width=220 的 app；鼠标 px=219, py=200
    // dispatch 后 actions 应包含 AppAction::SetCursor(CursorIcon::EwResize)
}
```

**验证：**
1. `cargo test --workspace`
2. `cargo run -p app`：sidebar pinned 模式下，鼠标移到 sidebar 右边缘 ±4px 内，光标变 ↔；移开恢复 Text。

**Commit：** `feat(sidebar): resize 区域 cursor=EwResize（接通 cursor_hint）`

---

### 阶段 E：HoverPeek 滑入动画 + fade（接通 PaintCtx::global_alpha）

**前置：** 阶段 C，建议也在 D 之后。

**重要决策（已接受）：**
- 仅对 `HoverPeek` 状态做动画，**`Pinned` 切换继续瞬变**（避免动画期间编辑器视口重新布局）。
- HoverPeek 的 `editor_left_offset` 恒为 0，编辑器布局不受动画影响。

**目标：** sidebar 从 Hidden → HoverPeek 时宽度 0→cfg.width 滑入 + 透明度 0→1 fade；HoverPeek → Hidden 时反向。Pinned 切换不动画。

#### E.1 接通 helper 读 global_alpha

`crates/ui/src/core/paint.rs::DrawList`：

```rust
pub fn fill_with_offset(&mut self, rect: Rect, color: [f32; 4], offset: (f32, f32)) {
    // 注意：alpha 由调用方提供给具体 helper —— 我们不在 DrawList 内拿到
    // PaintCtx，需要 helper 接收 alpha 参数或扩展签名。
    // 实际实现见下方决议。
}
```

**实现路径决议：**
有两种实现方式：

| 方案 | 优劣 |
|---|---|
| **E.1.a** helper 签名扩展，多一个 `alpha: f32` 参数（默认调用点用 `ctx.global_alpha`） | 单点改造但调用点改动多（数十处） |
| **E.1.b** `PaintCtx` 增加方法 `pub fn fill(&mut self, ...)`，把 list / offset / alpha 封装；DrawCmd 仍只存最终颜色 | 调用点最干净（`ctx.fill_rect(r, color)`），但要给 PaintCtx 加大量委托方法 |

**推荐 E.1.b**，但工程量较大，本阶段实施时先用 E.1.a：所有 `_with_offset` helper 增加 alpha 参数，widget 层显式传 `ctx.global_alpha`。

```rust
pub fn fill_with_offset(&mut self, rect: Rect, color: [f32; 4],
    offset: (f32, f32), alpha: f32)
{
    let mut c = color;
    c[3] *= alpha;
    self.cmds.push(DrawCmd::FillRect {
        rect: Rect::new(rect.x + offset.0, rect.y + offset.1, rect.w, rect.h),
        color: c, radius: 0.0,
    });
}
// fill_rounded_with_offset / text_with_offset / clip_with_offset 同理
```

**Widget 调用点全量替换**（grep `_with_offset(`）：每处末尾追加 `ctx.global_alpha`。

⚠️ 这一步工作量等同于 v2 阶段 1 step 5 的字面量扫描——一次性扫完全 workspace。

#### E.2 SidebarPersistent 增动画进度

```rust
pub struct SidebarPersistent {
    // ... 现有字段
    pub anim_progress: f32,    // 0.0 = 完全隐藏，1.0 = 完全展开
    pub anim_target: f32,      // 0.0 或 1.0
    pub anim_last_tick: Option<Instant>,
}
```

`tick(now)` 每帧更新：

```rust
pub fn tick(&mut self, now: Instant) -> bool {
    // 现有 hover_enter/leave 逻辑保留，触发 visibility 变化时同时设 anim_target
    // 例如 visibility=HoverPeek 时 anim_target=1.0；visibility=Hidden 时 anim_target=0.0
    // Pinned 不动画：直接 anim_progress = anim_target = 1.0

    let dt = self.anim_last_tick.map(|t| now.duration_since(t).as_secs_f32()).unwrap_or(0.0);
    self.anim_last_tick = Some(now);
    if (self.anim_progress - self.anim_target).abs() < 0.001 {
        self.anim_progress = self.anim_target;
        return false;
    }
    // critically-damped spring 或简单 ease：t = 1 - exp(-dt / tau)
    let tau = 0.10;  // 100ms 时间常数
    let alpha = 1.0 - (-dt / tau).exp();
    self.anim_progress += (self.anim_target - self.anim_progress) * alpha;
    true   // 表示动画进行中，需要 redraw
}
```

ESC 强制收起：跳过动画，直接 `anim_progress = 0.0; anim_target = 0.0`。

#### E.3 SidebarWidget 应用 progress

`set_rect`：HoverPeek 状态下，layout 的 `bg_rect.w = cfg.width * progress`（宽度滑入）；list_clip / new_btn_rect 等子矩形按这个宽度等比例缩放。

`paint`：进入前：
```rust
let saved = ctx.global_alpha;
if self.state.visibility() == Visibility::HoverPeek {
    ctx.global_alpha = saved * self.state.persistent().anim_progress;
}
self.state.paint(ctx, ...);
self.list.paint(ctx);
ctx.global_alpha = saved;
```

#### E.4 触发 redraw

`SidebarWidget::tick(now)` 返回 `bool`，`true` 表示动画进行中。`crates/app/src/app.rs` 的主循环（已有 tick 路径）：

```rust
if app.ui_shell.sidebar_widget_mut().tick(Instant::now()) {
    self.window.as_ref().map(|w| w.request_redraw());
}
```

#### E.5 测试

```rust
#[test]
fn hover_peek_animates_progress_over_time() {
    let mut p = SidebarPersistent::new(&cfg);
    p.visibility = Visibility::HoverPeek;
    p.anim_target = 1.0;
    let t0 = Instant::now();
    let _ = p.tick(t0);  // 第一帧只设 last_tick
    let t1 = t0 + Duration::from_millis(50);
    let animating = p.tick(t1);
    assert!(animating);
    assert!(p.anim_progress > 0.3 && p.anim_progress < 0.7,
        "50ms 应推进到 ~0.4（tau=100ms）");
}

#[test]
fn pinned_skips_animation() {
    let mut p = SidebarPersistent::new(&cfg);
    p.visibility = Visibility::Pinned;
    p.anim_progress = 0.0; p.anim_target = 0.0;  // 异常初值
    p.tick(Instant::now());
    // pinned 直接置 1.0，无动画过程
    assert_eq!(p.anim_progress, 1.0);
}

#[test]
fn esc_cancel_skips_animation() {
    // hover_peek 中按 esc，anim_progress 应直接归 0
}
```

**验证：**
1. `cargo test --workspace`
2. `cargo run -p app`：
   - 鼠标贴左边缘等 150ms：sidebar 平滑滑入（约 200-300ms 完成）
   - 鼠标离开 sidebar 区域 300ms 后：平滑滑出
   - 切 pin（Cmd-Shift-P）：瞬变，无动画
   - 按 ESC：瞬变收起，无动画

**Commit：** `feat(sidebar): HoverPeek 滑入动画 + global_alpha fade`

---

### 阶段 F（可选）：Pinned 切换滑动

**前置：** E。

**风险高，独立评估。** Pinned 状态的宽度变化会改变 `editor_left_offset`，编辑器视口需要每帧重新布局——advance_cache 是否要重算？滚动是否需要补偿？

**建议：先不做。** 用户能接受 Pinned 切换瞬变（与 VS Code、Zed 行为一致）。如果未来确实要做，先评估编辑器布局重算成本。

---

## 4. 总验证清单

阶段 E 完成后跑一遍：

- [ ] `cargo test --workspace` 全绿
- [ ] DPI=1.0 / 1.5 / 2.0 下圆角平滑（验证 `paint_backend.rs::push_fill::segments=8` 是否够；不够则按 dpi 动态调）
- [ ] 1000 行 tabs 滚动 hover 时帧时间稳定（粗略目测无掉帧）
- [ ] hamburger / new / settings hover 圆角无毛刺
- [ ] HoverPeek 进出动画无闪烁
- [ ] resize 光标在 sidebar 右边缘正确切换
- [ ] Pinned 切换瞬变（无动画痕迹）
- [ ] ESC 收起 HoverPeek 瞬变（无动画拖沓）

## 5. 工作量估算

| 阶段 | 估时 | 风险 |
|---|---|---|
| A. Theme + 修 hover bug | 0.5h | 低 |
| B. List pill + accent | 1.5h | 中（测试断言数大量改） |
| C. Sidebar 框架按钮自绘 | 3h | 中（hover state 扩展） |
| D. Resize cursor | 1h | 低 |
| E. HoverPeek 动画 | 4h | 高（global_alpha 接通牵动 widget 全量调用点） |
| F. Pinned 滑动 | 8h+ | 极高（编辑器布局重算） |

**合计 A-E：约 10 小时。** 建议拆 5 个独立 PR / commit，逐个落地。
