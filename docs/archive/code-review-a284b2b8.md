# Code Review: a284b2b8 — Markdown Preview Pipeline

## 整体评价

架构设计良好：parser → builder → layout → render 四层 pipeline，每层职责清晰，对标 zed 的 markdown crate。38 个 markdown 单元测试（含 11 个 e2e）覆盖了核心路径。不过有几个值得关注的问题。

---

## 问题

### 1. 文本样式信息丢失——bold/italic/link 等未实际渲染（关键）

这是最大的功能问题。Builder 层正确维护了 `text_style_stack`（push/pop Bold、Italic、Link 等），但 `PendingLine` 只存了 `text: String`，没有存 style runs。`LaidOutLine` 也只有一个 flat `text` 字段。最终 `render_line()` 用单一颜色 emit 一个 Text cmd——所有 inline 样式（粗体、斜体、行内代码、链接颜色/下划线）全部丢失。

**影响**：预览中看不到 **bold**、*italic*、`code`、[links] 的视觉差异，显示为统一 plain text。

**修复方向**：`LaidOutLine` 需要支持 run-based 渲染，按不同样式分段 emit 多个 Text cmd，或至少保留 `TextStyleMod` 列表供 render pass 使用。

**涉及文件**：
- `crates/markdown/src/builder.rs` — `PendingLine` 需存储 style runs
- `crates/markdown/src/layout.rs` — `LaidOutLine` 需携带分段样式信息
- `crates/markdown/src/render.rs` — `render_line()` 需按 run 分段发射不同样式 Text

---

### 2. TaskListMarker 事件被 parser 丢弃

parser.rs 中 `_ => {}` 的 catch-all 会将 pulldown_cmark 的 `Event::TaskListMarker(bool)` 静默丢弃。Builder 层的 `ListBullet::TaskList` 类型和 render 层的 checkbox 渲染代码已写好，但因为 parser 不产生此事件，永远不会被触发。

**涉及文件**：
- `crates/markdown/src/parser.rs` — `Event::TaskListMarker(checked)` → 需新增对应 `MarkdownEvent` 变体

**修复**：
```rust
// parser.rs: 在 match event 中添加
Event::TaskListMarker(checked) => {
    events.push(MarkdownEvent::TaskListMarker(checked));
}
```

---

### 3. Image 事件处理为空操作

parser 产生 `MarkdownTag::Image { url, title }`，builder 处理为设置 `link_depth` + `link_url`，但没有任何渲染代码使用这些值。alt text 在 parser 转换过程中丢失。

**建议**：当前阶段至少显示 alt text 或 `[Image: url]` 作为占位文本。

**涉及文件**：
- `crates/markdown/src/builder.rs` — Image 事件处理段
- `crates/markdown/src/parser.rs` — Image alt text 丢失

---

### 4. `collect_text_lines` 返回空串兜底

```rust
if texts.is_empty() {
    texts.push(String::new());
}
```

这导致空 block 也创建一个空的 `LaidOutLine`，浪费 layout/render 计算。应该直接返回空 Vec，让调用方自行处理。

**涉及文件**：`crates/markdown/src/layout.rs` — `collect_text_lines()` 函数

---

### 5. app_renderer 中全文读取每帧都执行（性能）

```rust
let gb = dv.tb().gap_buffer();
let c1 = gb.read_forward(0);
let c2 = gb.read_forward(c1.len());
```

这会在每次 `render()` 调用时读取整个 gap buffer 的全部内容。虽然 `MarkdownPreview.set_source()` 内部有 hash 检查避免 re-parse，但 `read_forward` 本身的 O(n) 拷贝每帧都在发生。

**建议**：在 app 层先比较内容 hash，只有变化时才做全文提取和 `set_source`。

**涉及文件**：`crates/app/src/app_renderer.rs` — markdown preview 渲染段

---

### 6. `_style` 字段从未读取

```rust
struct MarkdownBuilder<'a> {
    _style: &'a MarkdownStyle,
```

Builder 不需要 style（样式在 layout 阶段才用到），传进来但不使用。可以直接去掉这个字段。

**涉及文件**：`crates/markdown/src/builder.rs` — `MarkdownBuilder` struct + `new()` 方法

---

### 7. `link_url` 写后不读

`MarkdownBuilder.link_url` 被 `Image` 和 `Link` 事件写入，但没有任何后续代码读取它来渲染链接样式。属于未完成的功能残留。

**涉及文件**：`crates/markdown/src/builder.rs` — `MarkdownBuilder.link_url` 字段

---

### 8. 新增依赖命名（验证项）

`Cargo.toml` 中：
```toml
edit-plus-markdown = { path = "../markdown" }
```

导入时用 `use edit_plus_markdown::...`。需确认 `paint_backend::drain()` 是 `paint_backend` 的公开方法，否则编译失败。

**涉及文件**：
- `crates/app/Cargo.toml`
- `crates/app/src/app_renderer.rs`

---

## 优点

- **Pipeline 架构清晰**：parser / builder / layout / render 四层分离得当，每层有明确的输入/输出类型
- **测试覆盖扎实**：38 个测试覆盖解析、构建、布局、渲染和端到端路径
- **缓存策略合理**：`LaidOutDoc` 在滚动时保持不变（仅 re-render），theme/source 变化时才 re-layout
- **Scroll 集成干净**：`app_scroll.rs` 中 preview 滚动处理和正常编辑器滚动完全独立
- **`from_theme()` 支持亮/暗主题**：两套颜色值，没有硬编码
- **Table 渲染完整**：含 header 背景、rows、vertical/horizontal grid lines
- **代码风格一致**：遵循项目现有约定（`pub(crate)` 可见性、`//!` 模块文档、中文注释在 app 层）

---

## 总结

核心 pipeline 架构是好的，测试也到位。但 **文本样式不渲染（问题 1）** 需要在合并前解决——否则预览模式下所有 markdown 看起来都是纯文本，失去了预览的核心价值。问题 2（TaskListMarker 丢失）也应一起修复。
