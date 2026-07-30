# 大文件拆分执行方案

目标：把 `app.rs`（1634 行代码）和 `document_view.rs`（1186 行代码 + 2456 行测试）拆成职责单一、< 800 行的子模块；保持行为零回归、可逐阶段独立验收。

不动：`text_buffer.rs` / `unicode/measurement.rs` / `icu.rs` —— 全部为 vendor 文件，按 plans.md §7 抄码 checklist 标记为 `y`（直抄不改）。

---

## 0. 设计原则与约束

### 0.1 不变量

每个阶段结束时，下列断言必须为真：

1. `cargo build --workspace` 0 警告 0 错误
2. `cargo test --workspace` 全绿（在阶段 0 修通编译之后）
3. `cargo clippy --workspace --all-targets -- -D warnings` 通过
4. `cargo fmt --check` 通过
5. 公共 API 行为不变（`App::new` / `DocumentView::new` / 全部 `pub fn` 签名等价）

### 0.2 边界

- **不改业务逻辑**：本计划仅做"把现有代码原地搬家"+ 抽小函数。任何"顺手优化一下"都不在本计划内，遇到要单独立 issue。
- **不引入新 crate**：所有拆分都在 `crates/app/src/` 之内做。把代码挪到 `crates/ui/` 是 stage 9 的事，本次预留接口、不抢跑。
- **不动 vendor 文件**：`text_buffer.rs` / `measurement.rs` / `icu.rs` 一行不改。
- **不动测试断言**：测试可以挪文件，但断言、辅助函数原样保留。

### 0.3 文件目标尺寸

| 类型 | 上限 |
|---|---|
| 模块代码（无测试） | < 800 行 |
| 单文件含测试 | < 1500 行 |
| 单 `impl` 块 | < 500 行 |

---

## 1. 当前状态盘点

### 1.1 `app.rs`（2372 行 = 1634 代码 + 738 测试）

```
1-19    use 声明
21-32   struct AdvanceCacheEntry        // 几何缓存条目
35-42   struct GpuState                 // wgpu 子状态
43-57   struct TextState                // shaping/render 子状态
58-134  pub struct App                  // 主结构
136-152 fn byte_to_x                    // 自由函数：渲染 helper
154-219 fn compute_selection_highlight_quads // 自由函数：选区几何
221-1439 impl App {                     // 1218 行的巨型 impl
        ├─ 223-269   生命周期 / 访问器（new / is_running / window_title）
        ├─ 270-405   初始化 + 文件 IO（init_window / init_text / load_file / resize）
        ├─ 428-468   hit_test
        ├─ 469-665   move_cursor_visual（197 行，纯几何 + 滚动）
        ├─ 666-744   selection_vertices / cursor_vertices
        ├─ 746-903   status_bar_*（3 个方法 + 状态栏字数缓存）
        ├─ 905-1299  shape_visible_lines（395 行，最大单方法）
        ├─ 1300-1407 render
        ├─ 1408-1417 scroll_by_visual_lines
        └─ 1418-1439 handle_scroll
1440-1445 impl Default for App
1446-1492 impl App { fn handle_command }       // 单方法的 impl
1492-1631 impl ApplicationHandler for App      // winit 回调
1635-2372 #[cfg(test)] mod tests               // 738 行
```

### 1.2 `document_view.rs`（3642 行 = 1186 代码 + 2456 测试）

```
1-852   pub struct DocumentView + impl  // 主体；其中：
        ├─ 42-113    构造（new / from_file）
        ├─ 115-227   read 路径（visible_line / visible_lines / line_byte_offset / 等）
        ├─ 228-246   viewport 透传（scroll / resize / set_crlf）
        ├─ 248-401   编辑代理（insert/delete/cursor_move_*）
        ├─ 402-490   剪贴板（extract / paste / copy / cut）
        ├─ 491-642   selection 扩展（extend_*）
        ├─ 645-655   undo / redo
        └─ 657-851   sync_after_edit / line_index 维护
854-1090 fn execute_edit_command         // 232 行的派发表
1092-1163 fn rebuild_line_index_from_tb  // 自由函数
1166-1184 fn replace_null_bytes
1187-3641 #[cfg(test)] 七组 mod          // 2456 行测试
2976-2998 fn normalize_paste_text         // 在测试块中间夹一个 pub 函数（怪味）
```

### 1.3 待解决的预先工作

`git status` 表明 `app.rs` 有未提交修改，且 `cargo test --workspace --lib --no-run` 因为 `hit_test()` arity 与 `saturating_sub` 类型不匹配而**编译失败**。本计划必须先消化这些，否则无法逐阶段回归。

---

## 2. 阶段切分

每阶段独立、可单独 commit、能跑通 build + clippy + test。每阶段约 1–3 小时。

### 阶段 0：先把测试编译跑通（前置，0.5 h）

**为什么先做**：后续每阶段都需要 `cargo test --workspace` 作为回归门槛。当前测试不过 = 没法验收。

**改动文件**：`crates/app/src/app.rs`

**具体动作**：
1. 修 `app.rs:1546` —— `hit_test` 当前签名返回 `Option<(usize, usize)>`，但调用处期望 `Option<(usize, usize, usize)>`。两条路：
   - **Option A**：把 `hit_test` 返回值改回 2-tuple，调用处去掉第 3 个解构变量
   - **Option B**：把 `hit_test` 改成返回 3-tuple（offset, doc_line, vis_row），调用处保持
   - 选择标准：看主分支 commit `66a228d feat(viewport): 引入 WrapIndex` 之前的 hit_test 形态，与之保持一致
2. 修 `app.rs:2335` / `2368` —— `first_visible_row().saturating_sub(first_visible_dr)`：当前 `first_visible_dr` 是 `usize`，`saturating_sub` 期望 `u32`。改成 `as_u32()` 包一层即可（`viewport.rs` 的 DisplayRow API 已支持）。

**验收**：
- `cargo test --workspace --lib --no-run` 成功
- `cargo test --workspace` 全绿
- 提交一个独立 commit：`fix: restore test build after viewport refactor`

---

### 阶段 1：app.rs —— 抽出纯几何 helper（1 h）

**目的**：`byte_to_x` / `compute_selection_highlight_quads` / `AdvanceCacheEntry` 已经是自由函数 + 数据类型，没有 `&self`，可以无缝挪走。

**新文件**：`crates/app/src/render_geom.rs`

**搬入内容**：
- `struct AdvanceCacheEntry`（app.rs:21-32）
- `fn byte_to_x`（app.rs:136-152）
- `fn compute_selection_highlight_quads`（app.rs:154-219）

**对应的测试**（在 `mod tests` 中匹配的几个）：
- `selection_quads_*`（约 14 个测试，app.rs:1922-2172 范围内）
- `selection_quads_does_not_highlight_adjacent_wrapped_line`
- `make_cache_entry`（辅助函数）

**接口要求**：
- 全部对外 `pub(crate)`
- `AdvanceCacheEntry` 字段保持现有可见性（按需 `pub(crate)`）

**验收**：
- `cargo build` 通过
- `cargo test -p edit-plus-app` 全绿
- `app.rs` 减少 ~80 代码行 + ~250 测试行
- 新 `render_geom.rs` ~330 行（含测试）

---

### 阶段 2：app.rs —— 抽出 StatusBar 模块（1.5 h）

**目的**：状态栏是相对独立的视图组件，3 个方法 + 一组状态栏字数缓存字段。

**新文件**：`crates/app/src/status_bar.rs`

**设计接口**：

```rust
// status_bar.rs
pub(crate) struct StatusBarCache {
    selection_anchor: Option<usize>,
    selection_cursor: usize,
    char_count: usize,
    byte_count: usize,
}

impl StatusBarCache {
    pub fn new() -> Self;
    pub fn invalidate(&mut self);
    pub fn ensure_fresh(&mut self, dv: &DocumentView);
}

pub(crate) fn build_text(...) -> String;
pub(crate) fn build_bg_vertices(...) -> Vec<GlyphVertex>;
pub(crate) fn build_text_vertices(...) -> Vec<GlyphVertex>;
```

**搬入内容**：
- `App::status_bar_text`（app.rs:746-776）
- `App::status_bar_bg_vertices`（app.rs:777-798）
- `App::status_bar_text_vertices`（app.rs:799-903）
- `App` 中状态栏专用字段（4 个 `selection_status_*` 缓存字段）打包成 `StatusBarCache`

**`App` 中保留**：`status_cache: StatusBarCache` 一个字段；render 路径调 `status_bar::build_*`。

**对应测试**：
- `status_bar_caches_selection_counts`
- `status_bar_cache_invalidated_on_selection_change`
- `status_bar_cache_cleared_when_no_selection`

**验收**：
- `cargo build` + `cargo test` 全绿
- `app.rs` 减少 ~160 代码行 + ~80 测试行
- 新 `status_bar.rs` ~250 行

---

### 阶段 3：app.rs —— 抽出 Hit-test + 鼠标处理（2 h）

**目的**：hit_test 是纯几何函数，与鼠标输入分发紧耦合；放一起最自然。

**新文件**：`crates/app/src/mouse.rs`

**设计接口**：

```rust
// mouse.rs
pub(crate) struct MouseState {
    pub pos: (f64, f64),
    pub is_down: bool,
    pub down_offset: Option<usize>,
    pub last_click_time: Instant,
    pub click_count: u8,
}

impl MouseState {
    pub fn new() -> Self;
}

/// Pure: pixel coords → (byte offset, doc_line). Does not depend on App.
pub(crate) fn hit_test(
    px: f32,
    py: f32,
    dv: &DocumentView,
    text: &TextState,
    layout: &LayoutMetrics,
) -> Option<(usize, usize)>;

/// Mutates DocumentView + MouseState according to mouse event.
pub(crate) fn handle_mouse_input(
    state: ElementState,
    mouse: &mut MouseState,
    dv: &mut DocumentView,
    modifiers: ModifiersState,
    layout_hit: Option<(usize, usize)>,
) -> bool;  // returns needs_redraw

/// Handle CursorMoved while button held.
pub(crate) fn handle_cursor_moved(
    pos: (f64, f64),
    mouse: &mut MouseState,
    dv: &mut DocumentView,
    layout_hit: Option<(usize, usize)>,
) -> bool;
```

**搬入内容**：
- `App::hit_test`（app.rs:428-468）
- `window_event` 中 `MouseInput` / `CursorMoved` 的整段逻辑（app.rs:1540-1610）
- `App` 中鼠标专用字段（`mouse_pos / is_mouse_down / mouse_down_offset / last_click_time / click_count`）打包成 `MouseState`

**`App` 中保留**：`mouse: MouseState`；`window_event` 改为薄壳：

```rust
WindowEvent::MouseInput { state, .. } => {
    let hit = mouse::hit_test(self.mouse.pos.0 as f32, self.mouse.pos.1 as f32,
                              self.doc_view.as_ref().unwrap(), &self.text, &self.layout);
    if mouse::handle_mouse_input(state, &mut self.mouse, dv, self.modifiers, hit) {
        self.needs_redraw = true;
    }
}
```

**对应测试**：
- `mouse_drag_creates_range_via_app_state`
- `mouse_drag_backward_creates_range`
- `mouse_click_without_drag_no_selection`
- `mouse_drag_then_click_clears_selection`

**验收**：
- `cargo build` + `cargo test` 全绿
- `app.rs` 减少 ~120 代码行 + ~150 测试行
- 新 `mouse.rs` ~300 行

---

### 阶段 4：app.rs —— 抽出 shape_visible_lines + 渲染管线（2.5 h）

**目的**：`shape_visible_lines`（395 行）是 app.rs 最庞大的单方法；它做的事属于"渲染管线第一阶段"，不属于 App 生命周期。

**新文件**：`crates/app/src/render_pipeline.rs`

**设计接口**：

```rust
// render_pipeline.rs
pub(crate) struct ShapeOutput {
    pub vertices: Vec<GlyphVertex>,
    pub clusters_per_line: Vec<Vec<(usize, f32)>>,
    pub doc_line_map: Vec<usize>,  // visual row → doc line
    // 等等：把当前 App 字段中"shape 的输出"全列出来
}

pub(crate) fn shape_visible_lines(
    dv: &mut DocumentView,
    text: &mut TextState,
    wrap_index: &mut WrapIndex,
    layout: &LayoutMetrics,
    advance_cache: &mut HashMap<...>,
) -> ShapeOutput;

pub(crate) fn cursor_vertices(
    dv: &DocumentView,
    shape: &ShapeOutput,
    layout: &LayoutMetrics,
    blink_visible: bool,
) -> Vec<GlyphVertex>;

pub(crate) fn selection_vertices(
    dv: &DocumentView,
    shape: &ShapeOutput,
    layout: &LayoutMetrics,
) -> Vec<GlyphVertex>;
```

**风险与缓解**：
- `shape_visible_lines` 当前直接读写多个 `&mut self.*` 字段。要抽成自由函数，必须先把这些字段**显式列出**作为参数。建议**先做小步重构**：
  - **步骤 4a**：把 `shape_visible_lines` 内的逻辑拆成 3-4 个小私有方法（仍在 `impl App` 中），各自只用必要字段——commit
  - **步骤 4b**：再把这些小方法的签名改成自由函数 + 显式参数——commit
  - **步骤 4c**：搬到 `render_pipeline.rs`——commit
- 每小步都可单独验收，避免一次大爆炸式重构。

**搬入内容**：
- `App::shape_visible_lines`（app.rs:905-1299）
- `App::selection_vertices`（app.rs:666-698）
- `App::cursor_vertices`（app.rs:699-744）

**对应测试**：
- `cursor_vertices_empty_when_visual_line_is_max`
- `move_up_into_skipped_area_moves_cursor_byte`
- `move_down_*` 系列（与 shape 输出耦合）

**验收**：
- `cargo build` + `cargo test` 全绿
- `app.rs` 减少 ~430 代码行
- 新 `render_pipeline.rs` ~500 行

---

### 阶段 5：app.rs —— 抽出 cursor_movement helper（1 h）

**目的**：`move_cursor_visual`（197 行）+ `scroll_by_visual_lines`，是与 wrap_index/viewport 紧耦合的纯计算。

**新文件**：`crates/app/src/cursor_motion.rs`

**设计接口**：

```rust
pub(crate) fn move_cursor_visual(
    delta: isize,
    dv: &mut DocumentView,
    wrap_index: &WrapIndex,
    sticky_x: &mut f32,
) -> bool;  // returns moved

pub(crate) fn scroll_by_visual_lines(
    delta: isize,
    dv: &mut DocumentView,
    wrap_index: &WrapIndex,
);
```

**搬入内容**：
- `App::move_cursor_visual`（app.rs:469-665）
- `App::scroll_by_visual_lines`（app.rs:1408-1417）

**对应测试**：移动光标的几个测试。

**验收**：`app.rs` 减少 ~210 代码行；新 `cursor_motion.rs` ~250 行。

---

### 阶段 6：app.rs —— 收尾整理（0.5 h）

**目的**：清理 app.rs 剩下的内容，确认最终形态。

**操作**：
- 把 `impl Default for App`（app.rs:1440-1445）和 `impl App { fn handle_command }`（app.rs:1446-1492）合并到主 `impl App` 块
- `use` 语句重新整理
- `App` 结构体定义中字段按"生命周期 / 子状态打包 / 输入状态"分组用注释隔开

**预期 app.rs 终态**：
- 总行数：~600 代码 + ~100 测试 = ~700 行
- 内容：`App` 结构、4 个 lifecycle 方法（new / init_window / init_text / load_file / resize）、`render` 主入口、`handle_command`、`window_event`/`about_to_wait`

**验收**：
- 总行数 ≤ 800
- 单 `impl App` 块 ≤ 400 行
- `cargo test` 全绿

---

### 阶段 7：document_view.rs —— 抽出命令派发表（1.5 h）

**目的**：`execute_edit_command`（document_view.rs:854-1090）是 232 行的 match 派发表，跟 DocumentView 的"数据 + 状态"是两件事。

**新文件**：`crates/app/src/commands.rs`

**搬入内容**：
- `pub fn execute_edit_command`
- 与之相关的 helper（`indent_column_offset` 中"判断是否在 indent" 的部分逻辑可以保留在 DocumentView，命令 handler 调即可）

**接口**：
```rust
// commands.rs
pub fn execute_edit_command(cmd: &EditCommand, dv: &mut DocumentView) -> bool;
```

**搬出注意**：
- DocumentView 中 `last_command_was_home: bool` 字段保持不动（被 commands.rs 读写）
- 测试 `mod command_tests`（document_view.rs:1700-2208）整块挪到 `commands.rs` 的 `#[cfg(test)] mod tests`

**验收**：
- `cargo test` 全绿
- `document_view.rs` 减少 ~230 代码行 + ~510 测试行
- 新 `commands.rs` ~750 行（其中 510 行测试）

---

### 阶段 8：document_view.rs —— 抽出 line_index 子模块（1.5 h）

**目的**：line_offsets / line_lengths / 增量重建逻辑（`sync_after_edit_*` / `rebuild_line_index_from_tb` / `rescan_lines_from`）是有边界的算法包，可以独立测试。

**新文件**：`crates/app/src/line_index.rs`

**设计接口**：

```rust
// line_index.rs
pub(crate) struct LineIndex {
    offsets: Vec<usize>,
    lengths: Vec<usize>,
}

impl LineIndex {
    pub fn rebuild_from(tb: &TextBuffer) -> Self;
    pub fn line_offset(&self, doc_line: usize) -> Option<usize>;
    pub fn line_length(&self, doc_line: usize) -> Option<usize>;
    pub fn line_count(&self) -> usize;
    pub fn line_for_offset(&self, offset: usize) -> usize;

    /// O(1) 路径：单字符插入/删除（无换行）
    pub fn shift_after(&mut self, edit_pos: usize, delta: isize, cursor_line: usize);

    /// 增量重扫：从 edit_pos 所在行开始重新扫描到末尾
    pub fn rescan_from(&mut self, tb: &TextBuffer, start: usize);
}
```

**搬入内容**：
- `DocumentView::line_offsets / line_lengths` 字段 → `LineIndex { offsets, lengths }`
- `rebuild_line_index_from_tb` → `LineIndex::rebuild_from`
- `DocumentView::sync_after_edit_incremental / sync_after_edit_full / rescan_lines_from` 大部分逻辑

**DocumentView 改造**：
- 字段：`line_index: LineIndex`
- 调用：`self.line_offsets[i]` → `self.line_index.line_offset(i).unwrap()`

**风险**：
- `sync_after_edit_*` 中混入了 `dirty / cursor_offset / viewport` 同步——这些**必须留在 DocumentView**，不进 LineIndex
- `LineIndex` 只负责数组维护；DocumentView 调 `LineIndex::shift_after` / `rescan_from` 之后自己处理 viewport 同步

**对应测试**：
- `incremental_update_*`（document_view.rs:2430-2470）—— 测的是 LineIndex 行为
- `rebuild_line_index_*` 间接测试

**验收**：
- `cargo test` 全绿，特别是 `single_insert_threshold_10k_lines` 性能测试不退化
- `document_view.rs` 减少 ~200 代码行
- 新 `line_index.rs` ~250 行

---

### 阶段 9：document_view.rs —— 测试外迁（1 h）

**目的**：document_view.rs 测试占 2/3，且已按 7 个 mod 分组——直接挪到 `tests/`。

**新文件**：
- `crates/app/tests/dv_command.rs`（mod command_tests）—— 已在阶段 7 移入 `commands.rs`，跳过
- `crates/app/tests/dv_boundary.rs`（mod boundary_tests）
- `crates/app/tests/dv_perf.rs`（mod perf_tests）
- `crates/app/tests/dv_cursor_visual.rs`（mod cursor_visual_tests）
- `crates/app/tests/dv_stage7.rs`（mod stage7_tests）
- `crates/app/tests/dv_b11.rs`（mod b11_tests）

**注意**：
- 集成测试只能调 `pub` API；如果原测试用了 `pub(crate)` / 私有字段，需要：
  - 要么在 DocumentView 上加 `#[cfg(test)] pub fn test_helper_*`
  - 要么测试留 `mod tests`（in-source 测试），仅做"分文件"而非"挪到 tests/"
- **保守策略**：先挪到 `crates/app/src/document_view/tests/`（in-module 子目录），保留 `pub(crate)` 访问；后续再考虑改 `tests/`

**预期 document_view.rs 终态**：
- 主体代码：~700 行
- 测试代码：< 200 行（保留少数核心 smoke test）

**验收**：
- `cargo test` 全绿
- document_view.rs 总行数 < 1000

---

### 阶段 10：normalize_paste_text 归位（0.2 h）

**位置怪味**：`document_view.rs:2976` 这个 `pub fn` 夹在测试块中间（`mod stage7_tests` 上方），是历史 hack。

**操作**：把 `normalize_paste_text` 移到主代码区（`fn replace_null_bytes` 旁边），或并入 `commands.rs` 的 paste 路径。

**验收**：build + test 全绿。

---

## 3. 阶段依赖关系

```
阶段 0 (修测试)
   ↓
阶段 1 (geom helpers) ─┐
阶段 2 (status_bar)    ├─ 这三个互相独立，可并行
阶段 3 (mouse)         ─┘
   ↓
阶段 4 (render_pipeline)   ← 依赖 1（用 byte_to_x、selection_quads）
   ↓
阶段 5 (cursor_motion)
   ↓
阶段 6 (app.rs 收尾)
   ─────────────────────
阶段 7 (commands.rs)       ← 不依赖前 6 个，可与阶段 1-6 并行
   ↓
阶段 8 (line_index)        ← 依赖 7（commands 中也调到 line index）
   ↓
阶段 9 (测试外迁)
   ↓
阶段 10 (normalize_paste_text 归位)
```

**单人推进**：按 0→1→2→3→4→5→6→7→8→9→10 顺序。

**两人并行**：A 做 0、1-6；B 做 7-10。会冲突的只有 `document_view.rs` 与 `app.rs` 的 use 声明。

---

## 4. 终态目标

| 文件 | 当前 | 目标 |
|---|---|---|
| `app.rs` | 2372 (1634 代码) | < 800 |
| `document_view.rs` | 3642 (1186 代码) | < 1000 |
| `render_geom.rs`（新） | — | ~330 |
| `status_bar.rs`（新） | — | ~250 |
| `mouse.rs`（新） | — | ~300 |
| `render_pipeline.rs`（新） | — | ~500 |
| `cursor_motion.rs`（新） | — | ~250 |
| `commands.rs`（新） | — | ~750（其中 ~510 测试） |
| `line_index.rs`（新） | — | ~250 |

总代码量基本不变（搬家而非新增）；最大文件从 1634 降到 < 800；单 `impl` 块从 1218 行降到 < 400 行。

---

## 5. 验收清单

每阶段都要走一遍：

- [ ] `cargo build --workspace` 无警告
- [ ] `cargo test --workspace` 全绿（含 doctest）
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 通过
- [ ] `cargo fmt --check` 通过
- [ ] `git diff` 复审：本阶段动了哪些文件、新增了哪些文件、是否有意外的代码变更
- [ ] commit 消息按 `refactor(app): extract <module> from app.rs` 格式
- [ ] 该阶段说明的"减少行数"与实际接近（±20%）

最终：
- [ ] `app.rs` < 800 行
- [ ] `document_view.rs` < 1000 行
- [ ] 所有新文件 < 800 行（含测试 < 1500）
- [ ] 没有任何 `text_buffer.rs` / `measurement.rs` / `icu.rs` 的改动

---

## 6. 风险与对冲

| 风险 | 缓解 |
|---|---|
| 阶段 4（shape_visible_lines）耦合复杂，一次搬不动 | 拆 4a/4b/4c 三小步；每步都先 commit |
| 抽函数后参数列表过长 | 把相关字段先打包成 `LayoutMetrics` / `RenderState` 等 newtype，参数 ≤ 5 个 |
| 集成测试访问私有字段 | 阶段 9 先用"in-module 子目录"过渡，不强求 `tests/` |
| 测试名重复（不同 mod 都有 `select_all_*`） | 挪文件后用 mod 名 + 文件名前缀避免冲突 |
| 跨 commit 编译过、单 commit 不过 | 每阶段验收的"4 项门槛"必须在该 commit 上单独跑通，不靠下一 commit 修 |
| 拆分过程中阶段 11+ plans 改动 | 本计划只动 app.rs / document_view.rs；plans.md 的 stage 11 不受影响 |

---

## 7. 不在本计划内（明确拒绝范围蔓延）

- 不引入新 crate（`crates/ui/` 留给 stage 9）
- 不修业务 bug（哪怕在搬家时看见也得记 issue 单独修）
- 不改任何 `pub` API 签名
- 不动 vendor 文件（`text_buffer.rs` / `measurement.rs` / `icu.rs` / `lsh/*`）
- 不重写测试断言
- 不"顺手" rename（变量名、函数名一律保持）

如本计划执行过程中发现"不重构没法继续"的硬阻塞，立刻停下并把该阻塞写成 issue，由用户决定是否扩展范围。
