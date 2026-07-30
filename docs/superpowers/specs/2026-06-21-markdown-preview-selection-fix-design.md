# Markdown Preview 选区修复 — 设计文档

> 日期：2026-06-21
> 性质：根本性重构 — 将选区坐标模型从分层索引改为扁平行索引

## 问题

| # | Bug | 现象 |
|---|-----|------|
| 1 | BlockQuote/Table 内无法选中文本 | 拖选无高亮、Cmd+C 无内容、方向键卡死 |
| 2 | 点击空白区域清除已有选区 | 行间距/边距处 hit test 返回 None → clear_selection |

## 根因

当前 `PreviewPos = (block_idx, line_idx, char_pos)` 中的 `block_idx` 指向 `laid_out.blocks` 顶层数组。但 BlockQuote 子块、Table 单元格嵌套在块内部，不在顶层数组中。`block_lines()` 对 BlockQuote/Table 只能返回 `None`——这不是疏忽，是模型无法表达嵌套位置的固有问题。

## 方案核心

**选区本质是线性的——从 A 到 B 按阅读序。** 将 `PreviewPos` 改为 `(flat_line_idx, char_pos)`，用一个递归遍历构建的扁平行数组 `Vec<FlatLine>` 统一所有选区操作。

```
旧模型：PreviewPos { block_idx, line_idx, char_pos }   ← 3 元组，分层
新模型：PreviewPos { flat_line_idx, char_pos }           ← 2 元组，线性

flat_lines: Vec<FlatLine>  // 阅读序下所有文本行的线性数组
FlatLine { line: LaidOutLine, flat_idx: usize, abs_y: f32, block_idx: usize }
```

## FlatLine.flat_idx 的语义

`flat_idx` 是该行在阅读顺序中的绝对位置。表头按列序->cell行序遍历；表体按行序->列序->cell行序遍历；BlockQuote 递归展开子块；ListItem 先自身行再递归子块。

## 关键影响

| 移除 | 新增 |
|------|------|
| `block_lines()` 方法 | `LazyLayout.flat_lines: Vec<FlatLine>` |
| `block_count()` 方法 | `build_flat_lines()` 递归构建方法 |
| `hit_test_blocks()` 递归函数 | `PreviewPos { flat_line_idx, char_pos }` |
| 所有 `block_lines()?.ok_or(continue)` 模式 | 直接 `flat_lines[idx]` 索引 |

## 吸附算法

- 点击在文档上方 → snap 到 flat_lines[0] 行首
- 点击在文档下方 → snap 到 flat_lines[last] 行尾
- 点击在两行之间 → 按 `abs_y + line_h/2` 距离选择最近行
- 空文档 → 仍返回 None
