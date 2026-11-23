# Markdown 间距规范化方案 — 已完成

## 实施结果

所有 7 个间距问题已修复，172 个测试全部通过。

### 修复内容

1. **BlockQuote 双重 paragraph_spacing** → 删除重复行
2. **CodeBlock→ListItem guard 叠加** → 删除 guard（CodeBlock 已有 trailing）
3. **Heading→ListItem guard 叠加** → 删除 guard（Heading 已有 trailing）
4. **HR→ListItem guard 叠加** → 删除 guard（HR 已有 trailing）
5. **List→non-list 间距不足** → guard 改为 `paragraph_spacing - list_item_spacing`
6. **Para→Heading 间距过大** → 实现 margin collapsing
7. **调整间距参数** → paragraph 0.75, h_top 1.0, h_bot 0.45, bq_pad 12, rule 18

### 间距参数 (line_height=24px)

| 名称 | 公式 | 像素值 |
|------|------|--------|
| `paragraph_spacing` | 0.75 × lh | 18px |
| `heading_spacing_top` | 1.0 × lh | 24px (H1) |
| `heading_spacing_bottom` | 0.45 × lh | 10.8px |
| `list_item_spacing` | 0.15 × lh | 3.6px |
| `blockquote_padding` | 固定 | 12px |
| `rule_spacing` | 固定 | 18px |

### 核心机制：Heading margin collapsing

Heading 的 `heading_spacing_top` 与前块的 `last_trailing_spacing` 做 max 运算：
```
extra = (desired_top - last_trailing_spacing).max(0.0)
```

### 最终间距矩阵

```
A→B          旧值   新值
Para→H1     50.4   24.0  (margin collapsing)
Code→List   43.2   18.0  (去重)
BQ→Para     43.2   18.0  (去重)
HR→List     70.6   18.0  (去重)
H→List      33.6   10.8  (去重)
List→end    7.2    18.0  (补足到 paragraph_spacing)
```

### 涉及文件

- `crates/markdown/src/layout.rs` — layout 逻辑 + margin collapsing + guard 修复
- `crates/markdown/src/style.rs` — 间距参数调整
