# plans_cursor_visual_line.md

## 目标

统一"光标属于哪个可视行"的判断逻辑，消除 `commands.rs` 和 `render_pipeline.rs` 中两套不一致的实现。

---

## 现状

### 两套"找可视行"逻辑

| 位置 | 数据源 | 边界判断 | 用途 |
|------|--------|---------|------|
| `commands.rs` — `cursor_visual_line_bounds` | `advance_cache: &[AdvanceCacheEntry]` | 统一 `offset <= vl_end` | Home / End 等导航命令 |
| `render_pipeline.rs` — 渲染定位 | `shaped.clusters` + `visual_lines` | 非末行 `<`，末行 `<=` | 光标渲染位置 |

两者回答同一个问题，但答案在边界处不同。

### 数据流关系

```
shaped.clusters + visual_lines     (render_pipeline 构建)
        │
        ├─→ 渲染光标定位（直接用 visual_lines）
        │
        └─→ build_advance_cache_entries()
                │
                └─→ advance_cache（传给 commands.rs 使用）
```

`advance_cache` 是渲染数据的序列化版本，结构不同但语义相同。这造成了两个不共享的"找可视行"实现。

### 其他导航键问题

| 命令 | 实现方式 | 问题 |
|------|---------|------|
| MoveUp / MoveDown | `app.rs` 拦截 → `move_cursor_visual` | ✅ 已可视行感知 |
| ExtendUp / ExtendDown | `app.rs` 拦截 → `extend_selection_visual` | ✅ 已可视行感知 |
| Home / End | `commands.rs` 使用 advance_cache | ⚠️ 边界判断 `<=` 与渲染不一致 |
| PageUp / PageDown | `commands.rs` 使用 `tb.cursor_visual_pos()` | ❌ 文档行坐标系，page_size 是可视行数 |

---

## 方案

### 第一步：提取共享判断函数

在 `cursor_motion.rs`（现有的导航工具模块）中新增：

```rust
/// 在可视行边界列表中查找 offset 所属的可视行索引。
///
/// `bounds`: &[(byte_start, byte_end)]，每个可视行的字节范围（相对偏移）。
/// `offset`: 光标在该文档行内的字节偏移。
///
/// 规则：非最后可视行使用 [start, end)，最后可视行使用 [start, end]。
/// 这与渲染定位逻辑一致。
pub(crate) fn find_visual_line_index(
    bounds: &[(usize, usize)],
    offset: usize,
) -> usize {
    let len = bounds.len();
    for (i, &(start, end)) in bounds.iter().enumerate() {
        let matches = if i + 1 < len { offset < end } else { offset <= end };
        if offset >= start && matches {
            return i;
        }
    }
    // fallback: 最后一行
    len.saturating_sub(1)
}
```

两处调用方改为调用它：

1. **`cursor_visual_line_bounds`（commands.rs）**：遍历 advance_cache 之前，先构建 `bounds` 列表，调用 `find_visual_line_index` 得到索引，再用该索引查 advance_cache 取绝对坐标。

2. **渲染定位（render_pipeline.rs）**：替换内联的 `end_match` 循环。

### 第二步：回退 End 键的 home_visual_line_bounds 补丁

`cursor_visual_line_bounds` 修好后，边界光标会正确识别为"下一可视行"，End 自然移动到其末尾。`MoveToLineEnd` 和 `ExtendToLineEnd` 改回使用 `cursor_visual_line_bounds`。

### 第三步：修复 PageUp / PageDown

当前问题：
- `page_size = visible_rows - 1` 是可**视行**数
- `tb.cursor_visual_pos()` 返回的是**文档行**坐标
- 两套坐标系混用，超长行时跳得过远

修复方式：在 `app.rs` 的 `handle_command` 中拦截 PageUp / PageDown，参考 MoveUp/MoveDown 的模式，使用 `move_cursor_visual` 但以 page_size 为步长。

```rust
EditCommand::PageUp => {
    let page = self.doc_view.as_ref().map_or(1, |dv| dv.viewport.visible_rows.saturating_sub(1).max(1));
    self.move_cursor_visual(-(page as isize));
    return;
}
EditCommand::PageDown => {
    let page = self.doc_view.as_ref().map_or(1, |dv| dv.viewport.visible_rows.saturating_sub(1).max(1));
    self.move_cursor_visual(page as isize);
    return;
}
```

### 第四步（可选）：home_visual_line_bounds 简化

`find_visual_line_index` 引入后，`home_visual_line_bounds` 的边界检测逻辑（`dv.cursor_offset != vl_end` 时找下一行）也可以用新函数简化。

---

## 影响范围

| 文件 | 改动 |
|------|------|
| `cr…tes/app/src/cursor_motion.rs` | 新增 `find_visual_line_index` |
| `cr…tes/app/src/commands.rs` | `cursor_visual_line_bounds` 调用新函数；End / ExtendToLineEnd 回退为使用 `cursor_visual_line_bounds` |
| `cr…tes/app/src/render_pipeline.rs` | 内联的 `end_match` 循环改为调用新函数 |
| `cr…tes/app/src/app.rs` | handle_command 中拦截 PageUp / PageDown |

---

## 不做的

- MoveUp / MoveDown / ExtendUp / ExtendDown：已有正确拦截，不动
- Home 键 indent 切换逻辑：功能正确，不动
- advance_cache 数据结构：语义合理，不动
