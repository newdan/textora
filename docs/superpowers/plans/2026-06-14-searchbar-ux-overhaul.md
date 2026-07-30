# Searchbar UX Overhaul — 开发计划

## 现状摘要

| 维度 | 当前实现 | 问题 |
|------|----------|------|
| 视觉 | 背景色 + query 文本 + 匹配计数 + 光标条 | 无分隔线、无图标、无占位文字、无"无结果"状态 |
| 光标 | `query.len() * 8.0 * dpi` 估算 | CJK 字符宽度不对；不闪烁 |
| 交互 | 键盘输入/退格/回车/Escape | 无清空按钮、无导航按钮、无点击定位、无选中/复制/粘贴、Escape 直接关闭太粗暴 |
| 搜索辅助 | 无 | 无历史、无选项可视化（大小写/正则/全词）、无搜索统计 |

---

## 数据流总览（现有 + 改动点）

```
app.rs                          app_renderer.rs              ui_shell.rs                   search_bar.rs
  │                                  │                           │                              │
  │ SearchState {                    │                           │                              │
  │   query, matches,               │                           │                              │
  │   active_match_idx,             │                           │                              │
  │   panel_visible,                │       ① TextMeasure       │                              │
  │   cursor_byte_pos  ← NEW        │      cursor_x =            │                              │
  │   search_history ← NEW          │        measure(query)      │                              │
  │   blink_on ← NEW                │        measure(query_prefix) │                            │
  │ }                               │                           │                              │
  │                                  │ ② SearchBarSnapshot {    │                              │
  │                                  │      query, match_count,  │                              │
  │                                  │      current_match,       │                              │
  │                                  │      visible,             │                              │
  │                                  │      cursor_x, ← NEW      │  ③ 不重建 dock children    │
  │                                  │      blink_on, ← NEW      │    仅 set_input 更新状态    │
  │                                  │      no_results, ← NEW    │                              │
  │                                  │      can_nav_prev, ← NEW  │                              │
  │                                  │      can_nav_next, ← NEW  │                              │
  │                                  │ }                         │                              │
  │                                  │                           │                              │
  │                                  │                           │→ SearchBarWidget.paint()    │
  │                                  │                           │  → 分隔线、图标、占位文字    │
  │                                  │                           │  → 精确光标位置             │
  │                                  │                           │  → 按钮区域                 │
```

---

## Phase 1 — 视觉翻新（当天完成）

### 1.1 底部分隔线

在 paint 末尾追加一条 1px 横线，`rect.w` 宽度，颜色用 `theme.sidebar_border` 或新增 `theme.search_bar_border`。

```
ctx.list.fill(
    Rect::new(0.0, self.rect.h - 1.0, self.rect.w, 1.0),
    ctx.theme.search_bar_border,  // 新增 theme 字段
);
```

**改动文件：**
- `crates/ui/src/theme.rs` — 新增 `search_bar_border`，dark/light 各一套 + 自动 flip
- `crates/ui/src/widgets/search_bar.rs` — paint 末尾画线（query 空时也画）

### 1.2 占位文字（placeholder）

query 为空时在左侧区域画 `"Find..."` 半透明文字。

```
if self.snap.query.is_empty() {
    ctx.list.text(
        pad_left, baseline, font_size,
        placeholder_color,  // theme.search_bar_fg * 0.4
        "Find...",
    );
}
```

不需要新增 theme 字段，直接在当前 `search_bar_fg` 上乘 alpha 衰减。

**改动文件：** `crates/ui/src/widgets/search_bar.rs` paint

### 1.3 左侧搜索图标

用 Unicode 字符 `"🔍"` 或更稳妥的 `"/"` 显示在左侧 pad 区域（x ≈ 12px 处）。

如果用 emoji 渲染不可控，则用纯文本 `"/"` — 接近 VSCode 的 search icon 语义。

**改动文件：** `crates/ui/src/widgets/search_bar.rs` paint

### 1.4 "无结果"状态

当 `query 非空 && match_count == 0` 时：
- 右侧（匹配计数位置）显示 `"No results"` 用浅红/橙色
- 或整条 bar 背景微微泛红

新增 `theme.search_bar_no_results_fg`。

**改动文件：**
- `crates/ui/src/theme.rs` — 新增颜色
- `crates/ui/src/widgets/search_bar.rs` — 条件绘制

### 1.5 匹配计数样式优化

当前 `"3/15"` 改成 `"3 of 15"` 更可读，或者保留 `"3/15"` 但加文字标签。同时当 `match_count > 0` 时，当前匹配索引用亮色，分隔符和总数用暗色。

**改动文件：** `crates/ui/src/widgets/search_bar.rs` paint

---

## Phase 2 — 光标精确化 + 闪烁（当天完成）

### 2.1 用 TextMeasure 计算光标 X 坐标

**关键约束：** `PaintCtx` 不含 `TextMeasure`（paint 不应触发字体 shaping）。所以测量在 `app_renderer.rs` 进行。

**方案：** 在 `app_renderer.rs` 构建 `SearchBarSnapshot` 时，用已有的 `MeasureFromShaper` 测量 `query[..cursor_byte_pos]` 前缀宽度，填入 snapshot。

`SearchBarSnapshot` 新增字段：
```rust
pub cursor_x: f32,       // 光标相对于 pad_left 的偏移（已 dpi 缩放）
pub cursor_byte_pos: usize,   // 光标在 query 中的字节位
```

**流程：**
1. `SearchState` 新增 `cursor_byte_pos: usize` 字段
2. 每次 InsertChar / Backspace 时更新（append 到末尾，或 pop 最后一个字符 → 末尾）
3. app_renderer 用 `measure.measure(query[..cursor_byte_pos], font_size)` 得到精确宽度
4. 传入 `Snapshot.cursor_x`

**光标位置支持非末尾：** 先用末尾光标（简单），后续 Phase 4 再做点击定位。

**改动文件：**
- `crates/ui/src/widgets/search_bar.rs` — `SearchBarSnapshot` 新增 `cursor_x`
- `crates/app/src/search_state.rs` — 新增 `cursor_byte_pos`
- `crates/app/src/app_renderer.rs` — 测量 + 填入 snapshot
- `crates/app/src/app.rs` — InsertChar/Backspace 时更新 cursor_byte_pos

### 2.2 光标闪烁

在 `SearchBarSnapshot` 中新增 `blink_on: bool`。

**方案：** app 层维护一个帧计数器 / 计时器，每 ~500ms toggle blink_on。不引入定时器的话，用 `frames_rendered % blink_interval == 0` 近似（60fps → 每 30 帧 toggle 一次 ≈ 500ms）。

当 `blink_on == false` 时跳过光标绘制。

**改动文件：**
- `crates/ui/src/widgets/search_bar.rs` — snapshot 新增 `blink_on`，paint 条件跳过
- `crates/app/src/app.rs` 或 `app_renderer.rs` — 在帧循环中 toggle blink 相位

---

## Phase 3 — 交互升级（2-3 天）

### 3.1 清空按钮（×）

query 非空时在右侧（匹配计数右边或左边）显示 `"✕"` 字符按钮。

**实现方式（纯 widget 内）：**
- paint 时额外画一个 button 区域 Rect（记作字段）
- hit 时如果点在 button 区域内，返回新 action `SearchBarAction::ClearQuery`
- app 收到后 `dv.search_state.query.clear()` + cursor_byte_pos = 0

```
fn paint(&self, ctx: &mut PaintCtx) {
    // ... query text, cursor ...
    if !self.snap.query.is_empty() {
        let btn_rect = ...;
        ctx.list.text(btn_x, baseline, font_size, fg, "✕");
        // 保存 btn_rect 用于 hit
    }
}
```

**改动文件：**
- `crates/ui/src/widgets/search_bar.rs` — 新增 `clear_btn_rect` 字段，paint 绘制，hit 判断
- `crates/ui/src/widgets/search_bar.rs` — 新增 `SearchBarAction::ClearQuery`
- `crates/app/src/app.rs` — 处理 ClearQuery action

### 3.2 上/下导航按钮（▲▼）

匹配计数旁边显示 `"▲ ▼"` 或 Unicode 箭头（`"↑" "↓"`），作为两个独立可点击区域。

```
< 3/15 >  或  ▲ 3/15 ▼
```

**实现方式：** 两个 button rect，点击时产出 `Next` / `Prev` action（已有这些 action 变体，直接复用）。

**改动文件：**
- `crates/ui/src/widgets/search_bar.rs` — paint 新增按钮区域 + hit 处理
- （action 已有 Next/Prev，无需变更）

### 3.3 Escape 两段式

当前 Escape 直接关闭。改为：
1. query 非空 → 清空 query + 重置 cursor_byte_pos（不关闭面板）
2. query 已空 → 关闭面板

**改动文件：** `crates/app/src/app.rs`（约 3 行的逻辑变更）

### 3.4 点击定位光标（Phase 4 提前）

**前提：** 已有 `cursor_byte_pos` 和 `TextMeasure`。

MouseDown 在 searchbar 区域内时：
1. 计算 `click_x` 相对于 `pad_left` 的偏移
2. 用二分法 + TextMeasure 找到最接近的字符边界 → 设置 cursor_byte_pos

但 paint 阶段没有 TextMeasure。需要换策略：
- 在 `app_renderer.rs` 中，每次测量时也预计算"每个字符的 x 偏移"数组，传入 snapshot
- 或者让 widget 在 on_event 时也能访问 TextMeasure（需要改 EventCtx）

**较简单方案：** 在 `EventCtx` 中加入 `Option<&mut dyn TextMeasure>`，searchbar 在 on_event 时用 measure 反查。

但点击事件是在 `app_renderer` 层转换成 `SearchBarAction::MoveCursor(byte_pos)` 的… 不对，应该让 widget hit + on_event(MouseDown) 时处理。

**最终方案：** 给 `SearchBarWidget` 的 `on_event` 增加 `MouseDown` 分支。在 `EventCtx` 增加 `measure: Option<&mut dyn TextMeasure>`。MouseDown 时二分查找 click 对应的 byte position，产出 `SearchBarAction::MoveCursor(usize)`。

这需要 EventCtx 改造。评估工作量后改进方案：

**备选——不改造 EventCtx：** snapshot 携带字符宽度数组 `char_x_offsets: Vec<f32>`（每个字节位置的 x 偏移），widget 用二分查找定位。每帧测量一次，开销可接受。

**推荐备选方案**，改动最小。

**改动文件：**
- `crates/ui/src/widgets/search_bar.rs` — snapshot 新增 `char_x_offsets`，paint 缓存，on_event 处理 MouseDown
- `crates/app/src/app_renderer.rs` — 测量时遍历字符计算累积偏移
- `crates/app/src/app.rs` — 处理 `MoveCursor` action

---

## Phase 4 — 深度体验（后续迭代）

### 4.1 搜索历史下拉

- `SearchState` 新增 `history: Vec<String>`（最多 20 条）
- 每次提交搜索（Enter）时 push 到 history
- Snapshot 传入 `history: Vec<String>`
- query 为空时，如果有历史，在 searchbar 下方弹出一个简单的选项列表（复用或扩展 popup_menu 机制）
- 上下键/点击选择历史项

**改动文件：**
- `crates/app/src/search_state.rs` — 历史存储
- `crates/ui/src/widgets/search_bar.rs` — 历史下拉渲染
- `crates/app/src/ui_shell.rs` — 可能需要 overlay 支持

### 4.2 搜索选项可视化

- Snapshot 新增 `options: SearchOptions`
- searchbar 右侧或 dropdown 中显示 `[Aa]`（case sensitive）、`[.*]`（regex）、`[""]`（whole word）
- 可点击 toggle
- 产出新 action：`ToggleCaseSensitive`, `ToggleRegex`, `ToggleWholeWord`

**改动文件：**
- `crates/ui/src/widgets/search_bar.rs` — button + action
- `crates/app/src/search_state.rs` — toggle 方法
- `crates/app/src/app.rs` — action 处理

### 4.3 平滑出现/消失动画

- searchbar 高度从 0 → SEARCH_BAR_HEIGHT 过渡
- 或者 slide-down + fade-in
- 需要 animation tick 机制

### 4.4 文本选中 + 复制/粘贴

- 支持 Shift+←→ 选中 query 中文字
- Ctrl+C / Ctrl+V 操作
- 需要 selection range 在 SearchState 中维护
- 绘制选中高亮（反色矩形）

---

## Theme 变更汇总

```rust
// theme.rs 新增字段
pub struct Theme {
    // 现有
    pub search_bar_bg: [f32; 4],
    pub search_bar_fg: [f32; 4],
    // 新增
    pub search_bar_border: [f32; 4],           // 底部边框
    pub search_bar_no_results_fg: [f32; 4],     // "No results" 文字颜色
    pub search_bar_placeholder_fg: [f32; 4],    // 占位文字颜色（可选，fg*0.4 也行）
    pub search_bar_btn_hover_bg: [f32; 4],      // 按钮 hover 背景
}
```

## Snapshot 变更汇总

```rust
pub struct SearchBarSnapshot {
    // 现有
    pub query: String,
    pub match_count: usize,
    pub current_match: usize,      // 当前匹配索引（0-based），用于显示 "current_match+1/match_count"
    pub visible: bool,

    // Phase 2
    pub cursor_x: f32,              // 光标 X 偏移（相对 pad_left，已 dpi 缩放）
    pub blink_on: bool,             // 光标闪烁相位

    // Phase 3
    pub char_x_offsets: Vec<f32>,   // 每个字节位置前缀宽度（用于点击定位）
    pub no_results: bool,           // query 非空 && match_count == 0（冗余但方便 widget 判断）

    // Phase 4
    pub history: Vec<String>,       // 搜索历史（最多 20 条）
    pub options: SearchOptions,     // 当前搜索选项
}
```

## Action 变更汇总

```rust
pub enum SearchBarAction {
    // 现有
    InsertChar(char),
    Backspace,
    Next,
    Prev,
    Close,

    // Phase 3
    ClearQuery,                    // 清空 query 但不关闭面板
    MoveCursor(usize),             // 点击定位光标到指定字节位

    // Phase 4
    ToggleCaseSensitive,
    ToggleRegex,
    ToggleWholeWord,
    SelectHistory(usize),          // 选择历史项索引
}
```

## 执行顺序

```
Phase 1（视觉）：  1.1 → 1.2 → 1.3 → 1.4 → 1.5
Phase 2（光标）：  2.1 → 2.2
Phase 3（交互）：  3.3（最简单）→ 3.1 → 3.2 → 3.4（最复杂）
Phase 4（深度）：  后续迭代，不在此轮
```

依赖关系：
- 2.1（TextMeasure 光标）是 3.4（点击定位）的**前置**
- 其余无耦合，可并行
