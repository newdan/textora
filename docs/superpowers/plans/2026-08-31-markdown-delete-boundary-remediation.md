# Markdown Delete Boundary Remediation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Delete 仅保护真实的 Setext 下划线、主题分隔线和当前 fenced code block 的合法闭合围栏，同时允许与 marker 前缀相似的普通段落正常并行合并。

**Architecture:** 所有判定收敛到 augmenter.rs 的完整物理行语法函数。Setext 与 thematic break 校验 marker 数量及行尾；代码体边界从 enclosing fenced block 的开围栏签名推导闭围栏，不修改公开 EnterContext 枚举。

**Tech Stack:** Rust、pulldown-cmark offset events、textora-markdown 单元测试。

## Global Constraints

- 不改变 augment_edit、EnterContext 或其他公开类型的签名。
- 统一支持 LF 与 CRLF，并允许 CommonMark 规定的至多 3 个前导空格。
- 闭围栏必须字符相同、长度不短于开围栏、尾部仅空格或 tab。
- marker-like 普通文本不能因 starts_with 被误判。
- 先写失败测试，再替换判定逻辑。

---

## Task 1: 为 Delete 边界补齐失败用例

**Files:**
- Modify: crates/markdown/src/augmenter.rs

- [ ] 在 augmenter tests 中新增 delete_forward_protects_short_setext_underlines，表驱动覆盖 title\n=、title\n==、title\n--，断言 augment_edit 返回 Some 且 replace_range == None。

- [ ] 新增 delete_forward_allows_marker_like_paragraphs，表驱动覆盖 first\n---not-a-rule、first\n===not-setext、first\n***plain，断言替换范围只覆盖换行，结果分别合并为 first---not-a-rule、first===not-setext、first***plain。

- [ ] 新增 delete_forward_inside_fenced_code_only_protects_matching_closer，覆盖：

    - 四反引号开围栏中的三反引号内容行允许默认删除；
    - 四波浪号开围栏中的三反引号内容行允许默认删除；
    - 三反引号开围栏后的 ```not-close 允许默认删除；
    - 三反引号开围栏后的三反引号加空白仍受保护。

- [ ] 运行新测试并确认 RED：

    cargo test -p textora-markdown --lib augmenter::tests::delete_forward_protects_short_setext_underlines -- --exact
    cargo test -p textora-markdown --lib augmenter::tests::delete_forward_allows_marker_like_paragraphs -- --exact
    cargo test -p textora-markdown --lib augmenter::tests::delete_forward_inside_fenced_code_only_protects_matching_closer -- --exact

- [ ] 提交测试：

    git add crates/markdown/src/augmenter.rs
    git commit -m "test(markdown): cover exact delete block boundaries"

## Task 2: 用完整物理行语法替换 marker 前缀判断

**Files:**
- Modify: crates/markdown/src/augmenter.rs

- [ ] 增加 source_line_marker_content，返回去除至多 3 个前导空格且去除 CR/LF 与尾随空白的当前物理行切片；超过 3 个前导空格时不作为块 marker。

- [ ] 增加精确 Setext 判定：

    fn line_is_setext_underline(source: &str, line_start: usize) -> bool {
        let Some(content) = source_line_marker_content(source, line_start) else {
            return false;
        };
        let Some(marker) = content.as_bytes().first().copied() else {
            return false;
        };
        matches!(marker, b'=' | b'-') && content.bytes().all(|byte| byte == marker)
    }

- [ ] 增加精确 thematic break 判定：忽略 marker 之间的空格与 tab；所有非空白字符必须同为 *、- 或 _；marker_count >= 3；行内出现其他字符立即返回 false。

- [ ] line_starts_independent_block 使用 line_is_setext_underline；line_starts_new_sibling_block 使用精确 thematic break，保留列表、ATX 标题与 fenced opener 的既有判定。

- [ ] 删除旧的 starts_with("===")、starts_with("***")、starts_with("---")、starts_with("___") 分支和失真的注释。

- [ ] 运行 Setext、marker-like 与既有 Delete/Backspace 测试：

    cargo test -p textora-markdown --lib augmenter::tests::delete_forward_protects_short_setext_underlines -- --exact
    cargo test -p textora-markdown --lib augmenter::tests::delete_forward_allows_marker_like_paragraphs -- --exact
    cargo test -p textora-markdown --lib augmenter::tests

- [ ] 提交完整行语法：

    git add crates/markdown/src/augmenter.rs
    git commit -m "fix(markdown): validate delete marker lines"

## Task 3: 只保护匹配当前开围栏的闭围栏

**Files:**
- Modify: crates/markdown/src/augmenter.rs

- [ ] 增加 line_closes_enclosing_fenced_block(source, current_byte, candidate_line_start)。内部使用 Parser::new_ext(...).into_offset_iter() 跟踪 CodeBlockKind::Fenced 的 Start/End range；仅当 current_byte 位于该 block 内容范围内时，从 frame.range.start 调用 opening_fence_signature，再以 candidate 行的 content_end 调用 line_is_closing_fence。

- [ ] 将 EnterContext::CodeBlock 分支中的 line_starts_code_fence 替换为 line_closes_enclosing_fenced_block；删除不再使用的宽松 helper。

- [ ] 确认候选行 ```not-close、短围栏、异类围栏都返回 false，等长或更长且尾部仅空白的同类围栏返回 true。

- [ ] 运行 fenced code 回归与 augmenter 全模块测试：

    cargo test -p textora-markdown --lib augmenter::tests::delete_forward_inside_fenced_code_only_protects_matching_closer -- --exact
    cargo test -p textora-markdown --lib augmenter::tests

- [ ] 格式化并检查包：

    cargo fmt --all -- --check
    cargo check -p textora-markdown

- [ ] 自审：确认 helper 只解析物理行、没有改变 EnterContext 公共形状、没有重复的宽松围栏判断。

- [ ] 提交围栏修复：

    git add crates/markdown/src/augmenter.rs
    git commit -m "fix(markdown): match delete closing fences"

## Task 4: 用绝对 Markdown 列统一容器边界

**Files:**
- Modify: crates/markdown/src/augmenter.rs

- [ ] 先增加稳定失败用例，覆盖一个 Tab 同时跨越容器必需列并留下合法叶块缩进：

    - `- item\n\n  ```\n  code\n\t```\n\npara`
    - `- title\n\t=`
    - `- para\n\t***`
    - `> 12. ```\n>     code\n> \t\t````
    - 对应 opener、blockquote/list 嵌套与 CRLF 变体

- [ ] 增加纯列推进测试：空格、Tab stop、Tab 恰好命中目标列、Tab 跨过目标列 1–3
  列，以及跨过 4 列后必须拒绝。

- [ ] 引入语义化行位置类型，至少携带 byte offset、absolute column 与 container target
  column。容器匹配返回该类型，禁止再用 `Option<usize>` 丢失 Tab 的虚拟余量。

- [ ] Setext、thematic、fenced opener/closer 从同一语义位置开始校验；叶块自身缩进使用
  `absolute_column - container_target_column`，并与后续物理空格共同限制为 0–3 列。

- [ ] 删除被替代的 byte 等宽 prefix matcher，保持各生产函数低于职责检查线；无法复用的
  fenced 二次 parser 扫描仅保留在单换行 CodeBlock Delete 冷路径。

- [ ] 运行定向测试、augmenter 全模块、Clippy 与包检查：

    cargo test -p textora-markdown --lib augmenter::tests
    cargo clippy -p textora-markdown --all-targets -- -D warnings
    cargo check -p textora-markdown
    cargo fmt --all -- --check

- [ ] 独立复审 pulldown-cmark 对照语料，确认合法结构全部 Consume，伪 marker 与空行路径
  保持 UseDefault。
