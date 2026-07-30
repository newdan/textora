# Markdown WYSIWYG 编辑器设计

## 1. 目标

在现有 `PreviewEngine` 架构上实现类似 Typora 的所见即所得编辑体验：

- 光标离开时隐藏 markdown 标记符（`#`, `**`, `*` 等），显示富文本排版
- 光标进入时局部展开该区域的源码标记符，允许原地编辑
- 纯预览模式（Novel / 只读 .md）走最快静态分支，零额外开销
- 暂不处理表格和图片的特殊交互

## 2. 核心原则：Thin View, Smart Controller

**app 层是大脑**：拥有输入（winit → EditCommand）、快捷键映射（Keybindings）、撤销重做（Undo/Redo）、IME 管理。

**插件是布局专家**：通过 `PluginQuery` 回答布局相关的查询（像素→字节映射、视觉导航），通过 `PluginMessage` 接收源码和光标变更通知。

插件不拦截原始按键，不直接修改 Document——编辑走 app 层统一管线。

## 3. 架构概览

```
crates/ui/src/plugin.rs          ← 新增 PluginMessage::SetCursorByte
                                    新增 PluginQuery: HitTestByte, VisualMove, CursorScreenPos, AugmentEdit

crates/markdown/src/
  view.rs                         ← MarkdownEditorView + MarkdownPreviewView (impl ViewPlugin)
  engine.rs                       ← PreviewEngine (共享引擎，从 view.rs 提取)
  builder.rs                      ← StyleSpan 增加 source_range
  parser.rs                       ← ParsedMarkdown 保留 event_offsets
  layout.rs                       ← EditContext + materialize_text()
```

```
app 层 (大脑)                           markdown 插件 (布局专家)
─────────────                          ─────────────────────────
winit → EditCommand                    PluginQuery::HitTestByte
    → DocumentView 执行编辑              PluginQuery::VisualMove
    → Undo/Redo 记录                    PluginQuery::CursorScreenPos
    → 快捷键映射                         PluginQuery::AugmentEdit
    → IME 光标定位                       PluginMessage::UpdateSource
    → Dispatch to plugin                PluginMessage::SetCursorByte

         app ──── PluginMessage ────→ plugin   (通知变更)
         app ←── PluginQuery ─────── plugin   (查询布局)
```

```
┌─ MarkdownEditorView ──┐    ┌─ MarkdownPreviewView ──┐    ┌─ NovelView ──┐
│  allows_editing: true   │    │  allows_editing: false   │    │  false       │
│  shows_cursor: false    │    │  shows_cursor: false     │    │  false       │
│  shows_gutter: false    │    │  shows_gutter: false     │    │  false       │
│                         │    │                          │    │              │
│  engine.edit_ctx:       │    │  engine.edit_ctx: None   │    │  None        │
│    Some(EditContext)    │    │                          │    │              │
└──────────┬──────────────┘    └──────────┬───────────────┘    └──────┬───────┘
           │                              │                           │
           └──────────────┬───────────────┴───────────────────────────┘
                          │
                   ┌──────▼──────┐
                   │ PreviewEngine│
                   │              │
                   │ edit_ctx:    │
                   │  Option<Edit │
                   │  Context>    │
                   └──────┬──────┘
                          │
                   ┌──────▼──────┐
                   │ LazyLayout   │
                   │ (per-span    │
                   │  text select)│
                   └─────────────┘
```

两个 View 各自持有自己的 `PreviewEngine` 实例，不作运行时共享。切换编辑/预览模式时，app 层创建新 View，通过 `PluginQuery::ScrollY` 恢复滚动位置。

## 4. PluginMessage / PluginQuery 扩展

### 4.1 新增 PluginMessage

```rust
pub enum PluginMessage {
    // ... 现有变体保持不变

    /// 通知插件光标在源码中的位置已变更。
    SetCursorByte(usize),
}
```

### 4.2 新增 PluginQuery

```rust
pub enum PluginQuery {
    // ... 现有变体保持不变

    /// 源码字节 → 屏幕像素坐标。用于 IME 选词框定位。
    CursorScreenPos(usize),
    // → PluginResponse::CursorRect(Option<(f32, f32, f32, f32)>)
    //   返回 (x, y, w, h)，None 表示无法解析

    /// 屏幕像素坐标 → 源码字节偏移。用于鼠标点击定位光标。
    HitTestByte { x: f32, y: f32, offset_x: f32, offset_y: f32 },
    // → PluginResponse::BytePosition(Option<usize>)

    /// 视觉方向导航：从 current_byte 向上/下/左/右移动一行（对于编辑模式需考虑折叠），
    /// 返回目标源码字节偏移。
    VisualMove {
        current_byte: usize,
        direction: MoveDirection,
        /// 上/下移动时保持的偏好 X 像素位置，用于跨行锚定。
        target_x: Option<f32>,
    },
    // → PluginResponse::BytePosition(Option<usize>)

    /// 询问插件是否需要对此次编辑做干预（如回车自动续接列表标记，
    /// 退格配对删除加粗标记符等）。
    AugmentEdit {
        current_byte: usize,
        kind: AugmentKind,
    },
    // → PluginResponse::Augmentation(Option<EditAugmentation>)
}

pub enum MoveDirection {
    Left,
    Right,
    Up,
    Down,
    LineStart,
    LineEnd,
}

pub enum AugmentKind {
    Enter,
    Backspace,
    Tab,
}

pub struct EditAugmentation {
    /// 实际删除范围 (替代默认单字符退格)。None 表示不修改删除范围。
    pub delete_range: Option<Range<usize>>,
    /// 实际插入文本 (替代默认 "\n")。None 表示使用默认。
    pub insert_text: Option<String>,
    /// 操作后的光标位置偏移。
    pub cursor_byte_after: usize,
}
```

### 4.3 新增 PluginResponse 变体

```rust
pub enum PluginResponse {
    // ... 现有变体

    /// (x, y, w, h) — 光标在文档坐标系中的矩形区域
    CursorRect(Option<(f32, f32, f32, f32)>),
    /// 源码字节偏移
    BytePosition(Option<usize>),
    /// 编辑干预建议
    Augmentation(Option<EditAugmentation>),
}
```

## 5. 源码映射层

### 5.1 parser: 保留偏移量

```rust
pub struct ParsedMarkdown {
    pub events: Vec<MarkdownEvent>,
    /// event_offsets[i] = events[i] 在源码中的起始字节偏移
    pub event_offsets: Vec<usize>,
}
```

`parse_markdown()` 中已有的 `event_offsets` 局部变量改为存入结构体。

### 5.2 builder: StyleSpan 增加 source_range

```rust
pub struct StyleSpan {
    pub start: usize,          // 折叠后文本中的字节偏移
    pub len: usize,            // 折叠后文本的字节长度
    pub style: InlineStyle,
    /// 此 span 在原始源码中的字节范围 (包含标记符)。
    /// e.g. "**world**" → source_range: 6..13
    pub source_range: Range<usize>,
}
```

以 `"hello **world** here"` 为例：

```
源码:           "hello **world** here"
字节:            0     6 8    11 13    18

span[0]: start=0, len=6,  source_range=0..6,   style=Plain    ← "hello "
span[1]: start=6, len=5,  source_range=6..13,  style=Bold     ← "**world**"
span[2]: start=11,len=5,  source_range=13..18, style=Plain    ← " here"
```

### 5.3 builder: BlockNode 增加 source_range

```rust
pub struct BlockNode {
    // ... 现有字段
    /// 此 Block 在原始源码中的字节范围 (包含标记符)。用于增量 diff 和字节定位。
    pub source_range: Range<usize>,
}
```

### 5.4 展开边界判定

```rust
/// 光标是否在此 span 的源码范围内。
/// 使用闭区间右侧 (<= end)：光标在 span 末尾时仍视为"在 span 内"，
/// 让用户能继续在当前样式区域内输入。
fn cursor_in_span(span: &StyleSpan, cursor_byte: usize) -> bool {
    span.source_range.start <= cursor_byte && cursor_byte <= span.source_range.end
}
```

闭区间右侧处理了 `**world**|` 末尾（cursor_byte == 13）的情况——用户在此位置输入，应继续使用加粗样式。

## 6. 展开机制

### 6.1 EditContext

```rust
// crates/markdown/src/layout.rs

pub struct EditContext {
    /// 光标在源码中的字节偏移量。
    pub cursor_byte: usize,
}
```

流经路径：

```text
app 层: DocumentView 光标变更
  → plugin.handle_message(PluginMessage::SetCursorByte(pos))
    → engine.edit_ctx = Some(EditContext { cursor_byte: pos })
      → 下一帧 render() 触发 rebuild_layout()
        → LazyLayout::from_doc(doc, style, viewport_w, engine.edit_ctx.as_ref())
          → materialize_text(edit_ctx) 按 span 拼接文本
```

当 `edit_ctx` 为 `None`（PreviewView / NovelView）时，`materialize_text` 全走折叠文本——和当前逻辑一致，零额外开销。

### 6.2 materialize_text

```rust
fn materialize_text(
    spans: &[StyleSpan],
    source: &str,
    edit_ctx: Option<&EditContext>,
) -> String {
    let Some(ctx) = edit_ctx else {
        // 快速路径：全部折叠 (PreviewView / NovelView)
        return spans.iter().fold(String::new(), |mut acc, s| {
            acc.push_str(s.fold_text(source));
            acc
        });
    };

    // 编辑路径：光标所在 span 展开显示源码
    let mut text = String::new();
    for span in spans {
        if cursor_in_span(span, ctx.cursor_byte) {
            text.push_str(&source[span.source_range.clone()]);  // 含标记符
        } else {
            text.push_str(span.fold_text(source));              // 折叠文本
        }
    }
    text
}
```

光标移动导致展开状态变化时：
1. 该行文本重新拼接 → 重新整形
2. 更新 `y_delta`（若行高/换行变化）
3. 该 Block 及后续 Block 的视觉 Y 重新调整

## 7. 关键数据流（app ↔ plugin）

### 7.1 字符输入

```text
winit ImeCommit("a")
  → app 转 EditCommand::InsertChar('a')
  → DocumentView::execute_edit()  (更新 TextBuffer, 记录 Undo, 更新光标)
  → app: plugin.handle_message(UpdateSource { text, generation })
  → app: plugin.handle_message(SetCursorByte(new_byte))
```

### 7.2 方向键（需要布局知识的视觉导航）

```text
winit Key::ArrowUp
  → app 转 EditCommand::MoveUp
  → app: plugin.query(VisualMove { current_byte, direction: Up, target_x })
  → plugin: 利用 LazyLayout 的 flat_lines + shaped 数据找到上一行同列的源码字节
  → plugin 返回 BytePosition(Some(new_byte))
  → app: DocumentView.set_cursor(new_byte)
  → app: plugin.handle_message(SetCursorByte(new_byte))
```

### 7.3 鼠标点击

```text
winit MouseDown { x, y }
  → app: plugin.query(HitTestByte { x, y })  ← 新查询，返回源码字节而非 flat line 坐标
  → plugin 返回 BytePosition(Some(byte))
  → app: DocumentView.set_cursor(byte)
  → app: plugin.handle_message(SetCursorByte(byte))
```

### 7.4 智能回车

```text
winit Key::Enter
  → app 转 EditCommand::NewLine
  → app: plugin.query(AugmentEdit { current_byte, kind: Enter })
  → plugin 检查光标是否在列表项内 → 返回 Augmentation(Some(EditAugmentation {
        insert_text: Some("\n- ".into()),
        cursor_byte_after: current_byte + 3,
        ..Default::default()
    }))
  → app: 使用 augment 结果执行插入（而非默认 "\n"）
  → app: DocumentView::execute_edit()
  → app: plugin.handle_message(UpdateSource + SetCursorByte)
```

### 7.5 智能退格

```text
winit Key::Backspace
  → app 转 EditCommand::DeleteBackward
  → app: plugin.query(AugmentEdit { current_byte, kind: Backspace })
  → plugin 检查光标是否在标记符边界 → 返回 Augmentation(Some(EditAugmentation {
        delete_range: Some(span.source_range),  // 配对删除整个 **world**
        ..
    }))
  → app: 使用 augment.delete_range 执行删除
  → app: DocumentView::execute_edit()
  → app: plugin.handle_message(UpdateSource + SetCursorByte)
```

### 7.6 IME 选词框定位

```text
app 需要 IME 光标位置
  → app: plugin.query(CursorScreenPos(current_byte))
  → plugin: resolve_cursor_screen_pos(current_byte) → (x, y, w, h)
  → app: 设置 IME candidate window position
```

## 8. 增量布局

每次 `UpdateSource` 通知后的更新流程：

**Step 1**: 全量重解析（pulldown_cmark，对 10KB 以内 .md 文件 <1ms）

**Step 2**: 块级 diff + 增量整形。比较新旧 `MarkdownDoc` 的每个顶层 Block（利用 `source_range` 做哈希对比），区分 Unchanged / Modified / Inserted / Deleted。只对变化 Block 及其后续做重新整形，未变化 Block 保留 `precise = true` 和已有 `y_delta`。

**Step 3**: 视口内 precise pass（HarfBuzz 整形），与现有 `ensure_precise_range` 一致。

## 9. 光标渲染

`PluginQuery::CursorScreenPos(byte)` 的实现：

```rust
impl PreviewEngine {
    fn resolve_cursor_screen_pos(&self, cursor_byte: usize) -> Option<(f32, f32, f32, f32)> {
        let lazy = self.lazy.as_ref()?;
        // 1. 在 MarkdownDoc 中找 source_range 包含 cursor_byte 的 StyleSpan
        let (block_idx, span_idx) = self.find_span_at_byte(cursor_byte)?;

        // 2. 算该字节在 materialize 后行内的字符偏移
        let char_offset = self.char_offset_in_line(block_idx, span_idx, cursor_byte)?;

        // 3. 查该行 ShapedRun → glyph_x_at_char(char_offset) → x 像素
        let flat_line = lazy.flat_line_for_byte(cursor_byte)?;
        let x = flat_line.shaped.as_ref()?.glyph_x_at_char(char_offset)?;

        // 4. 返回 (x, y, w, h)
        Some((x, flat_line.rect.y, 2.0, flat_line.rect.h))
    }
}
```

光标渲染本身：`DrawList` 追加竖线矩形，颜色取自 `theme.editor.cursor_color`，闪烁周期由 app 层 Tick 控制。

Implementation note: cursor screen resolution is based on `MaterializedLine` source maps. The map is produced before wrapping, sliced with wrapped lines, and then used by `HitTestByte`, `CursorScreenPos`, and visual movement. Inline marker expansion therefore has one source of truth.

## 10. MarkdownPreviewView 的键盘响应

预览模式也响应键盘——但只处理滚动/翻页：

```text
winit Key::ArrowDown / PageDown / Space  → plugin.query(HitTestByte) 不需要
                                           → scroll delta via PluginMessage::Scroll
```

PreviewView 的 `handle_message` 已支持 `PluginMessage::Scroll`，无需新增路径。app 层直接在收到滚动相关按键时转为 `PluginMessage::Scroll`，不走 EditCommand。

## 11. 编辑/预览模式切换

`.md` 文件默认以 `MarkdownEditorView` 打开。

用户可通过命令切换为只读预览（`MarkdownPreviewView`），切换时：
- app 层查询当前 View 的 `ScrollY` → 创建新 View → 恢复滚动位置
- 源码从 `DocumentView` 同步

## 12. 实现阶段

### Phase 1: 基础设施
- `PluginMessage::SetCursorByte` + 新增 PluginQuery 变体 + PluginResponse 变体
- `ParsedMarkdown` 保留 `event_offsets`
- `StyleSpan` / `BlockNode` 增加 `source_range`
- builder 传递 source_range 到 AST

### Phase 2: 引擎层
- 提取 `PreviewEngine`（若 merge 方案尚未完成）
- `EditContext` 定义，通过 `SetCursorByte` 流入引擎
- `materialize_text()` 按 span 选文本
- `resolve_cursor_screen_pos()` 光标屏幕坐标解析
- `hit_test_byte()` 屏幕坐标→源码字节映射
- `visual_move()` 视觉方向导航
- `augment_edit()` 智能编辑干预
- 增量布局 (块级 diff + 增量整形)

### Phase 3: View 层
- `MarkdownEditorView` impl ViewPlugin — 接收 UpdateSource/SetCursorByte，提供所有新查询
- `MarkdownPreviewView` impl ViewPlugin — 纯预览，edit_ctx = None
- 光标渲染 (CursorScreenPos + DrawList 竖线)

### Phase 4: app 层
- 编辑管线适配：EditCommand 执行后通知插件 UpdateSource + SetCursorByte
- 方向键 → VisualMove 查询
- 鼠标点击 → HitTestByte 查询
- Enter/Backspace/Tab → AugmentEdit 查询
- IME 定位 → CursorScreenPos 查询
- 编辑/预览模式切换
