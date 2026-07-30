# 阶段 7 完成质量评估

范围：`plans.md` §阶段 7（选择 + 剪贴板 + 撤销/重做）。代码状态：工作树未提交（`Cargo.lock` / `Cargo.toml` / `crates/app/**` / `crates/core/src/buffer/text_buffer.rs` 有改动）。

## 1. 总评

整体落地度 **85%**：`plans.md` 列出的自动化测试 13 项全部存在并通过，性能门槛在 release 下达标，键盘/鼠标交互齐全。**两个明显短板**：

1. **选区在屏幕上不可见**——`render()` 只发文本 + 光标 + 状态栏 vertex，没有为选区生成高亮矩形。逻辑层选区完全正确，视觉上看不出来（手动验收 §7.1「鼠标拖选」会失败）。
2. **双击选词号称 Unicode-aware，实为 ASCII-only**——`navigation.rs:33` 的 256-byte classifier 只标记 ASCII 分隔符，CJK / 阿拉伯 / Thai 全落入 `Word` 类，等于"非 ASCII 视作单词字符"。CJK 测试通过纯属巧合（连续多字节都是 Word，刚好一片选中），并未实现 ICU 边界。

## 2. 验收点逐条核对

### 2.1 自动化测试

| plans.md 要求 | 测试位置 | 状态 |
|---|---|---|
| `mouse_drag_creates_range` | `document_view.rs:3220`、`app.rs:1384` | ✅ |
| `shift_arrow_extends_selection` | `input.rs:473–518`（4 项）+ `document_view.rs:2719` | ✅ |
| `shift_click_extends_to_point` | `app.rs:1327–1332` 实现，`document_view.rs:2740` 测试 | ✅ |
| `triple_click_selects_line` | `app.rs:1312–1321`、`document_view.rs:2759` | ✅ |
| `double_click_selects_word_unicode`（CJK 词边界 ICU） | `document_view.rs:2800` | ⚠️ 通过但不达标，见 §3.2 |
| `history_undo_single_insert` | `text_buffer.rs:3367` | ✅ |
| `history_undo_replace` | `text_buffer.rs:3630` | ✅ |
| `history_redo_after_branch_loses_redo_stack` | `text_buffer.rs:3549` | ✅ |
| `history_coalesce_continuous_typing` | `text_buffer.rs:3580` | ✅ |
| `history_limit_memory_cap` | `text_buffer.rs:3600`（cap 1000） | ✅ |
| `clipboard_roundtrip_utf8` | `document_view.rs:3046` | ✅ |
| `clipboard_eol_normalization_on_paste` | `document_view.rs:2879/2886/2893/2900` | ✅ |
| `clipboard_strip_bom_on_paste` | `document_view.rs:2909/2923` | ✅ |
| `bench_select_1mb_redraw < 16 ms` | `scroll_bench.rs:203` | ⚠️ bench 写法有问题，见 §3.4 |

`cargo test -p edit-plus-core --lib history_` → 7 / 7 通过；`cargo test -p edit-plus-app --lib` Stage 7 套件 → 35 / 35 通过。

### 2.2 性能门槛

| 指标 | 阈值 | 实测 | 状态 |
|---|---|---|---|
| copy 1 MB 文本 | < 50 ms | release 下通过 `copy_1mb_text_performance`（断言内置，未输出实测值） | ✅ |
| undo/redo 50 步 | 总耗时 < 100 ms | release 下通过 `undo_redo_50_steps_performance` | ✅ |
| select 1 MB 重绘 | < 16 ms | bench 不可信（§3.4） | ⚠️ |

### 2.3 手动验收（§7.x）

`plans.md` §7 的手动条目仅在源码看得到逻辑支持，**目测无法验证**：

| 步骤 | 代码层支持 | UI 层 |
|---|---|---|
| 1. 鼠标拖选；状态栏显示选中字符 + 字节 | ✅ `app.rs:556–576` 状态栏文本生成 | ❌ 选区无视觉高亮 |
| 2. Shift+方向；Cmd+A 全选 | ✅ | ❌ 同上 |
| 3. 双击选词；三击选行 | ⚠️ 见 §3.2 | ❌ 同上 |
| 4. Cmd+C↔Safari 互换 | ✅ arboard | n/a |
| 5. Cmd+Z / Shift+Cmd+Z 50 次 | ✅ | ✅ |
| 6. 选中→打字替换 | ✅ `document_view.rs:884` | ❌ 替换发生但选区看不到 |
| 7. RTF 剪贴板取 plain text | ⚠️ 隐式依赖 arboard 行为，无显式测试 | n/a |

## 3. 主要问题

### 3.1 选区未渲染 ⛔ 阻塞

`crates/app/src/app.rs:1051` 的 `render()` 只 push 三类 vertex（文本 / 光标 / 状态栏），**没有 selection 矩形**。状态对得上，但用户看不到自己选了什么。这是 stage 7 最显眼的功能缺口。

修复路径：参考 `cursor_vertices`，新增 `selection_vertices`，对每条可见行算出 `(start_x, end_x, line_y)` 输出矩形。需要在 `shape_visible_lines` 里把 cluster→x 缓存暴露给选区计算（已经有 `advance_cache`）。

### 3.2 词边界不是 ICU ⚠️ 规范不达标

- `crates/core/src/buffer/navigation.rs:13–34`：`construct_classifier` 只接受 ASCII 分隔符（`assert!(ch < 128)`），所有 ≥0x80 字节都落到默认 `CharClass::Word`。
- 结果：对 CJK，整段连续 CJK 被当成一个"词"——表面上行为对，但其实没有真正的词边界。日文、韩文需要 ICU `BreakIterator`；阿拉伯/Thai 同理。
- `double_click_selects_word_unicode` 测试只覆盖 ASCII + hyphen + underscore + digit + 标点，**没有 CJK 大字段**。`double_click_selects_word_cjk` 也只是断言 4 个汉字一起被选中——验证的是字节连片，不是词边界。
- plans.md §阶段 11 才接 ICU；当前提前验收 ICU 词边界不现实，但应该把 §阶段 7 验收说明降级为「按 byte-class 选择，CJK 整段视作一词」，或登记 backlog。

### 3.3 手动测试协议未落地

`docs/manual_test_protocol.md` 不存在（plans.md §10 要求阶段 ≥3 起每阶段一节）。Stage 7 需要的 7 条手动验收没有可执行文档；选区不可见这种问题正是手动验收应该挡住的。

### 3.4 `bench_select_1mb_redraw` 写法错

`crates/app/benches/scroll_bench.rs:208` 用 `b.iter_custom(|_iters| {...})` 但忽略 `iters` 参数——只跑 1 次还报 `18446744074B iterations`，criterion 输出 `0.0000 ps`。结果不可信，等于没有这个 bench。

修复：用标准 `b.iter(|| {...})`，或在 `iter_custom` 里循环 `iters` 次。

### 3.5 字符计数路径低效（非阻塞）

`app.rs:557–576` 的 `status_bar_text` 在每帧重新 `extract_selected_text` + `from_utf8_lossy` + `chars().count()`。1 MB 选区就是每帧一次 1 MB 拷贝 + UTF-8 扫描。短期没问题（select bench 单次 < 50 ms），但当用户全选大文件并触发 60 fps 重绘时会把状态栏算到帧预算里。

修复：选区变化时缓存 `(byte_count, char_count)`，改后再计算。

### 3.6 `cut_selection_to_clipboard` 的 lossy 风险（边界 case）

`document_view.rs:429`、`445`：`String::from_utf8_lossy` 把非法 UTF-8 转成 U+FFFD 后写剪贴板。剪贴板里的内容已经不是原文档字节。plans.md §阶段 7 边界 case 列了「选区跨非法 UTF-8（lossy）」，**没有测试覆盖**。建议加一条：选区含 0xFE 0xFF，copy → clipboard 含 U+FFFD 文档不变，验证「lossy 但不崩」是约定行为。

### 3.7 `extract_selected_text` 全文复制（非阻塞但要看一眼）

`document_view.rs:121–142` 的 `word_select_at` 把整个 buffer 拷到 `Vec<u8>` 再调 `word_select`。1 MB 时无感，50 MB 文件双击会有 50 MB 拷贝。`word_select` 应该可以直接吃 `&dyn ReadableDocument`（`gap_buffer` 实现了），不需要先 `extend_from_slice`。这条 plans 没列，但属于"低 hanging fruit"。

## 4. 结论与建议

- **可声明 stage 7 完成度 85%**：算法 / 数据正确，自动化测试齐全，性能达标。
- **进入 stage 8 之前必须修**：
  - §3.1 选区渲染（不渲染选区，stage 8 的"选中后 Save As"等手动流程也走不通）
  - §3.4 select_1mb bench 写法
- **可以延期但需登记到 plans 里**：
  - §3.2 ICU 词边界（依赖 stage 11）
  - §3.3 手动测试协议骨架（plans.md §10 已有约定，要补文件）
  - §3.5、§3.7 性能 paper cuts
  - §3.6 lossy 选区测试

## 5. 复测命令

```sh
cargo test -p edit-plus-core --lib history_
cargo test -p edit-plus-app  --lib -- stage7_tests
cargo test -p edit-plus-app  --release --lib -- copy_1mb_text_performance undo_redo_50_steps_performance
```
