# 引用块显式行标记换行 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将同一引用块中每个显式 `>` 源码行渲染为独立视觉行，同时保留其他 Markdown 软换行语义。

**Architecture:** 解析器为每个 `SoftBreak` 记录下一物理行是否显式带有 `>`。构建器只在该标志为真且当前位于引用块时提交当前文本行；普通软换行继续映射成一个空格。硬换行复用提交当前文本行的正确路径，避免首行被丢弃。

**Tech Stack:** Rust、pulldown-cmark 0.13、textora-markdown 单元测试。

## Global Constraints

- 只修改 `crates/markdown` 的解析与构建层，不向 `ui` 暴露或传递应用层状态。
- 普通正文软换行和引用块懒续行必须继续显示为空格。
- `\\`、行尾两个空格和 HTML `<br>` 的强制换行必须保留。
- 使用语义化命名，避免 `.unwrap()`，并通过 `cargo fmt`。

---

### Task 1: 为软换行携带显式引用行来源

**Files:**
- Modify: `crates/markdown/src/parser.rs:8-22, 104-116, 280-410`

**Interfaces:**
- Consumes: `pulldown_cmark::Event::SoftBreak` 及其源码 `Range<usize>`。
- Produces: `MarkdownEvent::SoftBreak { next_line_has_explicit_blockquote_marker: bool }`，供 `MarkdownDoc::build` 匹配。

- [ ] **Step 1: 写失败的解析测试**

```rust
#[test]
fn parse_softbreak_tracks_the_next_explicit_blockquote_marker() {
    let parsed = parse_markdown("> first\n> second");

    assert!(parsed.events.iter().any(|event| {
        matches!(
            event,
            MarkdownEvent::SoftBreak {
                next_line_has_explicit_blockquote_marker: true
            }
        )
    }));
}

#[test]
fn parse_softbreak_keeps_a_lazy_blockquote_continuation_unmarked() {
    let parsed = parse_markdown("> first\nsecond");

    assert!(parsed.events.iter().any(|event| {
        matches!(
            event,
            MarkdownEvent::SoftBreak {
                next_line_has_explicit_blockquote_marker: false
            }
        )
    }));
}
```

- [ ] **Step 2: 运行测试并确认失败**

Run: `cargo test -p textora-markdown parse_softbreak_`

Expected: 编译失败，`MarkdownEvent::SoftBreak` 尚不是带字段的变体。

- [ ] **Step 3: 实现最小解析标记**

```rust
SoftBreak {
    next_line_has_explicit_blockquote_marker: bool,
}

fn next_line_has_explicit_blockquote_marker(src: &str, softbreak_range: &Range<usize>) -> bool {
    let Some(newline_offset) = src[softbreak_range.start..].find('\n') else {
        return false;
    };
    let next_line_start = softbreak_range.start + newline_offset + 1;
    matches!(src[next_line_start..].trim_start_matches([' ', '\t']).chars().next(), Some('>'))
}
```

在 `Event::SoftBreak` 分支调用该辅助函数并构造带标志的事件；其他事件保持不变。

- [ ] **Step 4: 运行解析测试并确认通过**

Run: `cargo test -p textora-markdown parse_softbreak_`

Expected: 两项测试通过。

- [ ] **Step 5: 提交任务**

```bash
git add crates/markdown/src/parser.rs
git commit -m "feat(markdown): mark explicit blockquote softbreaks"
```

### Task 2: 按显式引用行提交文本行

**Files:**
- Modify: `crates/markdown/src/builder.rs:652-681, 905-910, 1334-1380`

**Interfaces:**
- Consumes: `MarkdownEvent::SoftBreak { next_line_has_explicit_blockquote_marker }`。
- Produces: 引用块段落的 `BlockNode::text_lines`，每个显式 `>` 行对应一个元素。

- [ ] **Step 1: 写失败的构建测试**

```rust
#[test]
fn builder_preserves_each_explicit_blockquote_source_line() {
    let source = "> 日期：2026-07-20\n> 状态：待评审\n> 目标：加载 Wiki";
    let doc = MarkdownDoc::build(&parse_markdown(source), &default_style());
    let paragraph = &doc.blocks[0].children[0];

    assert_eq!(paragraph.text_lines, ["日期：2026-07-20", "状态：待评审", "目标：加载 Wiki"]);
}

#[test]
fn builder_keeps_lazy_blockquote_continuation_as_one_line() {
    let source = "> first\nsecond";
    let doc = MarkdownDoc::build(&parse_markdown(source), &default_style());
    let paragraph = &doc.blocks[0].children[0];

    assert_eq!(paragraph.text_lines, ["first second"]);
}

#[test]
fn builder_preserves_hard_break_lines() {
    let source = "first\\\nsecond";
    let doc = MarkdownDoc::build(&parse_markdown(source), &default_style());

    assert_eq!(doc.blocks[0].text_lines, ["first", "second"]);
}
```

- [ ] **Step 2: 运行测试并确认失败**

Run: `cargo test -p textora-markdown builder_preserves_`

Expected: 第一个测试把三行合并为一行；第三个测试仅保留最后一行。

- [ ] **Step 3: 实现最小构建逻辑**

```rust
fn is_inside_blockquote(&self) -> bool {
    self.block_stack.iter().any(|block| matches!(block.kind, BlockKind::BlockQuote))
}

MarkdownEvent::SoftBreak { next_line_has_explicit_blockquote_marker } => {
    if *next_line_has_explicit_blockquote_marker && builder.is_inside_blockquote() {
        builder.flush_line_into_current_block();
    } else {
        builder.push_soft_break(builder.current_event_range.clone());
    }
}
MarkdownEvent::HardBreak => builder.flush_line_into_current_block(),
```

将现有 `builder_preserves_blockquote_softbreak_source_jump` 改为断言两个独立投影行，首行终点位于首行文本后、第二行起点位于 `second` 源码位置。

- [ ] **Step 4: 运行构建测试并确认通过**

Run: `cargo test -p textora-markdown builder_preserves_`

Expected: 三项测试通过。

- [ ] **Step 5: 格式化并运行包级验证**

Run: `cargo fmt --check && cargo test -p textora-markdown && cargo check -p textora-markdown`

Expected: 所有命令以退出码 0 结束。

- [ ] **Step 6: 提交任务**

```bash
git add crates/markdown/src/builder.rs
git commit -m "fix(markdown): preserve explicit blockquote lines"
```
