# Markdown WYSIWYG 空行占位与编辑边界复审

日期：2026-09-06。基线：`23f4bba943d3b7be1c314115806441ee0eb74ee0`。范围：系统分析与诊断测试，不修改生产行为。

## 结论

当前问题应分成两层：**空段落的排版已经统一，创建、合并和删除空段落的编辑语义仍未全部统一。** 继续给分割线或标题追加光标偏移无法解决剩余问题。

当前提交已修复旧报告中的分割线前光标错位、标题/代码/列表后的样式继承、引用前重复预留高度，以及光标处于空段落时连续回车/退格跳槽等问题。本次运行当前基线的 Markdown 库测试：1217 通过，0 失败，包括对应的几何往返回归。

新增 7 个诊断测试：2 个观察/协议确认测试通过，5 个不变量测试失败。其中包括明确的输入/删除不对称，也包括旧行为与“每次创建或删除一个可编辑空段落”的目标冲突。标题开头的处理是现有测试明确固定的行为，不能冒称为本次重构引入的回归。

## 1. 先区分四类“空白”

| 类型 | 当前表示 | 是否有独立可编辑行 |
|---|---|---|
| Markdown 块分隔 | 空行串中的隐藏分隔符 | 否；源码锚点折叠到邻接内容 |
| 可编辑空段落 | 编辑树中的零宽 `Paragraph` | 是；正常正文行高、字号、段间距 |
| 元素自身的间距 | 标题 margin、分割线 padding、正文段间距 | 否；删除一个换行不保证这些间距消失 |
| 有语法的空内容 | `# ` 空标题、空引用/列表项、代码内部空行 | 由所属块决定，不能一律当成普通空段 |

纯预览不注入编辑用空段落；此报告讨论 WYSIWYG 编辑视图。

当前普通顶层文本的空段落数量如下（`\n` 表示一个 LF，而非两个字面字符）：

| 源码 | 可编辑空段数 |
|---|---:|
| `a\n\nb` | 0 |
| `a\n\n\nb` | 1 |
| `a\n\n\n\nb` | 2 |
| `a\n` | 1 |
| `a\n\n` | 1 |
| `a\n\n\n` | 2 |
| `\n\na` | 2 |
| 空文档 | 1 个基础编辑位置 |

同一串换行位于文中、文首或文末，意义不同。因此不能用统一的“插入两个换行”代表所有位置的新段落。

## 2. 当前实现链路与已修复部分

```text
源码 + 光标/选区
  → EditPolicy / augmenter
  → 源码替换事务
  → parse_markdown → MarkdownDoc::build_for_editing
  → EditableParagraphMap 判定空行归属及隐藏分隔
  → 注入零宽 Paragraph
  → 普通块排版 → FlatLine → SourceProjectionIndex
  → 光标、命中、导航、IME
```

关键代码：

- `crates/markdown/src/editable_paragraphs.rs:181`：空行串的容器、前后块及隐藏分隔数量；`:238` 注入真正的普通段落节点。
- `crates/markdown/src/layout/block.rs:117`：空段落与有文字段落走同一排版入口；`:1126` 标记零 grapheme 的空行投影。
- `crates/markdown/src/layout/types.rs:310`：空行几何取自实际布局行；原子块源码范围也参与隐藏分隔查询。
- `crates/markdown/src/editable_paragraph_navigation.rs:9`、`:42`：空段落上的 Enter/Backspace 共用归属表。
- `crates/markdown/src/editable_paragraph_edit.rs:11`、`:73`：输入文字和 Backspace 删除最后一个字共用归属表。

这修复了“空态和输入首字后的几何不同”，但只接入了部分编辑入口。

`view_empty_paragraph_tests.rs` 已有段落、分割线（包括文末）、标题、引用、列表、代码、表格的空态→输入→Backspace 往返，涵盖 1×/2× DPI；也有嵌套容器、IME 绘制、点击、空白输入和连续回车/退格测试。它们主要从已有空槽操作，没有覆盖下面的非空段落边界和替代删除方式。

## 3. 分割线：正常间距与删除异常要分开

以 `前段\n\n---\n\n后段` 为例，在前段末尾 Enter，生成的可编辑空段落位于分割线上方。当前提交的回归已保证：输入首字再用 Backspace 删空，行位不跨越分割线。

分割线自身占据布局高度，上下还有结构间距。点击分割线或其范围会展开源码标记；只有一条隐藏分隔时，间距本身不是额外空段。不能以“还看得见空白”判断删除失败。

当前仍有两种可复现的实际异常：

- 分割线前有额外可编辑空段时，从前段末尾按 Delete 会整次被护栏消费，空段删不掉（见第 6 节）。
- 空段里输入文字后，用 Delete 或选中删除删空，会增加一个可编辑空段（见第 5 节）。

它们发生在源码编辑阶段，排版只是忠实显示了不一致的编辑结果。

## 4. Enter：布局认空白行，旧入口只认连续换行字节

诊断矩阵：3 类前块（正文、H1、H2）× 7 类后缀 × LF/CRLF，共 42 组。测量真实编辑增强后的布局行数。18 组不满足“一次段末 Enter 增加一个空段落”；其余 24 组满足。

| 场景 | 实测结果 | 影响 |
|---|---|---|
| 常规文末 `a|` | `a\n\n|` | 新增一个空段，正常 |
| 常规块间 `a|\n\nb` | `a\n\n|\nb` | 新增一个空段，正常 |
| 已有终止换行 `a|\n` | `a\n\n|` | 总行数 2→2，没有新增；再 Backspace 得到 `a|`，原有尾部空段消失 |
| 分隔空行带空格 `a|\n \nb` | `a\n\n|\n \nb` | 总行数 2→4，一次多出两个空段；Backspace 一次仍留下额外空段 |
| 分隔空行带 Tab | 与带空格相同 | LF/CRLF 均复现 |
| 文首正文 `|a` | `\n\n|a` | 独立诊断：一条正文变成三行，多出两个空段 |

正文、H1、H2 的段尾问题一致。

根因：`augmenter.rs:719` 的 `emit_block_break` 仅检查紧邻的换行序列；没有用 `EditableParagraphMap` 判断带空格空行，也不知道文首、文末已有可编辑槽位。一般插入两个换行，紧贴 EOF 换行或连续换行时插入一个。这套旧字节规则与 `editable_paragraphs.rs:216` 的隐藏分隔规则没有保持统一。

建议：先计算本次创建普通段落前后的空槽数量及容器归属，再生成一次源码替换。不能靠追加 EOF、空格、Tab 的独立常量分支解决。

## 5. 删除最后一个字：Backspace、Delete、选中删除不等价

下面四组前后块都复现：普通段落/普通段落、普通段落/分割线、普通段落/标题、标题/普通段落。

```text
初始空段：a\n\n|\nb            （一个可编辑空段）
输入“新”：a\n\n新|\n\nb
Backspace：a\n\n|\nb          （恢复一个可编辑空段）
Delete 删除“新”：a\n\n|\n\nb （变成两个可编辑空段）
选中“新”再删除：同 Delete
```

真实排版总行数：初始 3，Backspace 后 3，Delete 后 4，选中删除后 4。这里存在实质上的多余占位，与字体度量无关。

根因：物化空段落时 `materialize_paragraph` 会补足隐藏分隔；`erase_last_grapheme` 专门撤掉了多余分隔，但只在 Backspace 路径调用。`augment_delete_forward` 没有对应处理，`plan_selection_edit` 仅增强 Enter/Shift+Enter，其余选区编辑直接 `UseDefault`。默认删除只删除指定 grapheme/选区字节。

代码：`editable_paragraph_edit.rs:39`、`:73`；`augmenter.rs:119`、`:145`；`view.rs:2647`；`crates/app/src/edit_transaction.rs:175`。

另一个协议测试直接调用真实 `EditPolicy::plan_edit`，确认单字前 Delete、选中单字后 Delete/Backspace 均为 `UseDefault`，无选区单字后的 Backspace 为 `Apply`。选中删除的源码结果按应用默认替换规则计算，没有模拟原生鼠标拖选或窗口按键事件。

建议：为“段落内容变空”提供与删除方向无关的语义处理。选区需按受影响块生成事务，保留选区外空段和容器；不能把任意跨块选区简单套成单字删除。

## 6. 从非空块边界删除：仍会跳过可编辑空段落

光标已经在空段落时，最近一次重构实现了逐段 Backspace；光标在下一条非空段落开头时，仍走旧的整串换行合并逻辑。

| 操作前 | 实测结果 | 总布局行数 |
|---|---|---:|
| `a\n\n\n\n|b`，Backspace | `a|b` | 4→1 |
| `# H\n\n\n\n|b`，Backspace | `# H|b` | 4→1 |
| `> q\n\n\n\n|b`，Backspace | `> q\n> |b` | 4→1，后段成为引用延续 |
| `- x\n\n\n\n|b`，Backspace | `- x\n  |b` | 4→1，后段成为列表延续 |
| `---\n\n\n\n|b`，Backspace | 少一个换行 | 4→3，对照正常 |
| `a|\n\n\n\nb`，Delete | `a|b` | 4→1 |
| `a|\n\n\n\n# H`，Delete | 不变 | 4→4 |
| 后块换成分割线、引用、列表，Delete | 不变 | 4→4 |

根因：`augmenter.rs:777` 的 `backspace_paragraph_boundary` 从物理行首反向吞掉整个连续换行串；`:251` 的 `delete_forward_block_boundary` 正向扫描到最终非空块，再决定“合并”还是“保护”。两者都没有先消费最近的可编辑空段落。护栏只看最终块类型，因此即使中间有可以安全删除的空段，也会拦截整个 Delete。

这是**规范迁移缺口**：8 月行为规范曾明确要求段首 Backspace/段尾 Delete 合并整个换行串；9 月新增规范则强调逐个移除可编辑空段。现有代码各自实现了其中一部分。建议统一为：有可编辑空段先移除一个；只剩隐藏边界时，再决定合并文本、移除标题样式或保护原子块。若产品仍希望段首一次跨过所有空段，必须明确作为例外，并让两个方向和导航表现可预期。

## 7. 标题前、中、后不是同一个操作

| 光标位置 | 当前实测行为 | 判定 |
|---|---|---|
| `# Ti|tle` | 前半标题、后半普通段落 | 现有标题中部语义 |
| `# Title|` | 创建普通尾随空段 | 常规路径已有回归；EOF/空白分隔例外见第 4 节 |
| `# |Title` | `# \n|Title` | 留下空标题，原标题内容降为正文；不是在原标题前插入普通空段 |
| `|Title\n===` | 在下划线后创建空段 | Setext 始终在标题末尾退出，不处理标题前插入 |

ATX 标题开头的产物确实包含一条标题行：默认主题下诊断得到空标题行高 35.1px、正文行高 24px。它不是普通空段落错误继承了标题字号，而是源码真的生成了空标题节点。

`classify_heading_hit`（`augmenter.rs:1694`）只区分 `at_end`，没有“内容起点”；`heading_enter_augmentation`（`:1143`）把内容起点和中部统一拆分。`heading_content_start_scans_actual_whitespace_after_hashes`（`:5550`）还明确断言这种结果，故这是既有产品行为，不是当前提交的偶发排版错误。

需要把“在前段末尾回车，把空段插到标题之前”与“在标题文字开头回车”分开：前者已有正常路径，后者目前产生空标题。建议明确新增 `HeadingStart` 语义，保留原标题样式，在标题之前创建普通空段；中间拆分和末尾退出继续独立。

## 8. 修复顺序与验收建议

按当前证据，优先处理可稳定复现的编辑不对称，再确定标题开头的产品语义。实施时拆为每个不超过 3 个文件的子任务：

1. 段落变空统一处理：让单字 Backspace、Delete、选区清空产生相同数量及归属的空段；先保留本报告的失败复现。
2. 块边界删除统一使用空段落映射：先消耗一个可编辑空段，最后才进入真实块合并/原子块保护。
3. 新段落创建统一使用语义边界：覆盖文首、EOF、LF/CRLF、带空格或 Tab 的空行；保证一次 Enter 增一槽，逆操作恢复原槽数量。
4. 标题前/中/后的状态分类与规范同步：避免把“保留标题并在前方插空段”实现为“创建空标题”。

不用重建已经统一的排版模型，也不要在 `ui` 引入 `app` 状态。编辑协议应接收源码、语义边界和选区，输出原子替换及光标落点。

验收至少检查：源码与光标、空段数量及容器、首字输入与删空的 x/y/字号、后继块坐标、命中和 IME、重复操作与撤销重做。几何不仅断言“不重叠”，还应断言同一槽位一致。原子块测试既要确保结构没被合并破坏，也要确保其前方额外空段能逐个删除。

文档需要同步：8 月规范的“文末空行全部可编辑”“InsertText 两侧修剪”“删除整个换行 run”等描述与最新实现/目标不完全一致，应由一份现行行为矩阵统一，旧报告保留基线标记。

## 9. 验证与范围

- `cargo test -p textora-markdown --lib`：1217 passed，0 failed，0 ignored。
- 初次编译被旧工作树绝对路径缓存阻断：LSH 库中编译期记录的 definitions 路径已不存在。执行 `cargo clean -p lsh` 重建对应缓存后成功；没有修改构建脚本或建立旧路径链接。
- 附录诊断临时接入 `view_empty_paragraph_tests.rs`，执行 `cargo test -p textora-markdown --lib boundary_audit -- --nocapture`：2 passed，5 failed，1217 filtered out。所有 5 项最终失败均为行为断言；标题观察和真实编辑协议确认测试通过。
- 最终段尾矩阵为正文/H1/H2；探索阶段曾加入代码围栏，但原始围栏光标不满足共用夹具的 caret 前提，已从最终矩阵移除，没有把那次夹具失败计成产品缺陷。
- 诊断之后移除临时接入，生产代码和现有测试保持基线；只保留本报告及附录复现代码。没有修改测试预期或提交默认忽略的失败测试。
- 没有实施功能修复，不运行全项目 `./scripts/verify.sh`；后续实际修复应按项目要求完整验证。
- 未操作原生应用窗口；没有完成真实 IME、鼠标拖选、撤销重做及滚动事件回放。本次新增边界几何诊断使用 1× DPI，旧回归包含 2× DPI；不把两者混称为新问题全部通过 DPI 验证。

## 10. 后续案例：列表回车把下段变成新列表项正文（已修复）

用户补充源码，在第一项末尾回车：

```markdown
- 金蝶频繁变化,调整太多

  小红书是异地, 上海的
```

修复前生成 `- 金蝶频繁变化,调整太多\n- \n  小红书是异地, 上海的\n`。真实解析树的第二个列表项直接持有“小红书…”正文，没有空项；真实绘制中该文字带上列表标记，光标反而映射到第一项末尾。

根因是 `list_item_enter_augmentation` 将“下一行没有列表标记”视为可消费的续行，空白分隔行也进入该分支。原分隔中的第一个换行被替换为新列表标记，破坏了新空项与后段之间的分隔。

本次分为三个小任务，均不超过三个修改文件：

1. 列表编辑入口及回归（`augmenter.rs`、`view_empty_paragraph_tests.rs`）：用原文复现失败，识别下方内容前唯一的空白分隔行，在该边界插入新项而不消费必要分隔。既有软换行/硬换行拆列表项、额外可编辑空行复用及 EOF 终止换行分支保留。
2. CRLF 空列表项投影（`layout/block.rs`、上述 View 测试）：第一次源码修复后 LF 全部通过，但 CRLF 的空项光标仍消失。根因是空项默认零宽投影以 parser 的 `block_end` 为锚点，其中包含行尾换行；改为物理 marker 行末尾的真实字节位置。没有在光标绘制处增加偏移。
3. 文档与完整验证：记录此例、修复范围与验证结果；第 3—8 节其他问题仍属于此前审查，未在本次修改。

现在回车后的源码为：

```markdown
- 金蝶频繁变化,调整太多
- 

  小红书是异地, 上海的
```

光标位于第二行 `- ` 后。下方原文和缩进原样保留。实际解析对照确认，空列表项之后保留空白分隔时，下方这条带两空格缩进的文字在这一结果中仍解析为独立段落；不需要主动去掉它的缩进。原输入中它曾是第一项续段，若要求继续归属于第一项，则必须把新列表项插在整个旧项后，这是另一种产品操作，不与本次“当前位置新增空项”混用。

验证记录：新增两项回归先在源码断言失败；源码修复后 1 通过、1 在 CRLF caret 断言失败；投影修复后两项全部通过。覆盖 LF/CRLF，空白/空格/Tab 分隔，无序/有序/任务列表源码，原中文实例的解析归属，以及 1×/2× DPI 光标和点击往返。Markdown 全库 1219 passed，0 failed，0 ignored。首次全项目验证发现应用层已有“多空行前新增项再退格恢复原文”的回归失败：全保留分隔会多留一个占位。保留该测试原断言，将修复收窄为只保护唯一必要分隔；有多条空行时继续复用一条生成新项。收窄后 Markdown 全库仍为 1219 passed；原应用层分割线往返测试保持原断言并通过。随后完整验证在同步模块本机 mock 端口监听处受到沙箱 PermissionDenied 阻断；经执行权限审批，在允许本机监听的环境重新运行 `./scripts/verify.sh`，最终退出码 0，架构、格式、工作区 Clippy、应用/工作区测试及文档测试全部通过。独立只读审查未发现 P1/P2；没有修改原有回归预期或跳过失败测试。

本节是后续修复记录；前文“未修改生产行为”描述的是此前 `23f4bba` 基线分析阶段。

## 附录：可重复诊断代码

将以下代码保存到 `/tmp/textora-md-boundary-audit.rs`，在 `crates/markdown/src/view_empty_paragraph_tests.rs` 末尾临时加入 `include!("/tmp/textora-md-boundary-audit.rs");`，运行上面的诊断命令后移除这一行。夹具复用现有 `apply_edit`、`render_at`，不用于默认生产测试集。5 个失败测试的期望是本报告建议统一的空段落不变量；特别是块边界合并，要同时参考第 6 节的旧规范说明。

```rust
fn audit_rows(view: &MarkdownEditorView) -> Vec<(String, f32, f32, f32)> {
    view.engine
        .flat_lines()
        .iter()
        .map(|line| (line.text.clone(), line.rect.x, line.rect.y, line.rect.h))
        .collect()
}

fn audit_forward(source: &str, cursor: usize) -> (String, usize) {
    let mut updated = source.to_owned();
    if let Some(augmentation) = crate::augmenter::augment_delete_forward(source, cursor) {
        updated.replace_range(
            augmentation.replace_range.unwrap_or(cursor..cursor),
            augmentation.insert_text.as_deref().unwrap_or(""),
        );
        return (updated, augmentation.cursor_byte_after);
    }
    let width = source[cursor..].graphemes(true).next().map_or(0, str::len);
    updated.replace_range(cursor..cursor + width, "");
    (updated, cursor)
}

#[test]
fn boundary_audit_enter_at_end_adds_one_row() {
    let mut failures = Vec::new();
    for newline in ["\n", "\r\n"] {
        for block in ["head", "# Title", "## Title"] {
            for suffix in [
                "",
                "\n",
                "\n\n",
                "\n\ntail",
                "\n\n---",
                "\n \ntail",
                "\n\t\ntail",
            ] {
                let prefix = block.replace('\n', newline);
                let source = format!("{prefix}{}", suffix.replace('\n', newline));
                let cursor = prefix.len();
                let (entered, entered_cursor) = apply_edit(&source, cursor, AugmentKind::Enter);
                let mut view = MarkdownEditorView::new();
                render_at(&mut view, &source, cursor, 1, 1.0);
                let before = audit_rows(&view);
                render_at(&mut view, &entered, entered_cursor, 2, 1.0);
                let after = audit_rows(&view);
                let (restored, restored_cursor) =
                    apply_edit(&entered, entered_cursor, AugmentKind::Backspace);
                if after.len() != before.len() + 1 {
                    failures.push(format!("{source:?}@{cursor} -> {entered:?}@{entered_cursor}: rows {} -> {}; backspace={restored:?}@{restored_cursor}", before.len(), after.len()));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "Enter must add one visible paragraph:\n{}",
        failures.join("\n")
    );
}

#[test]
fn boundary_audit_heading_front_and_roundtrip_observation() {
    for source in [
        "# Title",
        "head\n\n# Title",
        "# Title\n\ntail",
        "Title\n===",
    ] {
        let cursor = source.find("Title").expect("fixture has title");
        let (entered, entered_cursor) = apply_edit(source, cursor, AugmentKind::Enter);
        let (restored, restored_cursor) =
            apply_edit(&entered, entered_cursor, AugmentKind::Backspace);
        let mut view = MarkdownEditorView::new();
        render_at(&mut view, &entered, entered_cursor, 1, 1.0);
        println!(
            "HEADING FRONT {source:?}@{cursor} -> {entered:?}@{entered_cursor}; backspace={restored:?}@{restored_cursor}; rows={:?}",
            audit_rows(&view)
        );
    }
}

#[test]
fn boundary_audit_backspace_at_nonempty_start_removes_one_empty_row() {
    let mut failures = Vec::new();
    for previous in ["head", "# Title", "---", "> quote", "- item"] {
        let source = format!("{previous}\n\n\n\ntail");
        let cursor = source.find("tail").expect("fixture has tail");
        let (deleted, deleted_cursor) = apply_edit(&source, cursor, AugmentKind::Backspace);
        let mut view = MarkdownEditorView::new();
        render_at(&mut view, &source, cursor, 1, 1.0);
        let before = audit_rows(&view);
        render_at(&mut view, &deleted, deleted_cursor, 2, 1.0);
        let after = audit_rows(&view);
        println!(
            "BACKSPACE {source:?}@{cursor} -> {deleted:?}@{deleted_cursor}; rows {} -> {}",
            before.len(),
            after.len()
        );
        if after.len() + 1 != before.len() {
            failures.push(previous);
        }
    }
    assert!(
        failures.is_empty(),
        "nonempty-start Backspace skipped editable rows after {failures:?}"
    );
}

#[test]
fn boundary_audit_forward_at_end_removes_one_empty_row() {
    let mut failures = Vec::new();
    for next in ["tail", "# Title", "---", "> quote", "- item"] {
        let source = format!("head\n\n\n\n{next}");
        let cursor = "head".len();
        let (deleted, deleted_cursor) = audit_forward(&source, cursor);
        let mut view = MarkdownEditorView::new();
        render_at(&mut view, &source, cursor, 1, 1.0);
        let before = audit_rows(&view);
        render_at(&mut view, &deleted, deleted_cursor, 2, 1.0);
        let after = audit_rows(&view);
        println!(
            "FORWARD {source:?}@{cursor} -> {deleted:?}@{deleted_cursor}; rows {} -> {}",
            before.len(),
            after.len()
        );
        if after.len() + 1 != before.len() {
            failures.push(next);
        }
    }
    assert!(
        failures.is_empty(),
        "Delete skipped or could not remove editable rows before {failures:?}"
    );
}

#[test]
fn boundary_audit_last_character_delete_methods_keep_same_empty_row() {
    let mut failures = Vec::new();
    for source in [
        "head\n\n\ntail",
        "head\n\n\n---",
        "head\n\n\n# Title",
        "# Title\n\n\ntail",
    ] {
        let cursor = source.find("\n\n\n").expect("fixture has slot") + 2;
        let (typed, typed_cursor) =
            apply_edit(source, cursor, AugmentKind::InsertText("新".to_owned()));
        let (backspaced, backspaced_cursor) =
            apply_edit(&typed, typed_cursor, AugmentKind::Backspace);
        let insertion_start = typed_cursor - "新".len();
        let (forward, forward_cursor) = audit_forward(&typed, insertion_start);
        let mut selected = typed.clone();
        selected.replace_range(insertion_start..typed_cursor, "");
        let mut view = MarkdownEditorView::new();
        render_at(&mut view, source, cursor, 1, 1.0);
        let before = audit_rows(&view);
        render_at(&mut view, &backspaced, backspaced_cursor, 2, 1.0);
        let backward_rows = audit_rows(&view);
        render_at(&mut view, &forward, forward_cursor, 3, 1.0);
        let forward_rows = audit_rows(&view);
        render_at(&mut view, &selected, insertion_start, 4, 1.0);
        let selected_rows = audit_rows(&view);
        println!(
            "ERASE {source:?} typed={typed:?}; backspace={backspaced:?}; forward={forward:?}; selection={selected:?}; rows before/back/forward/selection={}/{}/{}/{}",
            before.len(),
            backward_rows.len(),
            forward_rows.len(),
            selected_rows.len()
        );
        if forward_rows.len() != before.len() || selected_rows.len() != before.len() {
            failures.push(source);
        }
    }
    assert!(
        failures.is_empty(),
        "last character removal added phantom paragraphs: {failures:?}"
    );
}

#[test]
fn boundary_audit_document_start_enter_adds_one_empty_paragraph() {
    let source = "head";
    let (entered, cursor) = apply_edit(source, 0, AugmentKind::Enter);
    let mut view = MarkdownEditorView::new();
    render_at(&mut view, &entered, cursor, 1, 1.0);
    let rows = audit_rows(&view);
    println!("DOCUMENT START {source:?}@0 -> {entered:?}@{cursor}; rows={rows:?}");
    assert_eq!(
        rows.len(),
        2,
        "one Enter should add only one leading empty paragraph"
    );
}

#[test]
fn boundary_audit_actual_edit_policy_confirms_delete_fallback() {
    use ui::plugin::{EditIntent, EditPlan, EditPolicy, EditRequest};
    let source = "head\n\n新\n\ntail";
    let character_start = "head\n\n".len();
    let character_end = character_start + "新".len();
    let mut view = MarkdownEditorView::new();
    view.set_source(source.to_owned(), 1);
    for intent in [EditIntent::DeleteForward, EditIntent::DeleteBackward] {
        let request = EditRequest {
            source_generation: 1,
            cursor_byte: character_end,
            selection: Some(character_start..character_end),
            intent,
        };
        assert!(matches!(view.plan_edit(&request), EditPlan::UseDefault));
    }
    let request = EditRequest {
        source_generation: 1,
        cursor_byte: character_start,
        selection: None,
        intent: EditIntent::DeleteForward,
    };
    assert!(matches!(view.plan_edit(&request), EditPlan::UseDefault));
    let request = EditRequest {
        source_generation: 1,
        cursor_byte: character_end,
        selection: None,
        intent: EditIntent::DeleteBackward,
    };
    assert!(matches!(view.plan_edit(&request), EditPlan::Apply(_)));
}

```

## 11. 系统修复实施记录

用户随后授权按本报告系统修复。实现规范见 [统一段落编辑语义](../specs/2026-09-06-wysiwyg-paragraph-edit-semantics.md)，任务与验收记录见 [实施计划](2026-09-06-wysiwyg-paragraph-edit-semantics.md)。本文前述失败输出和诊断代码保留为历史基线，不表示当前预期。

修复在同一套 `EditableParagraphMap` 上完成：

- 创建：正文起点、已有 EOF 空段、空格/Tab 分隔、标题起点/末尾按可编辑段数决定换行；标题前插保留原始语法与容器。
- 删空：Backspace、Delete 和整段选区清空复用完整段落范围删除；多行链接使用解析器完整语法范围，不残留链接地址或括号。
- 导航：相邻空段每次只删一个，优先于样式合并和原子块保护；跨引用/列表边界保留退出容器需要的中性分隔。
- 布局：Setext 不再套用 ATX 标记映射；合法列表空首行按真实 marker-only 源码和解析归属进入同一空段映射。列表回车分隔保护及 CRLF 空项投影修复保留。
- 事务：实际 EditPolicy 三种删除路径、1×/2× DPI、点击命中及应用 Undo/Redo 验证；选区跨多个段落或原子块继续既有默认策略，不扩展为跨块重写。

验证遵循先失败复现再修复。阶段审查发现的多行链接残留、跨引用吞并正文、样式边界遗漏及首行正文误判，均纳入正式回归；最终全量结果统一记录在实施计划。
