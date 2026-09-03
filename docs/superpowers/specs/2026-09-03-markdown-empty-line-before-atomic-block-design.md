# Markdown 原子块前空行光标定位设计

## 背景与复现

源码：

```md
图片需讨论

---

移动
```

光标位于“图片需讨论”末尾时按一次 Enter，编辑器会在段落与水平分割线之间插入一个可编辑空源码行。源码与后续输入位置均正确，但空行尚无文字时，光标错误显示在分割线下方；输入任意文字后，新段落获得普通文字投影，光标才回到分割线上方。

## 根因

`LazyLayout::collect_source_only_empty_line_projections` 构造 `RenderedLineLayout` 时只保留拥有 `source_projection` 的文字行。水平分割线是非文字原子块：它拥有 `atomic_source_range`，但没有文字投影，因此不会进入 `SourceLineMap::attach_layout`。

`SourceLineMap` 随后无法识别水平分割线所在源码行及其视觉矩形。它为 Enter 新增的空行计算邻接几何时会越过分割线，把分割线下方的普通段落当成下一渲染行，导致空行投影和光标矩形落到分割线下方。输入文字后，该空行成为普通段落并获得文字投影，所以布局恢复。

## 设计

让源码行布局同时接收以下两类视觉行：

1. 文字视觉行：继续使用 `source_projection.source_extent`。
2. 原子视觉块：使用 `atomic_source_range`。

两类输入统一转换为 `RenderedLineLayout { source_range, y_top, height }`，按源码范围和视觉顺序交给 `SourceLineMap::attach_layout`。水平分割线源码行因此被识别为真实渲染行，分割线前后的空行 run 会分别以它为边界计算。

原子块不获得伪造的 grapheme 投影，也不进入文字光标映射；它只参与源码行级别的垂直布局。因此，上一轮已经实现的原子块点击边界语义保持不变。

## 行为边界

- Enter 新增的可编辑空行必须位于前一段落与水平分割线之间。
- 空光标、IME 预编辑光标和输入文字后的普通段落使用一致的垂直槽位。
- 单个块间分隔空行仍为 `HiddenBlockSeparator`。
- 每个额外可编辑空行仍占用 `line_height + paragraph_spacing`。
- 不改变水平分割线的渲染高度、上下内间距或点击映射。
- 普通段落、标题、列表、代码块和表格的现有投影语义不变。
- 设计应自然适用于以后新增的非文字原子块，不对水平分割线类型做光标层特判。

## 数据流

```text
FlatLine
  ├─ source_projection.source_extent（文字行）
  └─ atomic_source_range（原子块）
             ↓
RenderedLineLayout
             ↓
SourceLineMap::attach_layout
             ↓
ProjectedEmptyLine / HiddenBlockSeparator
             ↓
空行光标、IME、导航和命中测试
```

## 测试策略

先写失败回归测试，再修改生产代码：

1. 使用最小源码复现段落末尾 Enter，并应用真实的 `EditAugmentation`。
2. 重新布局后断言新增空行光标底部不越过水平分割线顶边。
3. 在同一字节位置加入文字后重新布局，断言文字段落仍占用原空行槽位，不发生跨越分割线的跳变。
4. 在 1× 与 2× DPI 下重复核心几何断言。
5. 保留已有水平分割线点击、空行高度、源码更新和普通块命中测试。

## 非目标

- 不重新设计 Markdown 块间距。
- 不改变 Enter 的源码编辑策略。
- 不让水平分割线生成文字或 grapheme 投影。
- 不在绘制光标时增加只针对水平分割线的 y 坐标修补。
