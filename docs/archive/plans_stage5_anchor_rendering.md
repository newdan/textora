# 阶段 5：Anchor-based 渲染 — 技术方案

## 目标

彻底消除 `scroll_top` 依赖 `display_map` 累加和的不稳定性。视觉顶端永远精确锁定 `anchor.doc_line`，与远处 placeholder 的 VL 估算误差完全解耦。

## 核心思路

```
当前：scroll_top (display_row) → map_display_to_doc → 找到可见 doc_line → 渲染
阶段5：anchor.doc_line → 直接作为可见起点 → 渲染 → scroll_top 降级为派生量
```

## 涉及模块

| 模块 | 当前行为 | 目标行为 |
|------|---------|---------|
| `Viewport` | `scroll_top` 是 SOT | `anchor.doc_line` 是 SOT，`scroll_top` 纯派生 |
| `viewport::visible_doc_line_range` | 从 `scroll_top` 计算 | 从 `anchor.doc_line` 计算 |
| 渲染管线 | shape from `scroll_top.floor()` | shape from `anchor.doc_line` 向下遍历 |
| 滚动条 thumb | 基于 `scroll_top / total_rows` | 基于 `anchor.doc_line / total_doc_lines` |
| 滚动事件 | 修改 `scroll_top` + sync anchor | 直接修改 anchor（步进 doc_line） |
| page up/down | `scroll_top ± visible_rows` | anchor 步进 N 个 doc_line |
| cursor 跟随 | `scroll_to_row(cursor_display_row)` | `scroll_to_doc_line(cursor_doc_line)` |
| drain_reshape | `restore_scroll_from_anchor` | 不再需要——anchor 就是 SOT |
| `sync_anchor_from_scroll` | 滚动后 sync | 不再需要——anchor 直接由滚动事件设置 |
| `restore_scroll_from_anchor` | 从 anchor 算 scroll_top | 反向——从渲染结果写回 scroll_top |
| `clamp_scroll_top` | clamp display_row | clamp doc_line |
| 滚动条 drag | 设置 scroll_top | 设置 anchor |

## 架构变化

### 1. Viewport 新增 API

```rust
impl Viewport {
    /// 可见的 doc_line 范围（直接从 anchor 计算）。
    pub fn visible_doc_range_from_anchor(
        &self,
        map: &impl LineMap,
        line_height: f32,
    ) -> Range<usize> {
        let start = self.scroll_anchor.doc_line;
        let mut remaining_pixels = self.viewport_height as f32 * line_height
            - self.scroll_anchor.pixel_offset;
        let mut end = start;
        while end < map.map_line_count() && remaining_pixels > 0.0 {
            let vl = map.visual_line_count(end); // 需要新增
            remaining_pixels -= vl as f32 * line_height;
            end += 1;
        }
        start..(end + 1).min(map.map_line_count())
    }

    /// 滚动后更新 anchor（替代 sync_anchor_from_scroll）。
    pub fn scroll_doc_lines(&mut self, delta: isize, map: &impl LineMap) {
        let new_doc = self.scroll_anchor.doc_line.saturating_add_signed(delta)
            .min(map.map_line_count().saturating_sub(1));
        self.scroll_anchor = ScrollAnchor::new(new_doc, 0.0);
    }

    /// 派生 scroll_top（仅供滚动条/外部使用）。
    pub fn derive_scroll_top(&mut self, map: &impl LineMap, line_height: f32) {
        let display_row = map.map_doc_to_display(self.scroll_anchor.doc_line) as f64;
        self.scroll_top = display_row + self.scroll_anchor.pixel_offset as f64 / line_height.max(1.0) as f64;
    }
}
```

### 2. LineMap trait 新增

```rust
pub trait LineMap {
    // ... 现有方法 ...
    /// O(1) 获取某个 doc_line 的 visual_line_count（折合多少 display_row）。
    fn visual_line_count(&self, doc_line: usize) -> u16;
}
```

`DisplayLineMap` 通过 `self.entries[doc_line].visual_line_count` 实现（O(1)）。

### 3. 各路径改动

#### 滚动事件（handle_scroll, scroll_by_visual_lines）

```
旧：dv.viewport.scroll_by(delta) → clamp → sync_anchor
新：dv.viewport.scroll_doc_lines(delta_lines, map)
```

PixelDelta 精确滚动：
```
旧：visual_lines = -pos.y / line_height → scroll_by
新：计算 anchor 行内 pixel_offset = clamp(0, line_height)
    累积 pixel_offset + pixel_delta，满 line_height 时步进 doc_line
```

#### 滚动条

```
旧：thumb 位置 = scroll_top / total_rows
新：thumb 位置 = anchor.doc_line as f64 / total_doc_lines
```

拖拽：
```
旧：scroll_top = drag_ratio * total_rows
新：anchor.doc_line = (drag_ratio * total_doc_lines) as usize
```

#### Page Up / Down

```
旧：scroll_by(±visible_rows)
新：scroll_doc_lines(±visible_doc_count)
```

`visible_doc_count` 从 `visible_doc_range_from_anchor` 计算。

#### Cursor 跟随（ensure_cursor_visible）

```
旧：cursor_display_row < first_visible → scroll_to_row
新：cursor_doc_line < anchor.doc_line → anchor.doc_line = cursor_doc_line
    cursor_doc_line > last_visible → anchor = last_visible - visible_doc_count
```

#### 渲染（shape_visible_lines）

```
旧：first_display = scroll_top.floor(); doc_range = visible_doc_line_range
新：doc_range = visible_doc_range_from_anchor(map, lh)
    从 anchor.doc_line 开始渲染，用 anchor.pixel_offset 偏移第一行
```

#### drain_reshape_results

```
旧：rebuild_tree → restore_scroll_from_anchor → clamp_scroll_top
新：rebuild_tree（仅更新 mapping，供 derive_scroll_top / 滚动条用）
```

不再需要从 anchor 恢复 scroll_top——anchor 就是渲染起点的 SOT。

### 4. 删除/简化

| 删除 | 原因 |
|------|------|
| `sync_anchor_from_scroll` | anchor 由用户操作直接设置，不用从 scroll_top 反推 |
| `clamp_scroll_top`（display_row 版） | 替代为 `clamp_anchor`（doc_line 版） |
| `restore_scroll_from_anchor` 的大部分调用 | 仅保留 `derive_scroll_top`（供滚动条外部使用） |
| `scroll_by` / `scroll_to_row` | 替代为 `scroll_doc_lines` / `scroll_to_doc_line` |

## 实施计划

### 子阶段 5.1：LineMap 扩展 + Viewport 新 API（1 天）

- `LineMap` trait 加 `visual_line_count(doc_line) -> u16`
- `DisplayLineMap` 实现 O(1)
- `Viewport::visible_doc_range_from_anchor`
- `Viewport::scroll_doc_lines`
- `Viewport::derive_scroll_top`
- 删 `Viewport::sync_anchor_from_scroll`（mark deprecated 一版后删）
- 删 `Viewport::scroll_by` / `scroll_to_row`
- 删 `Viewport::clamp_scroll_top`（display_row 版）
- 新增 `Viewport::clamp_anchor`

**测试**：visible_doc_range_from_anchor 在各种 VL 下正确；scroll_doc_lines 边界正确。

### 子阶段 5.2：滚动事件 + 滚动条（0.5 天）

- `handle_scroll`：PixelDelta → anchor.pixel_offset 精确滚动
- `scroll_by_visual_lines`：改为 `scroll_doc_lines`
- 滚动条 thumb：改用 `anchor.doc_line / total_doc_lines`
- 滚动条 drag：直接用 anchor
- Cursor 跟随：用 anchor 替代 scroll_top

**测试**：滚动平滑；scrollbar thumb 位置正确；cursor 跟随不跳。

### 子阶段 5.3：渲染管线（1 天）

- `shape_visible_lines`：从 anchor 向下遍历
- `visible_doc_line_range` → `visible_doc_range_from_anchor`
- 第一行用 `anchor.pixel_offset` 偏移
- `submit_reshape_ahead`：基于 anchor.doc_line 范围提交
- `drain_reshape_results`：只 rebuild_tree，不调任何 scroll 方法

**测试**：可见行正确；首帧位置正确；reshape 后内容不跳。

### 子阶段 5.4：清理 + 回归（0.5 天）

- 删除所有 `sync_anchor_from_scroll` 调用（已被滚动事件直接写入替代）
- 删除 `restore_scroll_from_anchor`（被 `derive_scroll_top` 替代——仅滚动条使用）
- 删除 scroll_top 直接赋值（除 `derive_scroll_top` 外）
- 全量回归测试
- `scroll_top` 标记 `#[doc(hidden)]` 为纯派生量

## 关键设计决策

### 滚动条精度

阶段 5 用 `anchor.doc_line / total_doc_lines` 近似 thumb 位置。placeholder 期间 VL 不准，total_doc_lines 精确（文档行数已知），thumb 位置略有不精确但不影响可用性。reshape 完成后 VL 准确，thumb 位置也准确。

### PixelDelta 精确滚动

```
fn scroll_pixels(&mut self, dy: f32, map: &impl LineMap, line_height: f32) {
    let mut remaining = dy;
    let mut current_vl = map.visual_line_count(self.scroll_anchor.doc_line) as f32 * line_height;

    if remaining > 0.0 {
        // Scroll down
        let space_in_line = current_vl - self.scroll_anchor.pixel_offset;
        if remaining <= space_in_line {
            self.scroll_anchor.pixel_offset += remaining;
        } else {
            remaining -= space_in_line;
            self.scroll_anchor = ScrollAnchor::new(
                self.scroll_anchor.doc_line + 1, 0.0);
            // Recurse for remaining
            self.scroll_pixels(remaining, map, line_height);
        }
    } else {
        // Scroll up
        let space_up = self.scroll_anchor.pixel_offset;
        if -remaining <= space_up {
            self.scroll_anchor.pixel_offset += remaining;
        } else {
            remaining += space_up;
            if self.scroll_anchor.doc_line > 0 {
                let prev_vl = map.visual_line_count(self.scroll_anchor.doc_line - 1) as f32 * line_height;
                self.scroll_anchor = ScrollAnchor::new(
                    self.scroll_anchor.doc_line - 1, prev_vl);
            }
            self.scroll_pixels(remaining, map, line_height);
        }
    }
}
```

## 风险

- **Page up/down 体验变化**：旧用 display_row 步进（等量视觉行），新用 doc_line 步进（文档行）。对全是单行段落差异不大，对有长段落的文件 page down 可能跨越更少视觉行。可后续调整为基于视觉行数估算。
- **全量回归范围大**：改动触及渲染、滚动、事件系统。分 4 个子阶段提交，每个子阶段独立可编译，降低风险。

## 验收

- [ ] scroll_top 完全成为派生量，没有任何"用户路径"直接写 scroll_top
- [ ] 50k CJK 文件，反复切 tab，视觉位置 0 漂移
- [ ] 冷启动恢复 snapshot，位置精确
- [ ] 滚动条 thumb 位置正确
- [ ] 所有现有测试通过
