# Scrollbar 交互模块 实施计划

> **Spec:** `docs/superpowers/specs/2026-06-03-scrollbar-interaction-design.md`

**Goal:** 新建 `scrollbar.rs` 模块，从 `app.rs` 拆分滚动条渲染和交互逻辑。

**Architecture:** `ScrollbarState`（状态）、`ScrollbarLayout`（NDC几何）、hit-test + drag 交互。滚动比例基于 `WrapIndex::total_display_rows()`。

---

### Task 1: 创建 scrollbar.rs 模块
**Files:** Create `crates/app/src/scrollbar.rs`

完整模块代码见 spec 第 3 节。关键类型：
- `ScrollbarState` — hovered, dragging, drag_start 字段
- `ScrollbarLayout` — NDC 几何（bar_left/right/wide, thumb_top/bottom）
- `ScrollbarHit` — TrackAbove/Below/Thumb/None  
- `ScrollbarAction` — PageUp/Down/StartDrag/ScrollTo/None
- `compute_layout()` — DPI感知，用 total_display_rows
- `hit_test()` — NDC 命中判定
- `generate_vertices()` — hover时用 bar_right_wide
- `handle_mouse_move/down/drag/up()` — 交互处理

含完整 `#[cfg(test)]` 测试（layout/hit/drag/vertices/mouse 共 25+ 个）。

### Task 2: 添加滚动条颜色到 Theme
**Files:** Modify `crates/app/src/theme.rs`

在 `Theme` struct 添加 `scrollbar_track` 和 `scrollbar_thumb: [f32; 4]`。
`dark()` → `[0.18,0.18,0.20,0.5]` / `[0.35,0.35,0.40,0.7]`
`light()` → `[0.85,0.85,0.85,0.5]` / `[0.60,0.60,0.60,0.7]`

### Task 3: 注册模块
**Files:** Modify `crates/app/src/lib.rs`

添加 `pub mod scrollbar;`

### Task 4: 集成到 app.rs
**Files:** Modify `crates/app/src/app.rs`

1. 删除 `push_quad_verts` 自由函数 (L149-158)
2. 删除 `scrollbar_vertices()` 方法 (L1401-1442)
3. App struct 添加 `scrollbar: ScrollbarState` 字段
4. `new()` 中初始化 `scrollbar: ScrollbarState::new()`
5. render() 中用 `scrollbar::compute_layout()` + `scrollbar::generate_vertices()` 替换旧调用
6. CursorMoved: 文本 hit-test 前检查 scrollbar hover
7. MouseInput(Left, pressed): 文本 click 前检查 scrollbar click（PageUp/Down/StartDrag）
8. CursorMoved: 拖拽中更新 scroll_top
9. MouseInput(Left, released): 结束拖拽

### Task 5: 验证
Run: `cargo test -p edit-plus-app --lib` — 全部通过
Run: `cargo check -p edit-plus-app` — 编译通过
