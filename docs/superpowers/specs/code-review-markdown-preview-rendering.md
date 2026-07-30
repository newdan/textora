# Code Review: `codex/markdown-preview-rendering`

## 概览

15 个提交，18 个文件，新增 2,591 行。实现了完整的 Markdown 预览渲染管线（parse → build → layout → render → DrawList），通过 `Cmd+Shift+M` 切换预览模式。

**测试结果：** 59 个 markdown 测试全部通过，0 失败。

---

## 架构（良好）

管线分层清晰：

| 模块 | 职责 |
|------|------|
| `parser.rs` | 封装 `pulldown-cmark`，产出无生命周期的 owned 事件流 |
| `builder.rs` | 事件驱动 AST 构建，产出 `BlockNode` 树 |
| `layout.rs` | 计算位置/尺寸，可选接入 Shaper 做精确测量 |
| `render.rs` | 将 laid-out blocks 转为 `DrawCmd` 列表 |
| `style.rs` | 纯数据配置，从 `Theme + Settings` 派生 |

`md_preview.rs` 中的 `MarkdownPreview` 缓存设计合理——滚动时不重新解析/布局，仅在源文本变化、buffer generation 变化或主题/样式变化时失效。

---

## 需要修复的问题

### 1. 硬编码的裁剪宽度 — `render.rs:2378`

```rust
dl.cmds.push(DrawCmd::PushClip(Rect::new(offset_x, offset_y, 5000.0, viewport_h)));
```

`5000.0` 是随意选取的魔数。应使用实际视口宽度（或者若意图是「不裁剪水平方向」则用 `f32::MAX`）。在超宽显示器或编辑器区域超过 5000px 时，裁剪会出错。**这是最具体的问题。**

### 2. 死代码 — `render.rs`

`render_block` 和 `render_line`（不带 offset 的版本）未被使用。只有 `_with_offset` 变体被调用。要么移除，要么让非 offset 版本委托给 `_with_offset` 并传入 `0.0, 0.0`。当前产生 3 个编译警告。

### 3. `estimate_text_width` 使用了字节长度 — `render.rs:2585`、`layout.rs:1178`

```rust
text.len() as f32 * font_size * 0.55
```

`text.len()` 返回的是字节长度，不是字符数。对 CJK 文本会高估宽度最多 3 倍。这仅在无 Shaper 的降级路径上使用，影响有限，但内联样式的光标定位会偏差。降级路径可考虑用 `text.chars().count()`。

### 4. 表格行高未考虑多行单元格 — `layout.rs:1443`

```rust
row_y += line_h + 4.0;
```

所有单元格行高固定为 `line_h + 4.0`，不考虑单元格内实际折行后的行数。多行内容的单元格会溢出或重叠。

### 5. `from_utf8` 静默失败 — `md_preview.rs:128`

```rust
text.push_str(std::str::from_utf8(c1).unwrap_or(""));
```

如果 gap buffer 中出现了非法 UTF-8，预览会静默地不显示文本。对文本编辑器而言这是极端边缘情况，但加一个 `log::warn!` 有助于调试。

### 6. 嵌套内联样式丢失信息 — `builder.rs:646`

只记录了栈顶样式。`***bold italic***` 仅保留一个样式（取决于哪个标签后打开）。对 v1 而言可接受（目前仅按颜色区分），但后续若需要区分粗体/斜体字体渲染，这里需要重做。

---

## 次要问题 / 风格

- **中英混合注释**：`ui_shell.rs:407` 用中文（"切换 markdown 预览模式"），其他文件用英文。建议统一。
- **`BlockKind::TableRow_` / `TableCell_`**：尾部下划线不寻常，暗示它们是内部/中间节点——宜加注释说明原因。
- **`crates/app/Cargo.toml`** 末尾多了一个空行，不影响功能。

---

## 做得好的地方

- **CJK 边界安全**：`floor_char_boundary`、按字符索引二分搜索、回归测试
- **裁剪**：渲染前正确 clip 到视口
- **滚动集成**：预览独立滚动，`scroll_y` 带 `max_scroll` clamp
- **缓存粒度**：样式、源文本、generation 分别 hash，各自独立失效
- **测试覆盖全面**：parser、builder、layout、render 均有单元测试；集成测试验证端到端；边界测试覆盖 CJK、emoji、空文档、零宽度视口、负滚动等

---

## 结论

**建议通过，附带小修。** 上述问题均属于「合入前值得修」级别，非阻塞性问题。硬编码裁剪宽度（#1）和死代码警告（#2）是最值得处理的两项。架构扎实，测试覆盖全面。
