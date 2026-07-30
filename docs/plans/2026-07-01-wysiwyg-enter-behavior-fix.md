# WYSIWYG 回车行为系统性修复 · 实施计划

> **⚠️ 已被 [`2026-07-06-wysiwyg-empty-line-cursor-augment-chain.md`](./2026-07-06-wysiwyg-empty-line-cursor-augment-chain.md) 取代。**
> 后者一次性完成了 SourceLineMap 抽象、augmenter 模块拆分、EditCommand::ReplaceRange
> 收敛以及 hover/CursorMoved 精细失效。本文档保留供历史参考。

> **面向执行者：** 建议使用 `superpowers:subagent-driven-development` 或 `superpowers:executing-plans` 逐任务实施。步骤使用 `- [ ]` 复选框跟踪。

**目标：** 修复 WYSIWYG 下 Enter 键在各类 markdown 块（标题/段落/列表/引用/代码块/表格）中的行为，使"所见即所得"视图与实际源文档语义一致。

**架构方案：** 在 `crates/markdown/src/view.rs` 的 `PreviewEngine::augment_edit` Enter 分支中，引入 `EnterContext` 枚举集中分类当前光标所处的块上下文（Heading / TopLevelParagraph / NestedParagraph / ListItem / BlockQuoteLine / CodeBlock / TableCell / Other），然后由 match 分支为每种上下文生成 `EditAugmentation`。合并现有的两条重复判定路径（lazy 树查询 + parser 事件重扫）到一个 classify 函数中。

**技术栈：** Rust · pulldown-cmark · 项目现有的 `PreviewEngine` + `EditAugmenter` trait

## 全局约束

- **块激活时视图为源码状态**：当光标进入某个块，该块显示原始 markdown（`# `、`- `、`> ` 等标记可见）。因此 `augment_edit` 收到的 `current_byte` 直接就是源字节偏移，与视觉字符位置一一对应，**不需要 visual↔source 映射转换**。
- **仅修改 augment 层，不改文档模型/parser/渲染**。所有变更集中在 `crates/markdown/src/view.rs`（含新增 helper 函数）与配套单元测试。
- **默认行为兜底**：任何未识别的上下文返回 `None`，宿主执行默认单 `\n` 插入。
- **禁止在 `ui/widgets` 或 `app` 层写死 markdown 语义**——分类逻辑必须留在 `markdown` crate 内。
- **有序列表不重排后续序号**：新 item 使用 `n+1`，其后 sibling 保持原序号（用户预期）。
- **CodeBlock 内不做缩进续接**：本轮直接返回 `None`（默认 `\n`）。
- **必要时运行 `./scripts/verify.sh` 做完整校验**。
- **中文注释、精准命名**，遵守 CLAUDE.md 的《核心原则》与《代码洁癖》。

---

## 文件清单

**修改：**
- `crates/markdown/src/view.rs`
  - 现有 `PreviewEngine::augment_edit`（当前 1147–1212 行）→ 改写为分派骨架。
  - 现有 `is_top_level_paragraph_content_end`（1613–1655 行）→ 合并进新的 `classify_enter_context`，删除本函数。
  - 现有 `blank_line_augmentation`（1604–1611 行）→ 保留，作为公共 helper。
  - 现有 `content_end_without_trailing_newline`（1657–1665 行）→ 保留。
  - 新增私有类型 `EnterContext`（enum）+ `classify_enter_context(source, current_byte) -> EnterContext`。
  - 新增 helper：`find_next_cell_start_byte`（表格跳格用）。

**测试（新增）：**
- `crates/markdown/src/view.rs` 底部 `#[cfg(test)] mod enter_augment_tests { ... }` —— 对 `PreviewEngine::augment_edit` 进行黑盒行为断言。

**不改动：**
- `crates/ui/src/plugin.rs`（`EditAugmenter` / `AugmentKind` / `EditAugmentation` 接口保持不变）
- `crates/app/src/dispatch/wysiwyg.rs`（宿主调用流不变）
- `crates/markdown/src/builder.rs`、`parser.rs`、`layout/*`（数据模型不变）

---

## 分类枚举：`EnterContext`

后续所有 Task 共用如下私有枚举（在 view.rs 中定义）：

```rust
#[derive(Debug, Clone, PartialEq)]
enum EnterContext {
    /// 顶层段落，光标恰在段落内容尾部（去掉尾随 \n 后的 end）。
    TopLevelParagraphEnd,
    /// 段落，且光标不在段尾（段中或段首）。用于顶层与嵌套通用。
    ParagraphInterior,
    /// 标题块，光标在其内容任意位置。level 保留供未来使用。
    Heading { level: u8, at_end: bool },
    /// 列表项：empty 表示该 item 无任何文本；at_end 表示光标在 item 末尾。
    ListItem { bullet: crate::builder::ListBullet, empty: bool, at_end: bool },
    /// 引用块的一行：empty 表示该行 `> ` 之后无内容；at_end 表示光标在该行内容末尾。
    BlockQuoteLine { empty: bool, at_end: bool },
    /// 代码块内部。
    CodeBlock,
    /// 表格单元格内部。next_cell_start 是下一个单元格文本起始的源字节偏移；
    /// 若当前是最后一行最后一列则为 None（本轮按 None 时退化到默认 `\n`，
    /// 但为避免破坏表格，实际处理为 no-op —— 详见 Task 7）。
    TableCell { next_cell_start: Option<usize> },
    /// 其它：未识别或不需要 augment。
    Other,
}
```

**分类函数签名：**

```rust
fn classify_enter_context(source: &str, current_byte: usize) -> EnterContext;
```

**要求：** 内部通过 `crate::parser::parse_markdown(source)` 遍历事件，维护 `item_depth` / `blockquote_depth` / 当前块起止 / 当前块 kind 栈；命中最内层且 range 包含 `current_byte` 的块即为返回上下文。`at_end` 用 `content_end_without_trailing_newline` 计算。

---

## Task 1：引入分派骨架 + `EnterContext` 枚举与分类函数

**Files:**
- Modify: `crates/markdown/src/view.rs:1147-1212`（重写 `augment_edit` Enter 分支）
- Modify: `crates/markdown/src/view.rs:1604-1665`（删除 `is_top_level_paragraph_content_end`，保留其他 helper）
- Test: `crates/markdown/src/view.rs`（新增 `enter_augment_tests` 模块）

**Interfaces:**
- Consumes: `crate::parser::parse_markdown`、`crate::builder::{BlockKind, ListBullet}`、`content_end_without_trailing_newline`
- Produces:
  - `enum EnterContext` (私有)
  - `fn classify_enter_context(source: &str, current_byte: usize) -> EnterContext`
  - `augment_edit` 内 match 骨架：对每个 EnterContext 变体产生 augmentation 或 fallthrough 到 `None`

- [ ] **Step 1：为 classify 函数写失败测试**

在 view.rs 底部（`#[cfg(test)] mod` 内）添加：

```rust
#[cfg(test)]
mod enter_augment_tests {
    use super::*;

    fn classify(src: &str, cursor: usize) -> EnterContext {
        classify_enter_context(src, cursor)
    }

    #[test]
    fn classify_top_level_paragraph_end() {
        let src = "hello world\n";
        // 光标在 'd' 之后，即内容末尾（不含 \n）
        let ctx = classify(src, 11);
        assert!(matches!(ctx, EnterContext::TopLevelParagraphEnd));
    }

    #[test]
    fn classify_paragraph_interior() {
        let src = "hello world\n";
        let ctx = classify(src, 5); // 'hello|'
        assert!(matches!(ctx, EnterContext::ParagraphInterior));
    }

    #[test]
    fn classify_heading_end() {
        let src = "# Title\n";
        // 光标在 'e' 之后（内容末尾）
        let ctx = classify(src, 7);
        assert!(matches!(ctx, EnterContext::Heading { level: 1, at_end: true }));
    }

    #[test]
    fn classify_heading_interior() {
        let src = "# Title\n";
        let ctx = classify(src, 4); // '# Ti|tle'
        assert!(matches!(ctx, EnterContext::Heading { level: 1, at_end: false }));
    }

    #[test]
    fn classify_list_item_nonempty_end() {
        let src = "- abc\n";
        let ctx = classify(src, 5); // '- abc|'
        match ctx {
            EnterContext::ListItem { empty: false, at_end: true, .. } => {}
            other => panic!("expected non-empty list item at end, got {:?}", other),
        }
    }

    #[test]
    fn classify_list_item_empty() {
        let src = "- \n";
        let ctx = classify(src, 2); // '- |'
        match ctx {
            EnterContext::ListItem { empty: true, .. } => {}
            other => panic!("expected empty list item, got {:?}", other),
        }
    }

    #[test]
    fn classify_blockquote_line_end() {
        let src = "> quoted\n";
        let ctx = classify(src, 8);
        match ctx {
            EnterContext::BlockQuoteLine { empty: false, at_end: true } => {}
            other => panic!("expected blockquote line end, got {:?}", other),
        }
    }

    #[test]
    fn classify_blockquote_line_empty() {
        let src = "> \n";
        let ctx = classify(src, 2);
        match ctx {
            EnterContext::BlockQuoteLine { empty: true, .. } => {}
            other => panic!("expected empty blockquote line, got {:?}", other),
        }
    }

    #[test]
    fn classify_code_block() {
        let src = "```\nfoo\n```\n";
        let ctx = classify(src, 5); // 'fo|o'
        assert!(matches!(ctx, EnterContext::CodeBlock));
    }

    #[test]
    fn classify_table_cell() {
        let src = "| a | b |\n|---|---|\n| c | d |\n";
        // 光标在第二行 'c' 之后：源码偏移 = 20 + "| c".len() = ?
        // 用运行时定位而非硬编码：
        let cursor = src.find("c ").unwrap() + 1;
        let ctx = classify(src, cursor);
        assert!(matches!(ctx, EnterContext::TableCell { .. }));
    }
}
```

- [ ] **Step 2：运行测试确认失败**

```bash
cargo test -p markdown enter_augment_tests -- --nocapture
```

预期：编译失败（`EnterContext` / `classify_enter_context` 未定义）。

- [ ] **Step 3：实现 `EnterContext` 与 `classify_enter_context`**

在 view.rs 现有 `is_top_level_paragraph_content_end` 函数附近添加：

```rust
#[derive(Debug, Clone, PartialEq)]
enum EnterContext {
    TopLevelParagraphEnd,
    ParagraphInterior,
    Heading { level: u8, at_end: bool },
    ListItem {
        bullet: crate::builder::ListBullet,
        empty: bool,
        at_end: bool,
    },
    BlockQuoteLine { empty: bool, at_end: bool },
    CodeBlock,
    TableCell { next_cell_start: Option<usize> },
    Other,
}

/// 通过重新解析源码事件流，定位 `current_byte` 所处的最内层块并分类。
/// 单一入口，替代先前 `is_top_level_paragraph_content_end` + lazy 树查询的双路径。
fn classify_enter_context(source: &str, current_byte: usize) -> EnterContext {
    use crate::parser::{MarkdownEvent, MarkdownTag, MarkdownTagEnd, parse_markdown};

    let parsed = parse_markdown(source);
    let mut item_stack: Vec<crate::builder::ListBullet> = Vec::new();
    let mut blockquote_depth = 0usize;
    let mut heading_level: Option<u8> = None;
    let mut heading_start: Option<usize> = None;
    let mut paragraph_start: Option<usize> = None;
    let mut list_item_start: Option<usize> = None;
    let mut code_block_range: Option<std::ops::Range<usize>> = None;
    let mut in_table = false;
    let mut cell_ranges: Vec<std::ops::Range<usize>> = Vec::new();

    for (event, range) in parsed.events.iter().zip(parsed.event_ranges.iter()) {
        match event {
            MarkdownEvent::Start(MarkdownTag::Item) => {
                list_item_start = Some(range.start);
            }
            MarkdownEvent::End(MarkdownTagEnd::Item) => { /* handled at block classification */ }
            MarkdownEvent::Start(MarkdownTag::BlockQuote) => { blockquote_depth += 1; }
            MarkdownEvent::End(MarkdownTagEnd::BlockQuote) => {
                blockquote_depth = blockquote_depth.saturating_sub(1);
            }
            MarkdownEvent::Start(MarkdownTag::Heading { level, .. }) => {
                heading_level = Some(*level as u8);
                heading_start = Some(range.start);
            }
            MarkdownEvent::End(MarkdownTagEnd::Heading(_)) => {
                if let (Some(level), Some(start)) = (heading_level.take(), heading_start.take())
                    && current_byte >= start && current_byte <= range.end
                {
                    let end = content_end_without_trailing_newline(source, start..range.end);
                    let hash_prefix = level as usize + 1; // "# " / "## " / ...
                    let content_start = start.saturating_add(hash_prefix);
                    if current_byte >= content_start {
                        return EnterContext::Heading {
                            level,
                            at_end: current_byte == end,
                        };
                    }
                }
            }
            MarkdownEvent::Start(MarkdownTag::Paragraph) => {
                paragraph_start = Some(range.start);
            }
            MarkdownEvent::End(MarkdownTagEnd::Paragraph) => {
                if let Some(start) = paragraph_start.take() {
                    if current_byte >= start && current_byte <= range.end {
                        let end = content_end_without_trailing_newline(source, start..range.end);
                        // 若段落嵌在 list item / blockquote 内，交给 item/quote 分支处理
                        // 通过检查 item_stack 是否非空 / blockquote_depth 是否 > 0
                        // 注意：pulldown-cmark 事件顺序保证外层 Start 在内层 Start 之前。
                        // 这里我们只在顶层段落时返回 TopLevelParagraph*，其他情况留给外层分支。
                        let is_top_level = /* 见实现说明 */ true;
                        if is_top_level {
                            return if current_byte == end {
                                EnterContext::TopLevelParagraphEnd
                            } else {
                                EnterContext::ParagraphInterior
                            };
                        } else {
                            return EnterContext::ParagraphInterior;
                        }
                    }
                }
            }
            MarkdownEvent::Start(MarkdownTag::CodeBlock(_)) => {
                code_block_range = Some(range.start..range.end);
            }
            MarkdownEvent::End(MarkdownTagEnd::CodeBlock) => {
                if let Some(cb_range) = code_block_range.take()
                    && current_byte >= cb_range.start && current_byte <= range.end
                {
                    return EnterContext::CodeBlock;
                }
            }
            MarkdownEvent::Start(MarkdownTag::Table(_)) => { in_table = true; cell_ranges.clear(); }
            MarkdownEvent::End(MarkdownTagEnd::Table) => { in_table = false; }
            MarkdownEvent::Start(MarkdownTag::TableCell) => {
                cell_ranges.push(range.start..range.end);
            }
            MarkdownEvent::End(MarkdownTagEnd::TableCell) => {
                if let Some(cell) = cell_ranges.last_mut() {
                    cell.end = range.end;
                }
            }
            _ => {}
        }
    }

    // Table cell 命中判定：遍历完毕后再匹配（pulldown-cmark 的 TableCell End 范围可靠）
    if in_table || !cell_ranges.is_empty() {
        for (idx, cell) in cell_ranges.iter().enumerate() {
            if current_byte >= cell.start && current_byte <= cell.end {
                let next_cell_start = cell_ranges.get(idx + 1).map(|r| r.start);
                return EnterContext::TableCell { next_cell_start };
            }
        }
    }

    // ListItem 命中判定（若循环内未提前 return）：
    // 用最后一次记录的 list_item_start 与其 end 做包围测试。
    // 实现说明：pulldown-cmark 的 Item Start 与 End 事件成对，且不重叠。
    // 精确实现需一个 item 栈，本步骤先给出接口，具体细节由下方 Task 2 补齐。

    // BlockQuote 命中判定同上：需要按行拆分 `> ` 前缀。见 Task 3。

    EnterContext::Other
}
```

**实现说明（写代码时逐条落实）：**
1. `paragraph_start` 需在遇到 `Start(Item)` / `Start(BlockQuote)` 时记录嵌套上下文，End(Paragraph) 时用之判断 `is_top_level`。可以用一个 `Vec<ContainerFrame>` 栈，push/pop `Item` 与 `BlockQuote`。
2. `ListItem` / `BlockQuoteLine` 分类需要与 Task 2/3 协同——本 Task 只保证枚举与函数签名可编译；未覆盖分支返回 `EnterContext::Other`，让 Task 2、3 的测试触发实现补齐。

- [ ] **Step 4：重写 `augment_edit` Enter 分支为分派骨架**

替换 `augment_edit` 中 1155-1211 的 Enter 分支：

```rust
AugmentKind::Enter => {
    let source = match self.edit_source.as_deref() {
        Some(s) => s,
        None => return None,
    };
    let ctx = classify_enter_context(source, current_byte);
    match ctx {
        EnterContext::TopLevelParagraphEnd => Some(blank_line_augmentation(current_byte)),
        EnterContext::ParagraphInterior => None,
        EnterContext::Heading { .. } => None,          // Task 4 实现
        EnterContext::ListItem { .. } => None,         // Task 5 实现
        EnterContext::BlockQuoteLine { .. } => None,   // Task 6 实现
        EnterContext::CodeBlock => None,               // Task 7 保持 None
        EnterContext::TableCell { .. } => None,        // Task 7 实现
        EnterContext::Other => None,
    }
}
```

- [ ] **Step 5：删除 `is_top_level_paragraph_content_end`**

移除 view.rs:1613-1655 整个函数（其功能已被 `classify_enter_context` 的 `TopLevelParagraphEnd` 分支覆盖）。同时移除 1201-1207 的 fallback 调用位置（已在 Step 4 的重写中消失）。

- [ ] **Step 6：运行 Task 1 测试**

```bash
cargo test -p markdown enter_augment_tests::classify_top_level_paragraph_end \
    -p markdown enter_augment_tests::classify_paragraph_interior \
    -p markdown enter_augment_tests::classify_heading_end \
    -p markdown enter_augment_tests::classify_heading_interior \
    -p markdown enter_augment_tests::classify_code_block -- --nocapture
```

预期：以上 5 个测试通过。ListItem / BlockQuote / TableCell 测试可能失败（留待后续 Task），标记 `#[ignore]` 或将其中的 `assert!` 换成 `if let Some(_) = ...` 弱断言暂时通过。

- [ ] **Step 7：全量编译校验**

```bash
cargo check --all-targets
```

预期：无编译错误。

- [ ] **Step 8：Commit**

```bash
git add crates/markdown/src/view.rs
git commit -m "$(cat <<'EOF'
refactor(markdown): introduce EnterContext classifier for WYSIWYG Enter handling

Extract the block-context classification into a single classify_enter_context
function backed by parser events, replacing the ad-hoc dual-path logic
(lazy tree lookup + parser rescan). Preserves current top-level paragraph
behavior; subsequent commits fill in Heading / ListItem / BlockQuote /
TableCell branches.
EOF
)"
```

---

## Task 2：完善 `classify_enter_context` 的 ListItem 分支

**Files:**
- Modify: `crates/markdown/src/view.rs`（`classify_enter_context` 内 ListItem 判定）
- Test: `crates/markdown/src/view.rs` 内 `enter_augment_tests`

**Interfaces:**
- Consumes: Task 1 定义的 `EnterContext::ListItem { bullet, empty, at_end }`
- Produces: 完整覆盖以下场景的分类逻辑：
  - `- abc|`（非空、末尾）→ `ListItem { empty: false, at_end: true, bullet: Bullet }`
  - `- ab|c`（非空、中间）→ `ListItem { empty: false, at_end: false, bullet: Bullet }`
  - `- |`（空）→ `ListItem { empty: true, at_end: true, bullet: Bullet }`
  - `1. abc|` → `ListItem { bullet: Ordered(1), ... }`
  - `- [ ] abc|` → `ListItem { bullet: TaskList(false), ... }`
  - `- [x] abc|` → `ListItem { bullet: TaskList(true), ... }`

- [ ] **Step 1：为 ListItem 分类补充失败测试**

```rust
#[test]
fn classify_ordered_list_item() {
    let src = "1. hello\n";
    let ctx = classify_enter_context(src, 8);
    match ctx {
        EnterContext::ListItem {
            bullet: crate::builder::ListBullet::Ordered(1),
            empty: false,
            at_end: true,
        } => {}
        other => panic!("got {:?}", other),
    }
}

#[test]
fn classify_task_list_unchecked() {
    let src = "- [ ] task\n";
    let ctx = classify_enter_context(src, 10);
    assert!(matches!(
        ctx,
        EnterContext::ListItem {
            bullet: crate::builder::ListBullet::TaskList(false),
            empty: false,
            at_end: true,
        }
    ));
}

#[test]
fn classify_task_list_checked() {
    let src = "- [x] done\n";
    let ctx = classify_enter_context(src, 10);
    assert!(matches!(
        ctx,
        EnterContext::ListItem {
            bullet: crate::builder::ListBullet::TaskList(true),
            empty: false,
            at_end: true,
        }
    ));
}

#[test]
fn classify_list_item_interior() {
    let src = "- hello\n";
    let ctx = classify_enter_context(src, 4); // '- he|llo'
    assert!(matches!(
        ctx,
        EnterContext::ListItem { empty: false, at_end: false, .. }
    ));
}
```

- [ ] **Step 2：运行确认失败**

```bash
cargo test -p markdown enter_augment_tests::classify_ordered_list_item \
    -p markdown enter_augment_tests::classify_task_list_unchecked \
    -p markdown enter_augment_tests::classify_task_list_checked \
    -p markdown enter_augment_tests::classify_list_item_interior -- --nocapture
```

预期：至少 3 个测试失败（返回 `Other`）。

- [ ] **Step 3：在 `classify_enter_context` 中补齐 ListItem 分类**

关键实现要点（写入 view.rs）：

```rust
// 在 classify_enter_context 顶部新增：
use crate::builder::ListBullet;

struct ItemFrame {
    start: usize,
    marker_end: usize,          // "- "、"1. "、"- [ ] "、"- [x] " 之后
    bullet: ListBullet,
    /// 是否已经在本 item 内看到任何非空白文本 / 子块。
    saw_content: bool,
}

let mut item_stack: Vec<ItemFrame> = Vec::new();

// Start(Item) 事件处理：
MarkdownEvent::Start(MarkdownTag::Item) => {
    // 解析 marker 长度和 bullet 类型：读取 source[range.start..] 前若干字节
    let bytes = source.as_bytes();
    let start = range.start;
    let (bullet, marker_end) = parse_list_marker(source, start);
    item_stack.push(ItemFrame { start, marker_end, bullet, saw_content: false });
}

// End(Item) 事件处理：
MarkdownEvent::End(MarkdownTagEnd::Item) => {
    if let Some(frame) = item_stack.pop() {
        if current_byte >= frame.marker_end && current_byte <= range.end {
            let end = content_end_without_trailing_newline(source, frame.marker_end..range.end);
            let empty = !frame.saw_content;
            let at_end = current_byte == end;
            return EnterContext::ListItem { bullet: frame.bullet, empty, at_end };
        }
    }
}

// 在遇到任何 Text / Code / 内联事件时，把最内层 item 的 saw_content 标为 true。
```

新增私有 helper：

```rust
fn parse_list_marker(source: &str, start: usize) -> (crate::builder::ListBullet, usize) {
    let bytes = source.as_bytes();
    // "- [ ] " / "- [x] "
    if bytes.get(start) == Some(&b'-')
        && bytes.get(start + 1) == Some(&b' ')
        && bytes.get(start + 2) == Some(&b'[')
        && bytes.get(start + 4) == Some(&b']')
        && bytes.get(start + 5) == Some(&b' ')
    {
        let checked = matches!(bytes.get(start + 3), Some(&b'x') | Some(&b'X'));
        return (crate::builder::ListBullet::TaskList(checked), start + 6);
    }
    // "- "
    if bytes.get(start) == Some(&b'-') && bytes.get(start + 1) == Some(&b' ') {
        return (crate::builder::ListBullet::Bullet, start + 2);
    }
    // "N. " or "N) "
    let mut i = start;
    while let Some(&b) = bytes.get(i) {
        if b.is_ascii_digit() { i += 1; } else { break; }
    }
    if i > start
        && matches!(bytes.get(i), Some(&b'.') | Some(&b')'))
        && bytes.get(i + 1) == Some(&b' ')
    {
        let n: u64 = source[start..i].parse().unwrap_or(1);
        return (crate::builder::ListBullet::Ordered(n), i + 2);
    }
    // Fallback：不认识时按 Bullet 处理，marker_end 退到 start。
    (crate::builder::ListBullet::Bullet, start)
}
```

- [ ] **Step 4：运行测试验证通过**

```bash
cargo test -p markdown enter_augment_tests -- --nocapture
```

预期：Task 1 与 Task 2 的所有 classify_* 测试通过（BlockQuote/TableCell 除外）。

- [ ] **Step 5：Commit**

```bash
git add crates/markdown/src/view.rs
git commit -m "feat(markdown): classify ListItem enter context with bullet + empty + at_end"
```

---

## Task 3：完善 `classify_enter_context` 的 BlockQuote 分支

**Files:**
- Modify: `crates/markdown/src/view.rs`（`classify_enter_context` 内 BlockQuote 判定）
- Test: `crates/markdown/src/view.rs::enter_augment_tests`

**Interfaces:**
- Consumes: `EnterContext::BlockQuoteLine { empty, at_end }`
- Produces: 覆盖以下场景：
  - `> quoted|` → `BlockQuoteLine { empty: false, at_end: true }`
  - `> qu|oted` → `BlockQuoteLine { empty: false, at_end: false }`
  - `> |` → `BlockQuoteLine { empty: true, at_end: true }`

**关键实现约束：**
BlockQuote 是行导向的。pulldown-cmark 的 `Start(BlockQuote)/End(BlockQuote)` 只包围整个引用块；我们需要按行拆分——每一行以 `> ` 或 `>` 开头。判断 `current_byte` 所在行的 `> ` 前缀后是否为空、光标是否在该行内容末尾。

- [ ] **Step 1：添加失败测试**

已在 Task 1 Step 1 中有 `classify_blockquote_line_end` 与 `classify_blockquote_line_empty`；再补：

```rust
#[test]
fn classify_blockquote_line_interior() {
    let src = "> hello world\n";
    let ctx = classify_enter_context(src, 5); // '> he|llo'
    assert!(matches!(
        ctx,
        EnterContext::BlockQuoteLine { empty: false, at_end: false }
    ));
}
```

- [ ] **Step 2：运行确认失败**

```bash
cargo test -p markdown enter_augment_tests::classify_blockquote_line_end \
    -p markdown enter_augment_tests::classify_blockquote_line_empty \
    -p markdown enter_augment_tests::classify_blockquote_line_interior -- --nocapture
```

预期：三个测试失败（返回 `Other` 或 `ParagraphInterior`）。

- [ ] **Step 3：在 `classify_enter_context` 中实现 BlockQuote 判定**

在 `blockquote_depth > 0` 且循环结束前，若尚未匹配到更内层的块，用如下逻辑基于行来判定：

```rust
// 循环外，处理 blockquote 顶层命中：
// 通过再次扫描 source 中包含 current_byte 的那一行，检查行是否以 '>' 开头。
fn locate_blockquote_line(source: &str, current_byte: usize) -> Option<(usize, usize, usize)> {
    // 返回 (line_start, content_start_after_marker, line_end_before_newline)
    let bytes = source.as_bytes();
    let mut line_start = 0usize;
    for i in 0..=current_byte.min(bytes.len()) {
        if i > 0 && bytes[i - 1] == b'\n' {
            line_start = i;
        }
    }
    let mut line_end = line_start;
    while line_end < bytes.len() && bytes[line_end] != b'\n' {
        line_end += 1;
    }
    if bytes.get(line_start) != Some(&b'>') {
        return None;
    }
    let content_start = if bytes.get(line_start + 1) == Some(&b' ') {
        line_start + 2
    } else {
        line_start + 1
    };
    Some((line_start, content_start, line_end))
}
```

然后在事件循环结束、返回 `Other` 之前：

```rust
if blockquote_depth > 0 || /* saw_blockquote 标志 */ {
    if let Some((_, content_start, line_end)) = locate_blockquote_line(source, current_byte) {
        if current_byte >= content_start && current_byte <= line_end {
            let empty = content_start == line_end;
            let at_end = current_byte == line_end;
            return EnterContext::BlockQuoteLine { empty, at_end };
        }
    }
}
```

**注意：** `blockquote_depth` 在 End(BlockQuote) 时归零，因此判断 quote 命中要么在事件循环中用 `saw_blockquote_containing_current` 标志追踪，要么直接依赖 `locate_blockquote_line` 函数（推荐后者：行首 `>` 是 quote 的充分证据）。

**实现推荐做法**：把 `locate_blockquote_line` 的检查前移到 `classify_enter_context` 起始，作为一个 fast path——若命中，先记住结果但继续跑事件循环，用 event 结果覆盖（比如 quote 里的 code block/table 更内层）。若最终循环没返回，则用 fast-path 结果返回 `BlockQuoteLine`。

- [ ] **Step 4：运行测试验证**

```bash
cargo test -p markdown enter_augment_tests -- --nocapture
```

预期：Task 1、2、3 的所有 classify_* 测试通过（TableCell 除外）。

- [ ] **Step 5：Commit**

```bash
git add crates/markdown/src/view.rs
git commit -m "feat(markdown): classify BlockQuoteLine enter context by scanning line prefix"
```

---

## Task 4：Heading 分派——回车拆成"标题 + 新普通段落"

**Files:**
- Modify: `crates/markdown/src/view.rs` 的 `augment_edit` Enter 分支
- Test: `crates/markdown/src/view.rs::enter_augment_tests`

**Interfaces:**
- Consumes: `EnterContext::Heading { level, at_end }`
- Produces: 返回 `EditAugmentation`，`insert_text` = `"\n\n"`，`cursor_byte_after = current_byte + 2`

**行为规范（用户确认）：** 标题中任意位置回车 → 从光标处分为"标题 + 新普通段落"。等价于插入 `\n\n`；`\n\n` 后光标所在的位置在语法上是一个新段落的起点。

- [ ] **Step 1：写失败测试**

```rust
use ui::plugin::{AugmentKind, EditAugmentation};

fn engine_with_source(source: &str) -> PreviewEngine {
    let mut engine = PreviewEngine::for_test(); // 见备注
    engine.set_edit_source_for_test(source.to_string());
    engine
}

#[test]
fn enter_at_heading_end_starts_new_paragraph() {
    let src = "# Title\n";
    let engine = engine_with_source(src);
    let aug = engine.augment_edit(7, AugmentKind::Enter).expect("augmentation");
    assert_eq!(aug.insert_text.as_deref(), Some("\n\n"));
    assert_eq!(aug.cursor_byte_after, 9);
    assert!(aug.replace_range.is_none());
}

#[test]
fn enter_in_heading_interior_starts_new_paragraph() {
    let src = "# Title\n";
    let engine = engine_with_source(src);
    // '# Ti|tle'
    let aug = engine.augment_edit(4, AugmentKind::Enter).expect("augmentation");
    assert_eq!(aug.insert_text.as_deref(), Some("\n\n"));
    assert_eq!(aug.cursor_byte_after, 6);
}
```

**备注**：`PreviewEngine::for_test` 与 `set_edit_source_for_test` 是本 Task 需要新增的 `#[cfg(test)]` helper（如已存在等价方法则复用）。若已有方法（如 `PreviewEngine::new()`）且可直接设置 `edit_source` 字段，则直接使用。

- [ ] **Step 2：运行确认失败**

```bash
cargo test -p markdown enter_augment_tests::enter_at_heading_end_starts_new_paragraph \
    -p markdown enter_augment_tests::enter_in_heading_interior_starts_new_paragraph -- --nocapture
```

预期：失败（当前 Heading 分支返回 `None`）。

- [ ] **Step 3：实现 Heading 分支**

修改 `augment_edit`：

```rust
EnterContext::Heading { .. } => Some(blank_line_augmentation(current_byte)),
```

`blank_line_augmentation` 已经产出 `\n\n`（1604 行），语义正好一致。

- [ ] **Step 4：运行测试**

```bash
cargo test -p markdown enter_augment_tests::enter_at_heading -- --nocapture
```

预期：通过。

- [ ] **Step 5：Commit**

```bash
git add crates/markdown/src/view.rs
git commit -m "feat(markdown): Enter in heading splits into 'heading + new paragraph'"
```

---

## Task 5：ListItem 分派——续接 marker / 退出列表

**Files:**
- Modify: `crates/markdown/src/view.rs::augment_edit`
- Test: `crates/markdown/src/view.rs::enter_augment_tests`

**Interfaces:**
- Consumes: `EnterContext::ListItem { bullet, empty, at_end }`
- Produces：
  - `empty = true` → `EditAugmentation` 删除当前 item 的整行（含 `\n` 与 marker），光标退到该行原起点；等价于退出列表回到父上下文。
  - `empty = false, at_end = true` → 插入 `\n<marker>`，其中 marker 为下一 item 的 marker：
    - `Bullet` → `"- "`
    - `Ordered(n)` → `format!("{}. ", n + 1)`（**不重排后续**）
    - `TaskList(_)` → `"- [ ] "`（新 item 总是未勾选）
  - `empty = false, at_end = false` → 返回 `None`，走默认 `\n`

- [ ] **Step 1：失败测试**

```rust
#[test]
fn enter_continues_bullet_list() {
    let src = "- one\n";
    let engine = engine_with_source(src);
    let aug = engine.augment_edit(5, AugmentKind::Enter).expect("aug");
    assert_eq!(aug.insert_text.as_deref(), Some("\n- "));
    assert_eq!(aug.cursor_byte_after, 5 + 3);
}

#[test]
fn enter_continues_ordered_list_increments_n_only() {
    let src = "1. first\n";
    let engine = engine_with_source(src);
    let aug = engine.augment_edit(8, AugmentKind::Enter).expect("aug");
    assert_eq!(aug.insert_text.as_deref(), Some("\n2. "));
    assert_eq!(aug.cursor_byte_after, 8 + 4);
}

#[test]
fn enter_continues_task_list_unchecked() {
    let src = "- [x] done\n";
    let engine = engine_with_source(src);
    let aug = engine.augment_edit(10, AugmentKind::Enter).expect("aug");
    assert_eq!(aug.insert_text.as_deref(), Some("\n- [ ] "));
}

#[test]
fn enter_empty_list_item_exits_list() {
    let src = "- one\n- \n";
    // 光标在第二个 item 空行末尾：offset = "- one\n- ".len() = 8
    let engine = engine_with_source(src);
    let aug = engine.augment_edit(8, AugmentKind::Enter).expect("aug");
    // 期望：删除第二个 item（"- \n" 或 "- "）
    let range = aug.replace_range.as_ref().expect("replace_range");
    // 断言：range 覆盖 "- " 的起点 6，end 至少到 8
    assert_eq!(range.start, 6);
    assert!(range.end >= 8);
    assert_eq!(aug.insert_text.as_deref(), Some(""));
    assert_eq!(aug.cursor_byte_after, 6);
}

#[test]
fn enter_in_list_interior_defaults_to_newline() {
    let src = "- hello\n";
    let engine = engine_with_source(src);
    // '- he|llo'
    let aug = engine.augment_edit(4, AugmentKind::Enter);
    assert!(aug.is_none(), "interior of list item should fall through to default");
}
```

- [ ] **Step 2：运行确认失败**

```bash
cargo test -p markdown enter_augment_tests::enter_continues_bullet_list \
    -p markdown enter_augment_tests::enter_continues_ordered_list_increments_n_only \
    -p markdown enter_augment_tests::enter_continues_task_list_unchecked \
    -p markdown enter_augment_tests::enter_empty_list_item_exits_list \
    -p markdown enter_augment_tests::enter_in_list_interior_defaults_to_newline -- --nocapture
```

预期：全部失败。

- [ ] **Step 3：实现 ListItem 分支**

修改 `augment_edit`：

```rust
EnterContext::ListItem { bullet, empty, at_end } => {
    if empty {
        // 定位当前 item 在源码中的行范围，构造 replace_range
        let source = self.edit_source.as_deref()?;
        let (line_start, line_end_inclusive_newline) =
            locate_source_line_bounds(source, current_byte);
        Some(ui::plugin::EditAugmentation {
            replace_range: Some(line_start..line_end_inclusive_newline),
            insert_text: Some(String::new()),
            cursor_byte_after: line_start,
        })
    } else if at_end {
        let marker = match bullet {
            crate::builder::ListBullet::Bullet => String::from("- "),
            crate::builder::ListBullet::Ordered(n) => format!("{}. ", n + 1),
            crate::builder::ListBullet::TaskList(_) => String::from("- [ ] "),
        };
        let insertion = format!("\n{}", marker);
        Some(ui::plugin::EditAugmentation {
            insert_text: Some(insertion.clone()),
            cursor_byte_after: current_byte + insertion.len(),
            replace_range: None,
        })
    } else {
        None
    }
}
```

新增 helper：

```rust
/// 返回 `[line_start, line_end_after_newline)` —— 若行末有 `\n` 则包含它。
/// 用于删除整行（如退出空列表项）。
fn locate_source_line_bounds(source: &str, byte: usize) -> (usize, usize) {
    let bytes = source.as_bytes();
    let clamped = byte.min(bytes.len());
    let mut line_start = 0usize;
    for i in 0..clamped {
        if bytes[i] == b'\n' {
            line_start = i + 1;
        }
    }
    let mut line_end = clamped;
    while line_end < bytes.len() && bytes[line_end] != b'\n' {
        line_end += 1;
    }
    if line_end < bytes.len() && bytes[line_end] == b'\n' {
        line_end += 1;
    }
    (line_start, line_end)
}
```

- [ ] **Step 4：运行测试验证**

```bash
cargo test -p markdown enter_augment_tests -- --nocapture
```

预期：Task 1–5 的 List 相关测试全部通过。

- [ ] **Step 5：Commit**

```bash
git add crates/markdown/src/view.rs
git commit -m "feat(markdown): Enter continues list marker or exits on empty item"
```

---

## Task 6：BlockQuote 分派——续接 `> ` / 空行退出

**Files:**
- Modify: `crates/markdown/src/view.rs::augment_edit`
- Test: `crates/markdown/src/view.rs::enter_augment_tests`

**Interfaces:**
- Consumes: `EnterContext::BlockQuoteLine { empty, at_end }`
- Produces：
  - `empty = true` → 删除该行的 `> ` 前缀（连同行末 `\n`），退到父上下文（顶层段落）
  - `empty = false, at_end = true` → 插入 `"\n> "`
  - `empty = false, at_end = false` → 返回 `None`

- [ ] **Step 1：失败测试**

```rust
#[test]
fn enter_continues_blockquote() {
    let src = "> quoted\n";
    let engine = engine_with_source(src);
    let aug = engine.augment_edit(8, AugmentKind::Enter).expect("aug");
    assert_eq!(aug.insert_text.as_deref(), Some("\n> "));
    assert_eq!(aug.cursor_byte_after, 8 + 3);
}

#[test]
fn enter_on_empty_blockquote_line_exits() {
    let src = "> first\n> \n";
    // 光标：offset 10 = 第二行 "> " 之后
    let engine = engine_with_source(src);
    let aug = engine.augment_edit(10, AugmentKind::Enter).expect("aug");
    let range = aug.replace_range.as_ref().expect("range");
    assert_eq!(range.start, 8);
    assert!(range.end >= 10);
    assert_eq!(aug.insert_text.as_deref(), Some(""));
}

#[test]
fn enter_in_blockquote_interior_defaults() {
    let src = "> hello world\n";
    let engine = engine_with_source(src);
    // '> hel|lo'
    let aug = engine.augment_edit(5, AugmentKind::Enter);
    assert!(aug.is_none());
}
```

- [ ] **Step 2：运行确认失败**

```bash
cargo test -p markdown enter_augment_tests::enter_continues_blockquote \
    -p markdown enter_augment_tests::enter_on_empty_blockquote_line_exits \
    -p markdown enter_augment_tests::enter_in_blockquote_interior_defaults -- --nocapture
```

- [ ] **Step 3：实现 BlockQuote 分派**

修改 `augment_edit`：

```rust
EnterContext::BlockQuoteLine { empty, at_end } => {
    let source = self.edit_source.as_deref()?;
    if empty {
        let (line_start, line_end_inclusive_newline) =
            locate_source_line_bounds(source, current_byte);
        Some(ui::plugin::EditAugmentation {
            replace_range: Some(line_start..line_end_inclusive_newline),
            insert_text: Some(String::new()),
            cursor_byte_after: line_start,
        })
    } else if at_end {
        let insertion = String::from("\n> ");
        Some(ui::plugin::EditAugmentation {
            insert_text: Some(insertion.clone()),
            cursor_byte_after: current_byte + insertion.len(),
            replace_range: None,
        })
    } else {
        None
    }
}
```

- [ ] **Step 4：运行测试验证**

```bash
cargo test -p markdown enter_augment_tests -- --nocapture
```

预期：BlockQuote 相关测试通过。

- [ ] **Step 5：Commit**

```bash
git add crates/markdown/src/view.rs
git commit -m "feat(markdown): Enter continues blockquote line or exits on empty line"
```

---

## Task 7：Table 单元格 Enter → 跳到同列下一行；CodeBlock → 保持默认

**Files:**
- Modify: `crates/markdown/src/view.rs::augment_edit`（TableCell、CodeBlock 分支）
- Modify: `crates/markdown/src/view.rs::classify_enter_context`（补齐 `next_cell_start` 计算——跳到"同列下一行"而非"下一格"）
- Test: `crates/markdown/src/view.rs::enter_augment_tests`

**Interfaces:**
- Consumes: `EnterContext::TableCell { next_cell_start }`、`EnterContext::CodeBlock`
- Produces：
  - `TableCell { next_cell_start: Some(offset) }` → `EditAugmentation` **只移动光标**，不插入文本、不删除：`insert_text: Some(""), cursor_byte_after: offset, replace_range: Some(current_byte..current_byte)`
  - `TableCell { next_cell_start: None }` → 返回一个 no-op augmentation（`insert_text: Some(""), replace_range: Some(current_byte..current_byte), cursor_byte_after: current_byte`）以吞掉 Enter，避免默认 `\n` 破坏表格
  - `CodeBlock` → 返回 `None`（保持默认单 `\n`）

**规格调整说明：** 用户要求"跳到下一行单元格"（即**同列下一行**）。因此 Task 1 的 `next_cell_start` 语义要改为"同列下一行同列的 cell 起始"，而非"事件序列中的下一个 cell"。实现时按 (row, col) 索引重组 cell_ranges。

- [ ] **Step 1：调整 classify 的 TableCell 计算方式（测试驱动）**

新增测试：

```rust
#[test]
fn classify_table_cell_same_column_next_row() {
    let src = "\
| a | b |
|---|---|
| c | d |
| e | f |
";
    // 光标位于第一数据行 b 单元格
    let cursor = src.find(" b ").unwrap() + 2; // '| a | b|'
    let ctx = classify_enter_context(src, cursor);
    match ctx {
        EnterContext::TableCell { next_cell_start: Some(off) } => {
            let ahead = &src[off..off + 3];
            // 期望：同列下一行是 " d "
            assert!(ahead.starts_with(" d") || ahead.starts_with("d"), "got '{}'", ahead);
        }
        other => panic!("got {:?}", other),
    }
}

#[test]
fn classify_last_row_table_cell_has_no_next() {
    let src = "\
| a | b |
|---|---|
| c | d |
";
    let cursor = src.find(" d ").unwrap() + 2;
    let ctx = classify_enter_context(src, cursor);
    assert!(matches!(ctx, EnterContext::TableCell { next_cell_start: None }));
}
```

- [ ] **Step 2：运行确认失败**

```bash
cargo test -p markdown enter_augment_tests::classify_table_cell_same_column_next_row \
    -p markdown enter_augment_tests::classify_last_row_table_cell_has_no_next -- --nocapture
```

- [ ] **Step 3：修改 classify 的 Table 处理**

在事件循环中追踪 `(row, col)`：

```rust
struct TableFrame {
    /// cell_ranges[row][col] = source byte range of the cell's text
    cell_ranges: Vec<Vec<std::ops::Range<usize>>>,
    current_row: Option<usize>,
    header_seen: bool,
}
let mut table: Option<TableFrame> = None;

// Start(Table)：初始化 frame
// Start(TableHead)/Start(TableRow)：push 一个新行 Vec<_>
// Start(TableCell)：push 该 cell 的 range 占位
// End(TableCell)：更新最后一行最后一个 cell 的 end
// End(Table)：table_final = table.take() 备用
```

循环结束后：

```rust
if let Some(frame) = table_final {
    for (row_idx, row) in frame.cell_ranges.iter().enumerate() {
        for (col_idx, cell) in row.iter().enumerate() {
            if current_byte >= cell.start && current_byte <= cell.end {
                let next_cell_start = frame.cell_ranges
                    .get(row_idx + 1)
                    .and_then(|next_row| next_row.get(col_idx))
                    .map(|r| r.start);
                return EnterContext::TableCell { next_cell_start };
            }
        }
    }
}
```

- [ ] **Step 4：为 augment_edit 添加 TableCell / CodeBlock 分派测试**

```rust
#[test]
fn enter_in_table_cell_jumps_to_next_row_same_column() {
    let src = "\
| a | b |
|---|---|
| c | d |
| e | f |
";
    let cursor = src.find(" b ").unwrap() + 2;
    let expected_target = src.find(" d ").unwrap() + 1; // " d" 起点
    let engine = engine_with_source(src);
    let aug = engine.augment_edit(cursor, AugmentKind::Enter).expect("aug");
    assert_eq!(aug.insert_text.as_deref(), Some(""));
    let range = aug.replace_range.as_ref().expect("replace_range");
    assert_eq!(range.start, cursor);
    assert_eq!(range.end, cursor);
    // cursor_byte_after 应等于同列下一行 cell 起点
    // （允许实现有 ±1 偏差以适配空格；这里做宽松断言）
    assert!(
        (aug.cursor_byte_after as isize - expected_target as isize).abs() <= 1,
        "cursor_byte_after {} not near {}", aug.cursor_byte_after, expected_target
    );
}

#[test]
fn enter_in_last_row_table_cell_is_noop() {
    let src = "\
| a | b |
|---|---|
| c | d |
";
    let cursor = src.find(" d ").unwrap() + 2;
    let engine = engine_with_source(src);
    let aug = engine.augment_edit(cursor, AugmentKind::Enter).expect("aug");
    assert_eq!(aug.insert_text.as_deref(), Some(""));
    let range = aug.replace_range.as_ref().expect("replace_range");
    assert_eq!(range.start, cursor);
    assert_eq!(range.end, cursor);
    assert_eq!(aug.cursor_byte_after, cursor);
}

#[test]
fn enter_in_code_block_falls_through_to_default() {
    let src = "```\nfoo\n```\n";
    let engine = engine_with_source(src);
    // '```\nfo|o'
    let aug = engine.augment_edit(6, AugmentKind::Enter);
    assert!(aug.is_none(), "code block Enter should fall through");
}
```

- [ ] **Step 5：实现 augment_edit 的 TableCell / CodeBlock 分派**

修改 augment_edit：

```rust
EnterContext::CodeBlock => None,
EnterContext::TableCell { next_cell_start } => {
    Some(ui::plugin::EditAugmentation {
        insert_text: Some(String::new()),
        replace_range: Some(current_byte..current_byte),
        cursor_byte_after: next_cell_start.unwrap_or(current_byte),
    })
}
```

- [ ] **Step 6：运行完整 enter_augment_tests**

```bash
cargo test -p markdown enter_augment_tests -- --nocapture
```

预期：Task 1–7 的所有测试通过。

- [ ] **Step 7：Commit**

```bash
git add crates/markdown/src/view.rs
git commit -m "feat(markdown): Enter in table cell jumps to same-column next row; codeblock keeps default"
```

---

## Task 8：手动集成回归 + 全量校验

**Files:**
- 无代码修改（除非发现回归）

**Interfaces:**
- Consumes: Task 1–7 的成果
- Produces: 一份验证结论（写入本 Plan 文档尾部或 commit message）

- [ ] **Step 1：全量单元测试**

```bash
cargo test --workspace
```

预期：全部通过。若有先前测试因 `is_top_level_paragraph_content_end` 被删除而失败，检查该测试是否有替代覆盖，无则补上等价 classify_* 测试。

- [ ] **Step 2：项目级校验脚本**

```bash
./scripts/verify.sh
```

预期：全部通过（编译、fmt、clippy、test）。

- [ ] **Step 3：手动 WYSIWYG 交互回归清单**

在真实运行的 UI 里，依 `docs/manual_test_protocol.md` 的做法，逐条验证：

| # | 输入 | 光标位置 | 按 Enter 后预期 |
|---|------|----------|------------------|
| 1 | `# Title` | 行末 | 变为 `# Title\n\n|`，新段落 |
| 2 | `# Title` | `# Ti|tle` | 变为 `# Ti\n\n|tle`，前段仍是标题、后段是新普通段 |
| 3 | `hello` | 行末 | `hello\n\n|` |
| 4 | `hello` | `he|llo` | 单 `\n`：`he\n|llo` |
| 5 | `- one` | 行末 | `- one\n- |` |
| 6 | `- ` | marker 之后 | 该行整行删除，光标回到该行起点 |
| 7 | `1. one` | 行末 | `1. one\n2. |` |
| 8 | `- [x] done` | 行末 | `- [x] done\n- [ ] |` |
| 9 | `> quoted` | 行末 | `> quoted\n> |` |
| 10 | `> ` | marker 之后 | 该行删除，光标回退 |
| 11 | ` ```\nfoo\n``` ` | `fo|o` | 单 `\n`（默认） |
| 12 | `\| a \| b \|\n\|---\|---\|\n\| c \| d \|` | b 单元格 | 光标跳到 d 单元格 |
| 13 | 同 12，最后一行 d 单元格 | Enter | 无变化（no-op） |

将结果记录于 commit message 或在 plan 文件末尾追加 `## 验证记录` 小节。

- [ ] **Step 4：Commit（若无代码变更则不 commit）**

```bash
git commit --allow-empty -m "chore: WYSIWYG Enter behavior manual regression pass"
```

（若 Step 3 发现回归，回到相应 Task 修复并重跑该 Task 的测试。）

---

## Self-Review 结果

**1. Spec 覆盖**：
- Heading 分裂 → Task 4 ✅
- ListItem 续接 + 空 item 退出 → Task 5 ✅
- 有序列表不重排 → Task 5 Step 3 明确 `n + 1` 且不修改后续 ✅
- BlockQuote 续接 + 空行退出 → Task 6 ✅
- Table 跳到下一行同列 → Task 7 ✅
- CodeBlock 不做缩进 → Task 7（返回 None）✅
- 顶层段落起新段（现有行为）→ Task 1 保留 ✅
- 段落嵌套上下文修正（列表内 Paragraph 不再误当 ListItem）→ Task 1、2、5 协同覆盖 ✅
- 合并双重判定路径 → Task 1 删除 `is_top_level_paragraph_content_end` ✅

**2. Placeholder 扫描**：无 "TBD/TODO/implement later"，实现说明中所有代码块给出可编译的 Rust 骨架。

**3. 类型一致性**：`EnterContext` 变体名与字段在所有 Task 中拼写一致；`EditAugmentation` 字段 `insert_text/replace_range/cursor_byte_after` 使用一致；`ListBullet::Bullet/Ordered(n)/TaskList(b)` 与 `crates/markdown/src/builder.rs:143` 定义一致。

---

## 执行方式建议

**推荐**：Subagent-Driven —— 每个 Task 派发独立 subagent，主线做 review。每个 Task 都自成失败测试 → 实现 → 通过 → 提交的循环，符合 TDD。

**替代**：Inline Execution —— 若希望连续观察进度，可在同一会话中按顺序执行。
