# 大文件结构分析与拆分优化方案

## 1. 现状概览

按行数排序的主要大文件（不含测试、生成代码、auto-gen 表格）：

| 文件 | 行数 | 职责 |
|------|------|------|
| `crates/app/src/app.rs` | 2916 | **核心应用壳** — 窗口初始化、事件循环、渲染、命令分发 |
| `crates/ui/src/tab_bar.rs` | 1714 | Tab 栏布局、绘制、命中测试、弹出菜单 |
| `crates/app/src/commands.rs` | 1361 | 编辑命令分发（含约 900 行测试） |
| `crates/app/src/document_view/mod.rs` | 1178 | 文档视图核心（光标、选区、viewport） |
| `crates/app/src/render_pipeline.rs` | 1172 | 可见行 shaping、文本顶点生成 |
| `crates/core/src/icu.rs` | 1026 | ICU 行断分析（第三方绑定） |
| `crates/lsh/src/runtime.rs` | 1028 | LSH 运行时 |
| `crates/app/src/file_history.rs` | 885 | 文件历史 + 持久化 |
| `crates/core/src/buffer/edit.rs` | 833 | 缓冲区编辑操作 |
| `crates/app/src/workspace.rs` | 817 | 工作区 + 多标签管理 |
| `crates/shaping/src/lib.rs` | 767 | 字体 shaping |
| `crates/render/src/lib.rs` | 758 | 渲染管线基础 |
| `crates/app/src/snap_tree.rs` | 660 | DisplayLineMap 树结构 |
| `crates/ui/src/scrollbar.rs` | 654 | 滚动条 |

---

## 2. 重点分析：app.rs（2916 行）

### 2.1 当前结构

`app.rs` 当前承担了太多职责，违反了 AGENTS.md 中定义的"薄壳"原则。按逻辑块拆解如下：

| 行范围 | 职责 | 行数 | 问题 |
|--------|------|------|------|
| 1–45 | imports + 常量 | 45 | 正常 |
| 49–65 | `compute_cursor_phase()` | 16 | ✅ 纯函数，但放在 app.rs 里不合适 |
| 80–143 | `struct App` 定义 | 63 | 字段过多（30+ 字段），很多是渲染/reshape 细节 |
| 146–210 | `App::new` | 64 | OK |
| 214–268 | `quit_app` / `record_*_history` / `save_history` | 55 | **应移至 file_history 或 workspace** |
| 299–402 | `handle_workspace_effect` / `update_tab_layout` / `init_display_map` | 100 | ⚠️ `init_display_map` 有复杂的 hash 校验逻辑 |
| 405–629 | `open_file` / `execute_commands` 含 ZoomIn/Out/Reset | 225 | ⚠️ **Zoom 逻辑重复 3 次**，各约 25 行几乎相同 |
| 637–780 | `dispatch()` — AppAction 分发 | 143 | 过大，混入了 scroll、reshape 逻辑 |
| 805–887 | `popup_menu_text_vertices` | 82 | **纯渲染代码，不属于 app 壳** |
| 897–982 | `init_window` / `load_file` | 85 | OK |
| 1000–1045 | `resize` | 45 | OK |
| 1050–1097 | `move_cursor_visual` / `extend_selection_visual` | 47 | ⚠️ 构建 CursorContext 有 unsafe 指针 |
| 1098–1419 | **vertex 生成函数群** | 321 | ❌ **全部是渲染细节**：cursor_vertices, status_bar_*, gutter_bg_*, tab_text_vertices |
| 1430–1506 | `shape_visible_lines` / `drain_reshape_results` | 76 | 已部分委托给 render_pipeline |
| 1509–1600 | `submit_reshape_ahead` / `post_shape_update` | 91 | reshape worker 协调逻辑 |
| 1631–1982 | `render()` | 351 | ❌ **整个 GPU 提交逻辑**，含顶点组装、search/selection 高亮、wgpu pass |
| 1984–2046 | `handle_scroll` | 62 | ⚠️ 含 tab bar 水平滚动逻辑 |
| 2056–2120 | `perform_search_for_active_doc` | 65 | ❌ **搜索业务逻辑**，gap buffer 遍历 |
| 2122–2395 | `handle_command` | 274 | ❌ **最核心问题**：混合了命令分发 + 搜索栏拦截 + display_map 同步 + reshape 取消 |
| 2397–2642 | `ApplicationHandler` impl | 245 | ⚠️ `window_event` 仍有 IME commit 的 display_map 同步逻辑 |
| 2643–2916 | zoom_tests | 274 | 测试代码 |

### 2.2 核心问题

1. **渲染代码侵入**: 约 **750 行**（vertex 生成 + render() + popup_text）是 GPU/渲染细节，不应在 app 壳中
2. **Zoom 逻辑 3x 复制**: ZoomIn、ZoomOut、ZoomReset 各约 25 行几乎相同的代码
3. **handle_command 膨胀**: 274 行，混合了命令分发 + search 拦截 + display_map 同步 + reshape 取消
4. **search 逻辑侵入**: `perform_search_for_active_doc` 65 行 gap buffer 遍历放在 app 壳中
5. **reshape 协调散落**: reshape_generation 增减、pending_reshapes 清理在 5+ 个不同地方重复出现
6. **struct App 字段过多**: 30+ 字段，很多是渲染缓存（advance_cache、cluster_pool、first_line、last_line）

---

## 3. app.rs 拆分方案

### Phase 1: 提取渲染器（~750 行 → `app_renderer.rs`）

**目标**: 将所有 vertex 生成 + GPU 提交逻辑抽取到 `AppRenderer` 结构体。

新文件 `crates/app/src/app_renderer.rs`：

```rust
pub(crate) struct AppRenderer;

impl AppRenderer {
    /// 生成 popup 菜单的文本顶点
    pub fn popup_menu_text_vertices(...) -> Vec<GlyphVertex> { ... }
    
    /// 生成 tab 栏文本顶点
    pub fn tab_text_vertices(...) -> Vec<GlyphVertex> { ... }
    
    /// 生成状态栏背景和文本顶点
    pub fn status_bar_bg_vertices(...) -> Vec<GlyphVertex> { ... }
    pub fn status_bar_text_vertices(...) -> Vec<GlyphVertex> { ... }
    
    /// 生成 gutter 背景顶点
    pub fn gutter_bg_vertices(...) -> Vec<GlyphVertex> { ... }
    
    /// 生成光标顶点
    pub fn cursor_vertices(...) -> Vec<GlyphVertex> { ... }
    
    /// 组装所有顶点 + GPU 提交
    pub fn render(app: &mut App) -> Option<()> { ... }
}
```

**影响的函数**:
- `popup_menu_text_vertices` (L805-887)
- `tab_text_vertices` (L1191-1419)
- `status_bar_bg_vertices` (L1131-1151)
- `status_bar_text_vertices` (L1176-1189)
- `gutter_bg_vertices` (L1154-1174)
- `cursor_vertices` (L1098-1114)
- `render()` (L1633-1982)

---

### Phase 2: 统一 Zoom 逻辑（消除 3x 重复）

将 ZoomIn/ZoomOut/ZoomReset 统一为一个函数：

```rust
// crates/app/src/app.rs
fn apply_zoom(&mut self, new_font_size: f32) {
    Settings::get_mut().set_font_size(new_font_size);
    self.render_cache.invalidate_all();
    if let Some(ref mut text) = self.text {
        text.shaper.set_font_size(new_font_size);
    }
    if let Some(ref gpu) = self.gpu {
        let h = gpu.ctx.config.height as f32;
        let visible_rows = self.visible_rows(h);
        let viewport_height = self.visible_height_lines(h);
        for dv in &mut self.workspace.doc_views {
            dv.resize(visible_rows, viewport_height);
            dv.viewport.restore_scroll_from_anchor(&dv.display_map, Settings::get().line_height);
            dv.viewport.clamp_scroll_top(&dv.display_map, Settings::get().line_height);
        }
    }
    self.invalidate_reshape();
    self.needs_redraw = true;
}
```

然后 `execute_commands` 中：
```rust
AppCommand::ZoomIn  => self.apply_zoom(Settings::get().font_size + 1.0),
AppCommand::ZoomOut => self.apply_zoom((Settings::get().font_size - 1.0).max(6.0)),
AppCommand::ZoomReset => self.apply_zoom(15.0),
```

**预计减少**: ~50 行

---

### Phase 3: 提取 reshape 协调（~150 行 → 强化 `reshape_worker.rs`）

将 reshape_generation 增减、pending_reshapes 管理统一封装：

```rust
// 新增到 app.rs
fn invalidate_reshape(&mut self) {
    self.reshape_generation += 1;
    self.pending_reshapes.clear();
    if let Some(ref w) = self.reshape_worker {
        w.cancel_before(self.reshape_generation);
    }
}
```

当前 `reshape_generation += 1; pending_reshapes.clear(); cancel_before()` 这个三步组合在 app.rs 中出现了 **7 次**（handle_workspace_effect、ZoomIn、ZoomOut、ZoomReset、resize、handle_command、IME commit），全部替换为 `self.invalidate_reshape()`。

**预计减少**: ~35 行 + 消除重复

---

### Phase 4: 搜索逻辑外迁（~130 行 → `search_state.rs`）

将 `perform_search_for_active_doc` 的 gap buffer 搜索逻辑移入 `search_state.rs`：

```rust
// crates/app/src/search_state.rs
impl SearchState {
    /// 在 gap buffer 中执行全文搜索
    pub fn perform_search(&mut self, gap: &GapBuffer) { ... }
}
```

同时，`handle_command` 中的搜索栏命令拦截（InsertChar 到 search query、Backspace、Enter、Tab）应提取为独立函数：

```rust
fn handle_search_bar_input(&mut self, cmd: &EditCommand) -> bool {
    // 返回 true 表示已消费该命令，不再传递给编辑器
}
```

**预计减少**: ~130 行

---

### Phase 5: handle_command 简化

经过 Phase 2-4 后，`handle_command` 应从 274 行缩减到 ~80 行，主要结构：

```rust
fn handle_command(&mut self, cmd: EditCommand, event_loop: &ActiveEventLoop) {
    // 1. Escape 处理（5 行）
    // 2. 搜索栏拦截（1 行：if self.handle_search_bar_input(&cmd) { return; }）
    // 3. 特殊命令匹配（MoveUp/Down → visual cursor, Save, Tab 切换等）（30 行）
    // 4. 通用编辑命令执行 + display_map 同步（30 行）
    // 5. reshape 失效（1 行：self.invalidate_reshape()）
}
```

---

### Phase 6: IME commit 逻辑去重

`window_event` 中 `Ime::Commit` 的处理（L2457-2505）与 `handle_command` 的编辑后处理几乎完全相同（display_map sync + reshape cancel + cursor blink reset）。应提取为公共方法：

```rust
fn post_edit_update(&mut self, outcome: &EditOutcome) {
    // display_map 同步
    // render_cache 失效
    // reshape 失效
    // cursor 状态更新
}
```

---

### Phase 7: render 相关文件整合

app crate 中有 4 个 render 相关文件，需要分析各自的依赖关系和正确归属。

#### 依赖链现状

```
render crate (纯 GPU 基础设施)
  │  GlyphAtlas, GlyphRenderer, GlyphVertex, WGSL shader
  │  依赖: wgpu, hashlink, bytemuck
  │  不依赖: ui, core, app
  │
  ├─ gpu.rs           (171 行) ─ GpuContext, create_gpu_context, MSAA
  │    依赖: wgpu, winit
  │    不依赖: ui, core, render, app 的任何其他模块
  │
  ├─ render_state.rs  (130 行) ─ GpuState, TextState
  │    依赖: gpu.rs, render::*, shaping::*, wgpu
  │    不依赖: ui, core, DocumentView
  │
  ├─ render_cache.rs  (361 行) ─ GlyphInstance, CachedLine, RenderCache
  │    依赖: render::GlyphVertex, ui::theme, core::highlight, hashlink
  │    不依赖: DocumentView, Workspace, app 状态
  │
  └─ render_pipeline.rs (1173 行) ─ shape_visible_lines, status/search_bar_text_vertices
       依赖: ⭐ 几乎所有 app 内部类型
       DocumentView, TextState, GpuState, RenderCache, cursor_motion,
       ui::gutter, ui::decorations, ui::layout, ui::settings,
       core::highlight, render::GlyphKey, render::GlyphRenderer
```

#### 分析结论

| 文件 | 行数 | 能迁到 render crate? | 原因 |
|------|------|-------------------|------|
| `gpu.rs` | 171 | ✅ **可以** | 纯 wgpu 初始化，零业务依赖 |
| `render_state.rs` | 130 | ✅ **可以** | 只依赖 gpu.rs + render + shaping，跟 gpu.rs 一起迁 |
| `render_cache.rs` | 361 | ⚠️ **有条件** | 依赖 `ui::theme` 和 `core::highlight` 做高亮色查询，需 render 新增对 ui/core 的依赖 |
| `render_pipeline.rs` | 1173 | ❌ **不能** | 重度依赖 `DocumentView`, `cursor_motion` 等 app 内部类型，会产生循环依赖 |

#### 推荐方案

**策略 A（推荐）：将 `gpu.rs` + `render_state.rs` 迁入 render crate**

render crate 已经依赖 wgpu 和 shaping，这两个文件恰好属于这个层次。迁移后 render crate 的职责变为「所有 GPU 资源管理」：

```
crates/render/src/
  ├─ lib.rs         (758 行) ← GlyphAtlas, GlyphRenderer, GlyphVertex, shader
  ├─ gpu.rs         (171 行) ← GpuContext, GpuError, create_gpu_context [NEW]
  └─ render_state.rs(130 行) ← GpuState, TextState [NEW]
```

render crate 的 Cargo.toml 需新增对 `winit` 的依赖（`GpuState` 使用了 `PhysicalSize`）。

**`render_cache.rs` 和 `render_pipeline.rs` 保留在 app crate**，因为：
- `render_cache.rs` 的 `emit_vertices_for_visual_line()` 需要 `ui::theme::Theme` 做高亮色查询，如果迁到 render 就需要 render 依赖 ui，这会产生 `render → ui → render` 循环依赖
- `render_pipeline.rs` 是 app 级的“连接层”，它把 DocumentView 的数据 → shaping → atlas → 顶点组装，这是 app 的职责

**`render_pipeline.rs` 重命名为 `text_shaping_pass.rs`**，更准确地描述其职责（“可见行的文本 shaping 和顶点生成”），避免与 render crate 混淆。

#### 迁移后的引用变更

```rust
// before (in app.rs)
use crate::gpu::{self, GpuContext, GpuError};
pub(crate) use crate::render_state::{GpuState, TextState};

// after
use render::gpu::{self, GpuContext, GpuError};
pub(crate) use render::render_state::{GpuState, TextState};
```

app 已经依赖 `render`，所以不会新增依赖。

---

## 4. 其他大文件分析

### 4.1 tab_bar.rs（1714 行）

**问题**:
- 混合了**三个独立关注点**: Tab 布局计算、顶点生成、弹出菜单
- `PopupMenu` + `ContextMenu` + `OverflowMenu` 逻辑混在一起

**拆分方案**:
| 新文件 | 职责 | 预估行数 |
|--------|------|----------|
| `ui::tab_bar::layout` | `layout_tabs`, `TabLayout`, `TabEntry`, hit-test | ~500 |
| `ui::tab_bar::render` | `tab_bar_vertices`, `tab_bar_text_positions` | ~500 |
| `ui::tab_bar::popup_menu` | `PopupMenu`, `PopupMenuAction`, `ContextMenuAction`, 菜单顶点 | ~400 |
| `ui::tab_bar` (mod.rs) | re-export + `TabInfo`, `TabBarCtx` | ~50 |

---

### 4.2 commands.rs（1361 行）

**分析**: 实际业务代码约 450 行，测试约 900 行。

**建议**:
- 将测试移至 `crates/app/src/commands_tests.rs`（或 `tests/` 目录）
- 业务代码本身的 450 行分发逻辑合理，不需要进一步拆分

---

### 4.2.1 input.rs（631 行）

**分析**: `EditCommand` 枚举（36 变体）+ `key_to_command` 映射。测试占 ~65%。

**建议**: 结构清晰，不需要拆分。测试较多但都是简单断言，可保持原位。

---

### 4.3 render_pipeline.rs（1172 行）

**分析**: 已经是从 app.rs 提取出的模块，职责单一（可见行 shaping + 顶点生成）。

**问题**:
- ❌ **Glyph atlas 查找 + 光栅化 + 纹理上传的管线代码重复了 3 次**（主文本 L697-750、状态栏 L952-996、搜索栏 L1104-1148），每次约 50 行，几乎完全相同
- ❌ **行号渲染代码重复 4 次**（L38-55、L204-210、L377-398、L639-660）
- ⚠️ `font_id_usize` 的 hash 计算通过 `DefaultHasher` 重复了 4 次

**建议**:
- 提取 `fn rasterize_and_upload_glyph(...)` 公共函数，消除 3x 重复
- 提取 `fn render_line_number(...)` 公共函数，消除 4x 重复  
- 将 `search_bar_text_vertices` 和 `status_bar_text_vertices` 移至 `app_renderer.rs`
- 核心的 `shape_visible_lines` 保留

---

### 4.4 document_view/mod.rs（1178 行）

**分析**: 职责已经比较清晰（文档视图的光标、选区、viewport 管理）。

**问题**:
- ⚠️ 存在 3 对 deprecated 方法仍未清理：`visible_line()` / `visible_line_wrap()`、`visible_line_key()` / `visible_line_key_wrap()`、`visible_line_count()` / `visible_line_count_wrap()`
- ⚠️ Buffer 读取循环 (`while result.len() < length && i < total { chunk = read_forward... }`) 出现 **4 次**，应提取为 `read_contiguous_bytes(offset, length)` 辅助函数
- `extend_selection_*` 系列方法遵循重复模式：`ensure_selection_active()` + 计算偏移 + `set_cursor_offset_synced()`

**建议**:
- 清理 deprecated 方法对，统一为 wrap 版本
- 提取 `read_contiguous_bytes()` 辅助函数
- 将 viewport/scroll 方法提取到 `document_view/viewport_ops.rs`，约 200 行
- 将 clipboard 方法提取到 `document_view/clipboard.rs`，约 100 行

---

### 4.5 workspace.rs（817 行）

**分析**: 结构合理，管理多标签页 + 快照持久化。

**问题**:
- ⚠️ `TabSnapshot` 结构体定义了两次（L369 用于序列化、L432 用于反序列化），可以共用
- ⚠️ `TabInfo` 构造在 `update_tab_layout()` 中重复了 2 次（L622-634 和 L664-676）
- ⚠️ `close_others()` / `close_right()` / `close_all()` 遵循相同模式（收集索引、倒序删除）

**建议**: 
- 将快照持久化（`save_snapshot` / `load_snapshot`）提取到 `workspace_persist.rs`，约 200 行
- 统一 `TabSnapshot` 定义
- 提取 `close_tabs(indices: &[usize])` 公共方法消除 close_* 系列重复

---

### 4.6 file_history.rs（885 行）

**分析**: 文件历史记录 + 磁盘序列化/反序列化。实际业务代码约 200 行，测试约 670 行（占 75%）。

**⚠️ Bug 发现**:
> [!CAUTION]
> 从 L619 开始有一个缺失的 `}` 闭合括号，导致 L755-886 的测试函数是 **完全重复的副本**（疑似合并冲突遗留）：
> `test_save_load_with_excluded_dirs`、`test_get_by_workspace_skips_nonexistent`、`test_record_batch_dedup_same_path` 等至少 6 个测试被完整复制了一遍。

**建议**: 
- 修复 L619 的 `}` 闭合问题并删除重复的测试
- 考虑将序列化格式（JSON 读写）提取到单独的 persistence 层
- 当前大小可接受，非紧急

---

## 5. 代码重复模式总结

### 5.1 Reshape 失效三步曲（出现 7 次）
```rust
self.reshape_generation += 1;
self.pending_reshapes.clear();
if let Some(ref w) = self.reshape_worker {
    w.cancel_before(self.reshape_generation);
}
```
→ 统一为 `self.invalidate_reshape()`

### 5.2 Zoom 逻辑（出现 3 次，各 ~25 行）
→ 统一为 `self.apply_zoom(new_size)`

### 5.3 屏幕尺寸获取（出现 15+ 次）
```rust
let screen_w = self.gpu.as_ref().map(|g| g.ctx.config.width as f32).unwrap_or(800.0);
let screen_h = self.gpu.as_ref().map(|g| g.ctx.config.height as f32).unwrap_or(600.0);
```
→ 统一为 `self.screen_size() -> (f32, f32)`

### 5.4 visible_rows + viewport_height 获取（出现 8+ 次）
```rust
let visible_rows = self.visible_rows(h);
let viewport_height = self.visible_height_lines(h);
```
→ 可以打包为 `self.viewport_metrics() -> ViewportMetrics`

### 5.5 编辑后的 display_map 同步（出现 2 次：handle_command、IME commit）
→ 统一为 `self.post_edit_update(&outcome)`

### 5.6 NDC 坐标转换（tab_bar.rs 中出现 30+ 次）
```rust
let ndc_x = px / screen_w * 2.0 - 1.0;
let ndc_y = 1.0 - py / screen_h * 2.0;
```
→ 统一为 `TabBarCtx::px_to_ndc(px, py) -> (f32, f32)`

### 5.7 Glyph 管线（render_pipeline.rs 中重复 3 次）
 atlas 查找 → 光栅化 → 纹理上传 → 顶点发射，每次约 50 行
→ 统一为 `fn shape_and_emit_text_vertices(...)`

### 5.8 DPI 缩放（tab_bar.rs 中出现 40+ 次）
```rust
* Settings::get().dpi_scale
```
→ 在 `TabBarCtx` 中缓存 `dpi_scale` 字段

---

## 6. 实施优先级

### P0（最高优先级，解决核心膨胀问题）

| 阶段 | 内容 | 减少行数 | 改动文件数 |
|------|------|----------|-----------|
| Phase 2 | 统一 Zoom 逻辑 | ~50 | 1 (app.rs) |
| Phase 3 | reshape 失效封装 | ~35 | 1 (app.rs) |
| Phase 5 | 屏幕尺寸 / viewport metrics 公共方法 | ~30 | 1 (app.rs) |

> **P0 估计：app.rs 从 2916 行减到 ~2800 行，消除 7 处重复**

### P1（显著改善，推荐做）

| 阶段 | 内容 | 减少行数 | 改动文件数 |
|------|------|----------|-----------|
| Phase 1 | 渲染器提取 | ~750 | 2 (app.rs → app_renderer.rs) |
| Phase 4 | 搜索逻辑外迁 | ~130 | 2 (app.rs → search_state.rs) |
| Phase 6 | IME/编辑后处理去重 | ~40 | 1 (app.rs) |

> **P0+P1 估计：app.rs 从 2916 行减到 ~1850 行**

### P2（进一步优化，可后续做）

| 阶段 | 内容 | 减少行数 | 改动文件数 |
|------|------|----------|-----------|
| tab_bar 拆分 | layout / render / popup | 0（总量不变，结构改善） | 4 |
| commands 测试分离 | 测试移至独立文件 | ~900 | 2 |
| document_view 拆分 | viewport_ops + clipboard | ~300 | 3 |
| workspace 持久化分离 | workspace_persist.rs | ~200 | 2 |

---

## 7. 拆分后的目标架构

### crate 层次依赖

```
crates/render/ (纯 GPU 资源管理)
  ├── lib.rs             (758 行) ← GlyphAtlas, GlyphRenderer, GlyphVertex, shader
  ├── gpu.rs             (171 行) ← GpuContext, GpuError, create_gpu_context [FROM app]
  └── render_state.rs    (130 行) ← GpuState, TextState [FROM app]

crates/ui/ (纯 UI 组件，不依赖 app)
  └── … 不变

crates/app/ (应用层)
  ├── app.rs             (~1200 行) ← 纯壳：初始化、事件分发、状态管理
  ├── app_renderer.rs    (~800 行)  ← vertex 生成 + GPU 提交
  ├── text_shaping_pass.rs (~900 行) ← 可见行 shaping（原 render_pipeline.rs 重命名）
  ├── render_cache.rs    (361 行)  ← 行级渲染缓存（保留，因依赖 ui::theme）
  ├── actions.rs         (93 行)   ← AppAction enum
  ├── commands.rs        (~450 行) ← 编辑命令执行
  ├── commands_tests.rs  (~900 行) ← 编辑命令测试
  ├── events.rs          (336 行)  ← 事件处理
  ├── input.rs           (630 行)  ← 按键映射
  ├── menu_handler.rs    (81 行)   ← 菜单命令映射
  ├── search_state.rs    (~200 行) ← 搜索状态 + gap buffer 搜索
  ├── workspace.rs       (~600 行) ← 多标签管理
  ├── workspace_persist.rs (~200 行) ← 快照持久化
  └── …
```

### app.rs 的目标职责

1. `struct App` 定义和初始化
2. `ApplicationHandler` 实现（事件循环薄壳）
3. 通过 `dispatch()` 转发 actions
4. 通过 `execute_commands()` 转发 menu commands
5. `apply_zoom()` / `invalidate_reshape()` 等状态管理原子方法

**app.rs 不应包含**：
- ❌ 顶点生成（→ app_renderer.rs）
- ❌ GPU 提交（→ app_renderer.rs）
- ❌ GPU 初始化（→ render::gpu）
- ❌ 搜索逻辑（→ search_state.rs）
- ❌ 复杂的编辑后同步（→ 提取为公共方法）

---

## 8. 风险评估

| 风险 | 等级 | 缓解措施 |
|------|------|----------|
| 大量代码移动导致 merge 冲突 | 中 | 按 Phase 逐步提交，每阶段独立 PR |
| 渲染器提取后的借用冲突 | 中 | 使用函数参数传递而非结构体方法 |
| 测试移动后路径变化 | 低 | 保持 `#[cfg(test)] mod` 结构 |
| pub(crate) 可见性调整 | 低 | 拆分后适当调整字段可见性 |

---

## 9. 决策待确认

1. **Phase 1 的渲染器形式**: 用自由函数（`fn render(app: &mut App)`）还是独立 struct（`AppRenderer`）？
   - 推荐：自由函数，因为渲染需要访问 App 几乎所有字段，独立 struct 会造成拆借问题
2. **commands_tests 放哪**: 同文件 `#[cfg(test)]`、同目录独立文件、还是顶层 `tests/`？
   - 推荐：同目录 `commands_tests.rs`，用 `#[path]` 或内联 mod
3. **tab_bar 拆分是否跨模块**: 目前 tab_bar 是单文件，拆成目录会影响 `use` 路径
   - 推荐：转为 `ui::tab_bar/` 目录 + mod.rs re-export，对外接口不变
4. **render_cache.rs 是否迁到 render crate**: 如果抽取 `highlight_kind_to_color` 为 callback/trait，就能解除对 ui::theme 的依赖，但增加复杂性
   - 推荐：保留在 app，不值得为了迁移而引入抽象
5. **render_pipeline.rs 是否重命名**: 当前名称容易与 render crate 混淆
   - 推荐：重命名为 `text_shaping_pass.rs`，明确表达其职责
