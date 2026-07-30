# Pinned Tab 优化方案

## 问题分析

Pinned tab 存在两个互相关联的问题：

### 问题 1：`rect_px` 宽度虚高（layout.rs）

Pinned tab 的宽度计算使用了和普通 tab 相同的公式：

```
width_px = (pad_x + indicator_pad + text_w + close_area).max(min_tab_w).min(max_tab_w)
```

其中：
- `close_area = 20.0 * dpi` — 为关闭按钮预留的空间，但 pinned tab 根本不显示关闭按钮
- `min_tab_w = 40.0 * dpi` — 最小宽度，对 pinned tab 来说偏大

2x DPI 下，一个短标题的 pinned tab：
- 实际内容需要：`pad_x(20) + text(~16) + 缓冲(~17)` ≈ 53px
- 计算结果：`53px` → 被 `min_tab_w(80px)` 钳住 → **80px**
- 多出 **~27px** 空白宽度

### 问题 2：`draw_tab_bg` 填满整个 rect（state.rs）

渲染时，`draw_tab_bg` 对 pinned tab 调用 `dl.fill(entry.rect_px, bg)`，填充整个 80px 宽的矩形。但实际内容（文字 + pin indicator + separator）只占 ~53px，右侧 ~27px 的空白背景直接覆盖了非 pinned tab 区域。

同时，`pinned_clip` 和 `non-pinned_clip` 都使用 `entry.rect_px.right() + 4.0 * dpi` 作为边界，导致 clip 区域也包含了这段空白。

### 问题链路图

```
close_area(20px) + min_tab_w(80px)
  → rect_px 宽 80px，实际内容 ~53px
    → draw_tab_bg 填满 80px
      → 右侧 ~27px 空白背景盖住非 pinned 区域
        → pinned_clip 以 rect.right()+4px 为界，空白区域在 clip 内
          → non-pinned_clip 从 rect.right()+4px 开始，被盖住的部分不可见
```

---

## 修复方案

### Phase 1：收窄 pinned tab 宽度（layout.rs）

**文件**：`crates/ui/src/widgets/tab_bar/layout.rs`  
**函数**：`layout_tabs` — Phase 1 宽度计算

#### 改动 1：新增 pinned 专用常量

```rust
// 现有
let min_tab_w = 40.0 * ctx.dpi;
let max_tab_w = 310.0 * ctx.dpi;
let close_area = 20.0 * ctx.dpi;

// 新增
let pinned_min_tab_w = 30.0 * ctx.dpi;   // pinned 更紧凑
let pinned_max_tab_w = 160.0 * ctx.dpi;  // pinned 不需要太宽
let pinned_right_pad = 12.0 * ctx.dpi;   // 替代 close_area（无关闭按钮）
```

#### 改动 2：宽度计算分支

```rust
// 现有（所有 tab 统一）
let width_px = (pad_x + indicator_pad + text_w + close_area).max(min_tab_w).min(max_tab_w);

// 改为
let is_pinned = pinned_indices.contains(&i);
let (right_pad, effective_min, effective_max) = if is_pinned {
    (pinned_right_pad, pinned_min_tab_w, pinned_max_tab_w)
} else {
    (close_area, min_tab_w, max_tab_w)
};
let width_px = (pad_x + indicator_pad + text_w + right_pad).max(effective_min).min(effective_max);
```

#### 效果（2x DPI，短标题 "a"）

| 指标 | 改前 | 改后 |
|------|------|------|
| right_pad | 40px (close_area) | 24px (pinned_right_pad) |
| 计算宽度 | 76px | 60px |
| min_tab_w | 80px | 60px |
| 最终 width_px | **80px** | **60px** |
| rect 覆盖超出 | ~27px | ~7px |

---

### Phase 2：收紧渲染 clip 区域（state.rs）

**文件**：`crates/ui/src/widgets/tab_bar/state.rs`  
**函数**：`to_drawlist`

#### 改动 1：pinned_clip 紧贴 separator 末尾

```rust
// 现有
let pinned_right = layout.tabs[lp].rect_px.right() + 4.0 * dpi;

// 改为：separator 宽 2px，位于 rect.right()+1px，末尾在 rect.right()+3px
let pinned_right = layout.tabs[lp].rect_px.right() + 3.0 * dpi;
```

#### 改动 2：non-pinned_clip 同步调整

```rust
// 现有
.map(|lp| layout.tabs[lp].rect_px.right() + 4.0 * dpi)

// 改为
.map(|lp| layout.tabs[lp].rect_px.right() + 3.0 * dpi)
```

#### 改动 3：pinned_total_width 减去 trailing gap

```rust
// 现有
let pinned_total_width = pinned_width; // = Σ(width_px + gap)，含末尾多余 gap

// 改为
let pinned_total_width = if pinned_width > 0.0 { pinned_width - gap } else { 0.0 };
```

---

### Phase 3（可选）：美化 pin indicator（state.rs）

**文件**：`crates/ui/src/widgets/tab_bar/state.rs`  
**函数**：`draw_tab_content`

#### 改动：pin indicator 改为圆角胶囊形

```rust
// 现有：简单矩形条
let bar_w = 2.0 * dpi;
dl.fill(Rect::new(bar_x, bar_y, bar_w, bar_h), [0.4, 0.55, 0.8, 0.8]);

// 改为：pill shape（三段式：上圆角 + 中间 + 下圆角）
let pill_x = entry.rect_px.x + 3.0 * dpi;
let pill_w = 5.0 * dpi;
let pill_h = entry.rect_px.h * 0.45;
let r = 2.0 * dpi;
dl.fill(Rect::new(pill_x + r, pill_y, pill_w - 2.0 * r, r), pin_color);        // top cap
dl.fill(Rect::new(pill_x + r, pill_y + pill_h - r, pill_w - 2.0 * r, r), pin_color); // bottom cap
dl.fill(Rect::new(pill_x, pill_y + r, pill_w, pill_h - 2.0 * r), pin_color);   // body
```

#### 改动：dirty indicator 和文本起点对齐 pin_pad

```rust
// dirty indicator 偏移
let pin_offset = if entry.pinned { 10.0 * dpi } else { 0.0 }; // 原来是 6.0

// 文本起点
let pin_pad = if entry.pinned { 10.0 * dpi } else { 0.0 };
let x = entry.rect_px.x + pin_pad + base_pad + indicator_pad;
```

---

## 改动文件汇总

| 文件 | 改动项 | 风险 |
|------|--------|------|
| `layout.rs` | pinned 专用 right_pad / min / max | 低：仅影响 pinned 分支 |
| `state.rs` | clip 收紧 4px→3px | 低：separator 仍在 clip 内 |
| `state.rs` | pinned_total_width 减 trailing gap | 低：消除 2px 错位 |
| `state.rs` | pin indicator 美化 | 低：纯视觉 |

## 验证要点

1. 短标题 pinned tab（如 "a"）不再覆盖非 pinned 内容
2. 长标题 pinned tab（如 "very_long_name.rs"）正常截断
3. 多个 pinned tab 之间 separator 正确显示
4. pinned + dirty 状态下 indicator 和文本对齐
5. 滚动时 pinned 区域固定不动，非 pinned 正常滚动
6. hit test：pinned tab 点击仍正常，close 按钮不响应
