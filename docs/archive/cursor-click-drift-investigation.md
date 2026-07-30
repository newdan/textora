# 光标相关问题排查报告

## 概述

排查两个新问题：
1. 键盘向下箭头经常停住
2. 高亮效果没了

## 问题 1：向下箭头停住 + 问题 2：高亮消失 → 共享根因

### 真正的根因：`ensure_cursor_visible` 只看文档行，不看守可视行

**关键代码**：`crates/app/src/document_view/mod.rs:600-607`

```rust
pub fn ensure_cursor_visible(&mut self, line_height: f32) {
    let cursor_line = self.cursor_line();  // ← 文档行号
    let visible_range = self.display.viewport
        .visible_doc_range_from_anchor(..);  // ← 文档行范围

    if cursor_line >= visible_range.start && cursor_line < visible_range.end {
        return;  // ← 同文档行就跳过，不管在哪个可视行
    }
```

`visible_doc_range_from_anchor` 返回的是**文档行**范围（`viewport.rs:271`，用 `doc_line` 遍历）。对于长行软折行，光标在同一个文档行内的不同可视行之间移动时，范围始终是 `0..1`（单文档行），`ensure_cursor_visible` 永远不会触发滚动。

**同样的问题存在于 render_pipeline.rs 的 autoscroll 代码**（只比较 `cursor_doc_line` 和 `anchor.doc_line`）。

### 复现场景

```
文档：一行很长的文本，软折为 20 个可视行
视口：只显示 10 行

初始：光标在 VL 9（视口底部）
```

按键向下箭头后：

| 步骤 | 发生什么 | 用户看到 |
|------|---------|---------|
| 1 | `move_cursor_visual(1)` → `move_down_past_visible` → 光标移到 VL 10 的字节位置 | - |
| 2 | `ensure_cursor_visible`：同一文档行 → **不滚动** | 视口停在原地 |
| 3 | 渲染：`cursor_visual_line = 10`，但 advance_cache 只有 VL 0-9 | 光标在屏幕外 |
| 4 | 再次按下箭头：`target_vis = 11`，再次 `move_down_past_visible`，光标移到 VL 11 | 视口仍然不动 |
| ... | ... | **光标"消失"了** |
| N | 光标到达文档行末尾 → 移动到下一个文档行 → `ensure_cursor_visible` 终于滚动 | 视口突然跳一大段 |

**症状如实对应**：
- **"向下箭头停住"**：视口不跟随，用户以为光标不动（其实光标在屏幕外移动）
- **"高亮消失"**：`cursor_visual_line` 超出可见范围，`cursor_vertices` 画的竖线在屏幕外不可见

### 影响范围

这不是边界 case——任何有软折行且内容超过一屏的文件都会触发：
- 一行很长的日志 / JSON
- 一段没有换行的文字
- 任何开启了 word-wrap 且行长度超过视口宽度的文本

---

## 次级问题：`move_down_past_visible` 跳过空行

**代码位置**：`crates/app/src/cursor_motion.rs:247-258`

```rust
} else if last_doc_line + 1 < total_lines {
    let mut target_line = last_doc_line + 1;
    while target_line < total_lines
        && dv.line_byte_length(target_line).unwrap_or(0) == 0  // ← 跳过空行
    {
        target_line += 1;
    }
```

光标处于最后一根可见行时向下移动，如果下一行是空行会被跳过。
虽然 `advance_cache` 中空行有占位条目，但 `move_down_past_visible` 的逻辑忽略它们。
这个问题只在光标需要滚动出新视口时才触发（`target_vis >= advance_cache.len()`）。

---

## 修复建议

### 修复 1（主要）：`ensure_cursor_visible` 增加可视行感知

**方案 A**：在 `ensure_cursor_visible` 中比较 `cursor_visual_line` 和可见范围：

```rust
pub fn ensure_cursor_visible(&mut self, line_height: f32) {
    let cursor_line = self.cursor_line();
    let visible_range = self.display.viewport
        .visible_doc_range_from_anchor(&self.display.display_map, line_height);

    // 如果光标在可见的文档行范围之外，按原逻辑滚动
    if cursor_line < visible_range.start || cursor_line >= visible_range.end {
        // ... 现有滚动逻辑 ...
        return;
    }

    // 光标在可见文档行内，但可视行可能超出视口
    // 检查 cursor_visual_line 是否在视口范围内
    if let Some(cursor_vl) = self.cursor_render_state.cursor_visual_line {
        if cursor_vl >= self.display.viewport.visible_rows {
            // 光标在视口下方：向下滚动一个可视行
            let lh = line_height;
            self.display.viewport.scroll_pixels(lh, &self.display.display_map, lh);
            self.display.viewport.clamp_anchor(&self.display.display_map, lh);
            self.display.viewport.derive_scroll_top(&self.display.display_map, lh);
        }
    }
}
```

**方案 B**：在 `move_cursor_visual`（app.rs）中移动光标后、在 `ensure_cursor_visible` 之前，显式检查可视行是否需要滚动。

### 修复 2：`move_down_past_visible` 不跳过空行

移除 `while` 循环中的空行跳过，直接移动到下一个文档行。

### 修复 3（相关）：render_pipeline.rs autoscroll 也需要可视行感知

`shape_visible_lines` 中的 autoscroll 代码（`render_pipeline.rs` 开头附近）同样只比较文档行，需要和修复 1 类似的处理。

---

## 测试用例建议

1. **长行软折行向下箭头**：单行 200 字，word-wrap 为 20 个可视行，视口 10 行。验证按 10 次向下箭头后视口正确跟随
2. **软折行选择高亮**：长行上 shift+向下箭头创建选择，验证高亮区域正确显示且视口跟随
3. **空行导航**：文件末尾有空行，向下箭头不应跳过空行
4. **快速连续按键**：快速按 10 次向下箭头，光标不应丢失
