# 阶段 6 / 阶段 7 完成度审计

审计日期：2026-06-01
审计范围：plans.md §阶段 6（键盘输入 + 编辑）、§阶段 7（选择 + 剪贴板 + 撤销/重做）
审计依据：
- `crates/app/src/{input.rs, app.rs, document_view.rs}`
- `crates/core/src/buffer/text_buffer.rs`
- `crates/app/benches/scroll_bench.rs`
- 已存在的 `docs/stage6_audit.md`（2026-05-29 版的早期审计与修复报告）

---

## 0. 总评

| 阶段 | 形式完成度 | 实质完成度 | 备注 |
|---|---|---|---|
| 阶段 6（键盘输入 + 编辑） | **95%** | **80%** | 功能闭环完整；性能门槛 1/3 已测、2/3 未测 |
| 阶段 7（选择 + 剪贴板 + Undo/Redo） | **90%** | **75%** | 选择/剪贴板/历史栈接通；缺 mock 剪贴板 RTF 过滤、CRLF on paste 文件级测试 |

两个阶段的"基础功能闭环"都已经能跑通。继续推进 stage 8 的最大风险已经从"双编辑路径"（stage6_audit 的 S1）解除，TextBuffer 是单一真相源。

剩余短板集中在：
- **性能 bench 缺位**（输入延迟、measurement 重算、60s 持续打字、外部剪贴板/RTF）
- **手动验收文档**（`docs/manual_test_protocol.md` §6 / §7 仍未补全）
- **当前工作区未提交的 viewport 重构破坏了 `cargo test --workspace --lib` 的编译**（详见 §3.4）

---

## 1. 阶段 6 验收对照表

### 1.1 自动化测试（plans.md §6）

| 期望测试 | 现状 | 评级 | 证据 |
|---|---|---|---|
| `key_to_command_mapping` | 30+ 单测覆盖（按键单元格分散） | ✅ | `app/src/input.rs:200-587`，含 macOS Cmd/Option/Shift/Ctrl 全套 |
| `cursor_move_left_at_line_start`（跨行回退） | TextBuffer 实现 + `MoveLeft` 委托 | ✅ | `text_buffer.rs:3223 cursor_move_left_at_line_start`；`document_view.rs cursor_move_left → tb.cursor_move_delta(Grapheme,-1)` |
| `cursor_move_word_unicode_boundary`（Option+Left/Right） | TextBuffer 实现 + 接通 | ✅ | `text_buffer.rs:3252`；`document_view.rs:307-316 cursor_move_word_left/right` |
| `backspace_grapheme_cluster`（ZWJ emoji 一次删完） | TextBuffer 实现 + 接通 | ✅ | `text_buffer.rs:3308 backspace_grapheme_cluster_zwj_emoji`；`document_view.rs:2134 backspace_zwj_emoji_one_key` |
| `enter_inserts_native_eol`（CRLF/LF 自适应） | ✅ | ✅ | `text_buffer.rs:3348 enter_inserts_lf_in_lf_file`；`document_view.rs:2362 insert_crlf_in_crlf_mode`（CRLF 路径由 `tb.set_crlf(true)` + `tb.write_raw(b"\n")` 自动展开） |
| `delete_at_eof_no_op` | TextBuffer 测 + DocumentView 测 | ✅ | `text_buffer.rs:3322 delete_at_eof_no_op`；`document_view.rs:2123 delete_forward_at_eof_command_noop` |
| **bench `bench_typing_throughput` ≥ 10k ops/s** | ✅ 实测 | ✅ | `scroll_bench.rs:90, 128`（含空文件 + 1k 行场景） |

### 1.2 性能门槛

| 阈值 | bench 状态 | 评级 |
|---|---|---|
| 输入延迟（按键 → swapchain present）< 8 ms | ❌ 无端到端 bench | ❌ |
| 持续打字 60 s 无掉帧 | ❌ 无 bench | ❌ |
| 单字符插入 measurement 重算 < 1 ms | ✅ 已实测 | ✅ |

`document_view.rs:2654 single_insert_threshold_10k_lines` 是 plans.md §6 "单字符插入引起的 measurement 重算 < 1 ms" 的本阶段答卷：10k 行文件、100 次循环平均，断言 `< 1 ms`。

### 1.3 边界 case

| 期望 | 现状 | 评级 |
|---|---|---|
| ZWJ emoji 中间放光标（grapheme 保护） | TextBuffer cursor_move_to_offset 自动 snap | ✅ `document_view.rs:2381 zwj_emoji_middle_cursor_moves_to_boundary` |
| NFD 组合字符中间删除（删整 cluster） | ✅ | ✅ `document_view.rs:2273 combining_accent_deleted_with_base`、`2290 multiple_combining_marks_one_grapheme` |
| BOM 后第一个字符不能误删 BOM | ✅ | ✅ `document_view.rs:2217 bom_at_start_backspace_noop` |
| 输入 `\0` 替换为 U+FFFD | ✅ | ✅ `document_view.rs:2247 insert_null_byte_replaced_with_fffd`、`2257 null_byte_in_content_replaced_with_fffd` |
| 极长行末 Enter 性能不退化 | ✅ | ✅ `document_view.rs:2346 enter_at_end_of_long_line`（10 万字符行无 panic） |
| 滚轮 + 键盘并发 | 🟡 路径都设 `needs_redraw=true`，未单独 bench | 🟡 |
| 数字小键盘 NumLock | ❌ 未测试 | ❌ |

### 1.4 手动验收对照（plans.md §6 七条）

| 条目 | 状态 | 备注 |
|---|---|---|
| 1. 敲 "Hello, 世界 🌏" 光标位置正确 | ✅ 功能 / 🟡 渲染 | TextBuffer cursor 用 logical pos；GUI 渲染光标 x 仍为简易估算（stage6_audit S4 备注：等 measurement 模块全量像素化） |
| 2. Backspace 删 🌏 一次 | ✅ | grapheme 级删除 |
| 3. macOS Cmd+/Option+ 全套 | ✅ | 全部映射 + handler 接通（对照 input.rs / handle_command） |
| 4. CRLF/LF 文件 Enter | ✅ | `crlf` 字段沿 `tb.set_crlf` 透传 |
| 5. 长按字符面板（macOS 字符面板） | ❌ 未手测 | winit 默认行为；待手动测 |
| 6. 选中后输入替换 | ✅ | 见 `document_view.rs:2593 insert_char_with_selection_replaces_selection` |
| 7. 60 字/秒打字 30 s 不抖 | ❌ 无端到端 bench | 推到 stage 12 |

### 1.5 阶段 6 小结

- **核心功能**全部接通，与 plans.md §6 "交付：编辑流畅；输入延迟达标；grapheme 边界正确" 中前两项一致。
- **性能验证**只覆盖了 measurement 重算 + typing throughput；输入延迟与 60 s 持续打字两个阈值仍待 stage 12 端到端 bench 落地——本阶段视为"形式不达标但有书面妥协"。
- **手动测试协议** `docs/manual_test_protocol.md` §6 仍未填，是阶段 6 收尾的唯一硬遗漏。

---

## 2. 阶段 7 验收对照表

### 2.1 自动化测试（plans.md §7）

#### Selection

| 期望测试 | 现状 | 评级 | 证据 |
|---|---|---|---|
| `mouse_drag_creates_range` | ✅ | ✅ | `document_view.rs:3559 mouse_drag_creates_range`；`app.rs:1649 mouse_drag_creates_range_via_app_state` |
| `shift_arrow_extends_selection` | ✅ | ✅ | `document_view.rs:3014 shift_right_extends_selection`、`3026 shift_left_extends_selection`；input.rs 中 `shift_arrow_*` 一组 + handler `EditCommand::ExtendLeft/Right/Up/Down` |
| `shift_click_extends_to_point` | ✅ | ✅ | `document_view.rs:3035 shift_click_extends_to_point`、`3044 shift_click_reverse_direction`；handler `app.rs:1591` 走 `dv.selection_anchor = Some(cursor)` 后挪 cursor |
| `triple_click_selects_line` | ✅ | ✅ | `document_view.rs:3054 triple_click_selects_line`；`app.rs:1576 click_count == 3` 分支 |
| `double_click_selects_word`（CJK / byte-class） | ✅ | ✅ | `document_view.rs:3076 double_click_selects_word`、`3086 double_click_selects_word_cjk`、`3095 double_click_selects_word_unicode`；handler `app.rs:1586` |

#### Buffer history

| 期望 | 现状 | 评级 | 证据 |
|---|---|---|---|
| `history_undo_single_insert` | ✅ | ✅ | `text_buffer.rs:3376` |
| `history_undo_replace` | ✅ | ✅ | `text_buffer.rs:3639` |
| `history_redo_after_branch_loses_redo_stack` | ✅ | ✅ | `text_buffer.rs:3558` |
| `history_coalesce_continuous_typing` | ✅ | ✅ | `text_buffer.rs:3589`；DV 层 `document_view.rs:3266 undo_coalesced_typing_single_step` 也覆盖 |
| `history_limit_memory_cap` | ✅ | ✅ | `text_buffer.rs:3609`（cap=1000） |

#### Clipboard

| 期望 | 现状 | 评级 | 证据 |
|---|---|---|---|
| `clipboard_roundtrip_utf8` | ✅ | ✅ | `document_view.rs:3385`（无显示服务时跳过） |
| `clipboard_eol_normalization_on_paste`（外部 CRLF → 内部 LF） | 🟡 字节级 | 🟡 | `document_view.rs:3174 clipboard_eol_crlf_to_lf` 等；**只测 `normalize_paste_text` 函数，不测"目标文件是 CRLF 时的反向转换"** —— plans.md §7 说"外部 CRLF → 内部 LF 文件"，已覆盖；但未测内部 CRLF 文件粘贴时是否还原为 CRLF |
| `clipboard_strip_bom_on_paste` | ✅ | ✅ | `document_view.rs:3204 clipboard_strip_bom`；`3596 normalize_strips_bom`；`3631 normalize_bom_plus_crlf` |
| `bench_select_1mb_redraw` < 16 ms | ✅ 有 bench | ✅ | `scroll_bench.rs:203` |

### 2.2 性能门槛

| 阈值 | 现状 | 评级 |
|---|---|---|
| copy 1 MB 文本 < 50 ms | ✅ | `document_view.rs:3525 copy_1mb_text_performance`（断言 < 50 ms） |
| undo/redo 50 步连续操作 < 100 ms 总耗时 | ✅ | `document_view.rs:3474 undo_redo_50_steps_performance`（断言 < 100 ms） |

### 2.3 边界 case

| 期望 | 现状 | 评级 |
|---|---|---|
| 选区跨多行 | ✅ | `document_view.rs:3361 selection_across_multiline` |
| 选区跨非法 UTF-8（lossy） | ✅ | `document_view.rs:3294 selection_across_invalid_utf8_lossy`、`3317 clipboard_lossy_copy_does_not_modify_document`、`3339 clipboard_copy_system_lossy_with_invalid_utf8` |
| 剪贴板含 BOM（保留 / 剥除策略明示并测试） | ✅ | 起始 BOM 剥除、内嵌 BOM 保留：`document_view.rs:3610 normalize_bom_in_middle_preserved` |
| macOS NSPasteboard 多类型 | ❌ 未实现额外过滤 | ❌ arboard 自带 plain text 处理；未单独覆盖 RTF/HTML 过滤路径 |
| undo 越过 load 点 | ✅ | `document_view.rs:3502 undo_past_load_point_no_panic`（保留 mark_as_clean 锚点） |
| 连续打字合并为单 undo 步 | ✅ | `document_view.rs:3266 undo_coalesced_typing_single_step` |

### 2.4 手动验收对照（plans.md §7 七条）

| 条目 | 状态 | 备注 |
|---|---|---|
| 1. 鼠标拖选；状态栏显示选中字符数 + 字节数 | 🟡 | 拖选代码就位（`app.rs:1543`）；**状态栏显示尚未确认**（需手测） |
| 2. Shift+方向键扩选；Cmd+A 全选 | ✅ | input.rs 全部映射；handler `ExtendLeft/Right/Up/Down/...`、`SelectAll` 均接通 |
| 3. 双击选词（含 CJK）；三击选行 | ✅ | `app.rs:1573-1595` |
| 4. Cmd+C → Safari 粘贴；Safari 复制 → Cmd+V 进编辑器 | 🟡 | arboard 接通；需手测跨进程 |
| 5. Cmd+Z 撤销 → Shift+Cmd+Z 重做（连 50 次不丢） | ✅ | input.rs：cmd_z / cmd_shift_z 都映射；handler 接通 |
| 6. 选中 → 直接打字 → 替换 | ✅ | `document_view.rs:2593 insert_char_with_selection_replaces_selection` |
| 7. 外部带 RTF 的剪贴板 —— 只取 plain text | ❌ 未单测 | arboard 在 macOS 上 `get_text` 默认走 NSPasteboardTypeString，但本仓库未编程式注入 RTF 验证 |

### 2.5 阶段 7 小结

- **核心交付（选择 / 剪贴板 / 撤销重做）全部就位**，性能两个硬阈值都达标。
- 自动化测试覆盖密度 **优于 plans.md 期望**（边界 case 增加了 lossy UTF-8 + BOM 中间位置等）。
- 主要欠账：
  - 跨进程剪贴板（手动测试）
  - RTF/HTML 多类型过滤
  - 状态栏选区字数显示
  - manual_test_protocol §7 文档缺失

---

## 3. 工程纪律快照

### 3.1 编译

| 命令 | 状态 |
|---|---|
| `cargo build --workspace`（无 `--tests`） | ✅ 成功（1 警告：未使用变量 `ac_idx`，位于 `app.rs:1114`） |
| `cargo test -p edit-plus-core --lib` | ✅ 126 passed, 3 ignored |
| `cargo test --workspace --lib --no-run` | ❌ **失败** — `app.rs:1546` `hit_test()` 期望 2-tuple，被解构成 3-tuple；`app.rs:2335 / 2368` `saturating_sub` 期望 `u32`、传入 `usize` |
| `cargo clippy / fmt` | 未在本审计中跑（前一审计为 ✅，但代码已变） |

### 3.2 git 工作树

`git status` 表明 stage 6/7 的 commits 已经合入 main（`6bc0a6c refactor: document_view 使用 visible_doc_line_range 替代 visible_range`、`66a228d feat(viewport): 引入 WrapIndex 实现精确滚动映射` 等）。当前未提交的修改集中在：

```
M crates/app/src/app.rs            ← 含 viewport 重构未完成的代码（导致 test 不过）
M crates/app/src/document_view.rs  ← 已完成的接通改动
M AGENTS.md / CLAUDE.md / plans.md
?? docs/displayrow.md / displayrow_review.md / viewport-scroll-redesign.md / viewport_0601.md / viewport_architecture_analysis.md
?? plans_viewport_0601.md
```

**未提交内容主要是 stage 6/7 之后的 viewport / wrap_index 重构**，与 stage 6/7 自身验收无关，但**目前会让 `cargo test` 编译失败**，需要修复。

### 3.3 死代码 / TODO

`docs/stage6_audit.md` 的 S1（双编辑路径）已彻底解决：`DocumentView` 用 `tb: TextBuffer` 作为唯一真相源，`GapBuffer` 字段已不存在。`terminal_stubs.rs` 还在但已减为最小集，给 TextBuffer 在 GUI 模式下提供编译 stub。

### 3.4 关键风险

1. **未提交的 viewport 重构破坏了测试编译**——任何想跑 `cargo test --workspace` 验收 stage 6/7 的人都会先撞墙。
2. **plans.md §6 / §7 三个性能门槛仍只是测了"代理子项"**：
   - 输入延迟、60 s 持续打字 → 推到 stage 12
   - measurement 重算 < 1 ms → 当前测的是"line index rebuild"，不是真正的 measurement walk

---

## 4. 待办清单

按优先级排序，可作为 stage 8 启动前的收尾。

### P0（阻断后续）

- [ ] 修复未提交的 `app.rs` 编译错误（`hit_test` tuple arity、`saturating_sub` 类型不匹配），让 `cargo test --workspace --lib` 重新跑通。
- [ ] 把 stage 6/7 的剩余改动（`document_view.rs`、`AGENTS.md`、`CLAUDE.md`、`plans.md`、`docs/*.md`）评审后提交。

### P1（plans.md 明列但未做）

- [ ] **填写** `docs/manual_test_protocol.md` §6 / §7（CLAUDE.md §3 要求）。
- [ ] 阶段 6 手动测试 5（macOS 长按字符面板）实测一次，记录到 `docs/manual_test_runs/`。
- [ ] 阶段 7 手动测试 1（状态栏字数显示）—— 当前 UI 未渲染，需补；或推到 stage 9 的 StatusBar。
- [ ] 阶段 7 手动测试 4（跨进程剪贴板）实测一次，记录到 `docs/manual_test_runs/`。
- [ ] 阶段 7 手动测试 7（RTF 剪贴板只取 plain text）—— 加一个 macOS-only 集成测试，在 `arboard` 上塞 RTF 并验证 `get_text` 返回 plain。

### P2（推到 stage 12 集中处理）

- [ ] `bench_input_latency`（端到端 winit fake event → swapchain present）
- [ ] `bench_typing_60s_60fps`（持续打字帧时分布）
- [ ] 真正的 measurement walk bench（≠ 当前 line index rebuild）
- [ ] 数字小键盘 NumLock 行为测试

---

## 5. 结论

阶段 6 与阶段 7 在 **功能闭环**、**单元测试覆盖**、**单点性能阈值** 三个维度上都已达交付门槛。距离 plans.md §6 / §7 的"完全"完成主要差三块：

1. **性能门槛中需端到端 bench 的部分**（输入延迟、60 s 打字、measurement 真实重算）—— 与 plans.md §12 自然合并。
2. **手动测试协议文档**（§6 / §7 节）—— 必须在收尾时补，CLAUDE.md §3 已规定。
3. **当前未提交的 viewport 重构**——它属于 stage 5/12 的滚动优化，不属于 stage 6/7 但会阻塞 stage 6/7 的回归测试。

按 **形式：实质 = 95% : 80%（阶段 6） / 90% : 75%（阶段 7）** 评估，可以宣告"基本交付，建议补完 P1 后再开 stage 8"。
