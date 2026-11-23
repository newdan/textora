# P2-8: DisplayLineMap per-DocumentView 实施方案

> 日期：2026-06-09
> 状态：方案阶段，待确认后实施

---

## 一、现状

### 架构

```
App
├── display_map: DisplayLineMap         ← 全局单例，所有 dv 共享
├── reshape_worker: Option<ReshapeWorker>
├── reshape_generation: u64
└── workspace: Workspace
    └── doc_views: Vec<DocumentView>
        ├── [0] DocumentView { viewport, tb, line_index, ... }  ← 无 display_map
        └── [1] DocumentView { ... }   ← （分屏时存在，罕见）
```

### 问题

- 不同 DocumentView 可能有不同 viewport 宽度 → 同一 doc_line 需要不同的 visual_line_count
- 当前共享 `display_map` 只能存一份，无法处理多视口场景
- 所有 `DocumentView` 方法（`visible_line_wrap`、`ensure_cursor_visible` 等）都把 `DisplayLineMap` 作为参数传入，耦合冗余

### 调用关系

```
app.rs 中对 display_map 的引用 (~30 处)：
├── self.display_map                    ← App 级别持有/修改（drain_reshape_results, init_display_map）
├── &self.display_map                   ← 传递给 dv 方法、viewport、render_pipeline
└── dv.viewport.*(&self.display_map)    ← Viewport 方法需要 display_map
```

---

## 二、目标架构

```
App
├── reshape_worker: Option<ReshapeWorker>
├── reshape_generation: u64
└── workspace: Workspace
    └── doc_views: Vec<DocumentView>
        ├── [0] DocumentView { display_map: DisplayLineMap, viewport, tb, ... }
        └── [1] DocumentView { display_map: DisplayLineMap, ... }
```

### 关键变化

1. `DisplayLineMap` 从 `App` 移到 `DocumentView`
2. `DocumentView` 方法不再接收 `&DisplayLineMap` 参数，直接用 `&self.display_map`
3. `Viewport` 方法不再接收 `&DisplayLineMap` 参数，由调用方提供
4. Reshape worker 结果按 `dv_idx` 路由到对应 `DocumentView`

---

## 三、实施步骤

### 阶段 1：DocumentView 增加 display_map 字段（结构迁移）

**文件**：`crates/app/src/document_view/mod.rs`

1. 在 `DocumentView` 结构体增加：
   ```rust
   pub display_map: DisplayLineMap,
   ```

2. `DocumentView::new()` 中初始化：
   ```rust
   display_map: DisplayLineMap::new(),
   ```

3. 移除方法签名中的 `wi: &DisplayLineMap` 参数，改用 `&self.display_map`：
   - `visible_line_wrap(vis_idx, wi)` → `visible_line_wrap(vis_idx)`
   - `visible_line_count_wrap(vis_idx, wi)` → `visible_line_count_wrap(vis_idx)`
   - `visible_line_key_wrap(vis_idx, wi)` → `visible_line_key_wrap(vis_idx)`
   - `ensure_cursor_visible(display_map, line_height)` → `ensure_cursor_visible(line_height)`
   - `page_up(display_map, line_height)` → `page_up(line_height)`
   - `page_down(display_map, line_height)` → `page_down(line_height)`

4. `Viewport` 结构体的 `restore_scroll_from_anchor`、`clamp_scroll_top` 等方法也需要 `&DisplayLineMap`。有两种选择：
   - **A**：改为由调用方传入（Viewport 保持纯净）
   - **B**：Viewport 存储 display_map 引用（增加生命周期复杂性）
   
   **推荐 A**。Viewport 保持数据结构角色。

5. 更新所有调用方（`app.rs`、`cursor_motion.rs`、测试）

**影响文件**：
- `crates/app/src/document_view/mod.rs`
- `crates/app/src/document_view/test_*.rs`（多处）
- `crates/app/src/cursor_motion.rs`
- `crates/app/src/app.rs`

### 阶段 2：App 层适配

**文件**：`crates/app/src/app.rs`

1. 移除 `App` 的 `display_map` 字段
2. `init_display_map` 改为操作 `dv.display_map`：
   ```rust
   // 改前
   fn init_display_map(&mut self, dv_idx: usize) {
       if let Some(dv) = self.workspace.doc_views.get(dv_idx) {
           // build entries from dv.line_index
           self.display_map.set_entries(entries);
       }
   }
   // 改后
   fn init_display_map_for_dv(&mut self, dv_idx: usize) {
       if let Some(dv) = self.workspace.doc_views.get_mut(dv_idx) {
           dv.display_map.set_entries(entries);
       }
   }
   ```

3. 所有 `&self.display_map` → 通过 active dv 获取：
   ```rust
   // 改前
   dv.viewport.clamp_scroll_top(&self.display_map, ...);
   // 改后
   let map = &dv.display_map;
   dv.viewport.clamp_scroll_top(map, ...);
   ```
   
   注意：需要先 borrow dv mutably 还是 immutably？`clamp_scroll_top` 需要 `&mut self`，`display_map` 需要 `&self`。Rust 不允许同时借用。需要先 clone 或分两步。

   **策略**：先将 `display_map` 不可变引用取出，再可变借用 `dv.viewport`：
   ```rust
   // 在循环中先收集 display_map 引用
   let maps: Vec<&DisplayLineMap> = self.workspace.doc_views.iter()
       .map(|dv| &dv.display_map).collect();
   for (dv, map) in self.workspace.doc_views.iter_mut().zip(maps.iter()) {
       dv.viewport.clamp_scroll_top(map, ...);
   }
   ```

   或者更简单：Viewport 方法改为接收 `total_rows: usize` 而不是 `&DisplayLineMap`（因为只需要 total_rows）。这样可以避免借用的复杂性。

4. `render()` 中：
   ```rust
   // 改前
   shape_visible_lines(..., &mut self.display_map, ...);
   // 改后
   let dv = &mut self.workspace.doc_views[self.workspace.active_index];
   shape_visible_lines(..., &mut dv.display_map, ...);
   ```

5. `drain_reshape_results`：
   - 当前遍历 `results` 并写入 `self.display_map`
   - 改为：需要知道 result 属于哪个 dv
   - **方案**：reshape result 增加 `dv_idx: usize` 字段，或先只支持 active dv

### 阶段 3：Reshape worker 适配

**文件**：`crates/app/src/reshape_worker.rs`

当前 `ReshapeRequest`：
```rust
pub struct ReshapeRequest {
    pub doc_line: usize,
    pub text: String,
    pub viewport_width: f32,
    pub font_size: f32,
    pub char_width: f32,
    pub font_family: String,
}
```

**方案 A（简单）**：增加 `dv_idx: usize` 字段
```rust
pub struct ReshapeRequest {
    pub dv_idx: usize,  // ← 新增
    ...
}
```

`ReshapeResult` 同理增加 `dv_idx`。

`drain_reshape_results` 中按 `dv_idx` 路由。

**方案 B（更简单）**：暂时只对 active dv 做 reshape

当前所有 reshape 请求都是针对 active dv 发出的。可以限制 worker 结果只写入 active dv 的 display_map。切换 dv 时 cancel + re-submit。

**推荐方案 A**，因为 reshape 已有 generation 机制，加 dv_idx 是自然扩展。

### 阶段 4：清理和测试

1. 移除 `DocumentView` 方法中所有 `wi: &DisplayLineMap` 参数
2. 更新所有测试（主要在 `test_*.rs` 文件中）
3. `cargo check` 编译通过
4. `cargo test` 全部通过
5. `cargo clippy` 无新增警告

---

## 四、借用问题重点分析

### 问题

`app.rs` 中常见模式：
```rust
for dv in &mut self.workspace.doc_views {
    dv.viewport.clamp_scroll_top(&self.display_map, ...);
    //                              ^^^^^^^^^^^^^^^^^ immutable borrow of App
    //                    ^^^^^^^^ mutable borrow of dv
}
```

改为 per-dv 后：
```rust
for dv in &mut self.workspace.doc_views {
    dv.viewport.clamp_scroll_top(&dv.display_map, ...);
    //  ❌ cannot borrow dv as immutable because it is also borrowed as mutable
}
```

### 解决方案

**方案 1**：Viewport 方法不接收 `&DisplayLineMap`，改为接收 `total_rows: usize`

`clamp_scroll_top` 当前：
```rust
pub fn clamp_scroll_top(&mut self, display_map: &DisplayLineMap, line_height: f32)
```
它只需要 `display_map.total_rows()` 来计算边界。改为：
```rust
pub fn clamp_scroll_top(&mut self, total_rows: usize, line_height: f32)
```

`restore_scroll_from_anchor` 同理，它也只需要 `total_rows`。

需要 `&DisplayLineMap` 的方法是 `visible_doc_line_range`（需要 tree 查询）。这些可以从 dv 不可变引用获取，不影响可变借用 dv.viewport。

**实施**：
1. `clamp_scroll_top` + `restore_scroll_from_anchor` → 参数改为 `total_rows: usize`
2. 调用方先取 `let total_rows = dv.display_map.total_rows();` 再传
3. 其他需要 tree 查询的方法，保持 `&DisplayLineMap` 参数

### 具体改动

在 loop 中：
```rust
// 收集 total_rows
let total_rows: Vec<usize> = self.workspace.doc_views.iter()
    .map(|dv| dv.display_map.total_rows())
    .collect();

for (dv, rows) in self.workspace.doc_views.iter_mut().zip(total_rows.iter()) {
    dv.viewport.restore_scroll_from_anchor(*rows, line_height);
    dv.viewport.clamp_scroll_top(*rows, line_height);
}
```

`shape_visible_lines` 调用时：
```rust
// 先取 active dv
let dv = &mut self.workspace.doc_views[self.workspace.active_index];
let map = &mut dv.display_map;
// 传给 shape_visible_lines(&mut map)
// 但 dv 需要同时作为参数传给 shape_visible_lines...
```

`shape_visible_lines` 签名：
```rust
pub fn shape_visible_lines(
    ...
    dv: &mut DocumentView,
    ...
    display_map: &mut DisplayLineMap,
    ...
)
```

改为从 dv 内部取：
```rust
pub fn shape_visible_lines(
    ...
    dv: &mut DocumentView,  // contains display_map
    ...
)
// 内部用 dv.display_map
```

这也需要改 `shape_visible_lines` 签名。

---

## 五、影响范围统计

| 文件 | 改动类型 | 预计行数 |
|------|----------|----------|
| `document_view/mod.rs` | 加字段 + 改方法签名 + 改实现 | ~30 行 |
| `document_view/test_*.rs` | 更新测试调用 | ~50 行 |
| `app.rs` | 移除 display_map 字段 + 改所有引用 | ~80 行 |
| `render_pipeline.rs` | shape_visible_lines 签名 + 内部 | ~15 行 |
| `cursor_motion.rs` | display_map 参数改为从 dv 取 | ~10 行 |
| `viewport.rs` | 方法签名简化 | ~10 行 |
| `reshape_worker.rs` | ReshapeRequest/Result 加 dv_idx | ~20 行 |
| **总计** | | **~215 行** |

---

## 六、风险

| 风险 | 概率 | 缓解 |
|------|------|------|
| 借用冲突（dv mutable vs display_map immutable） | 高 | 方案 1：Viewport 方法取 total_rows 替代 |
| 测试大规模更新 | 中 | 机械替换，逐文件改 |
| Reshape worker dv_idx 路由错误 | 中 | 先限制为 active dv，后续扩展 |
| 多 dv 场景未充分测试 | 低 | 当前单 dv 为主，保留结构扩展性 |

---

## 七、实施顺序

```
Phase 1（结构迁移，1-2h）：
  ├── DocumentView 加 display_map 字段 + new() 初始化
  ├── 改 DocumentView 方法签名（去 wi 参数）
  └── Viewport 方法改为 total_rows 参数

Phase 2（App 层适配，1-2h）：
  ├── App 移除 display_map 字段
  ├── init_display_map 适配
  └── 所有 &self.display_map → dv.display_map

Phase 3（Reshape 适配，1h）：
  ├── ReshapeRequest/Result 加 dv_idx
  └── drain_reshape_results 路由

Phase 4（测试更新 + 验证，1-2h）：
  ├── 测试编译修复
  ├── cargo test 全量通过
  └── clippy
```

---

## 八、审计文档补充上下文

来自 `wrap_pipeline_audit.md` §0-2：

- **问题根源**：`App` 全局只有一份 `display_map`，切 tab 时 `init_display_map` 不触发，导致新 tab 的 wrap 结果用旧数据
- **连锁影响**：`reshape_worker`、`advance_cache` 也是全局共享，但 P2-8 当前只要求改 `display_map`（`advance_cache` 暂不动，它在每帧 `render()` 中 drain 重建）
- **切 tab 路径**：需要在 `Workspace::switch_to()` 或 `App::handle_command(SwitchTab)` 中触发新 dv 的 `init_display_map`
- **协调点**：与 `docs/plan-ui-split.md` 的 Viewport 迁移方向一致（DisplayMap 下沉到 View）

### 切 tab 适配

在当前实现中（`app.rs`），切 tab 路径需要显式初始化新 dv 的 display_map：

```rust
// handle_command(SwitchTab) 或类似路径
let dv = &mut self.workspace.doc_views[new_idx];
if dv.display_map.is_empty() {
    let entries = build_entries_from_line_index(&dv.line_index, viewport_width, ...);
    dv.display_map.set_entries(entries);
}
self.reshape_generation += 1;
if let Some(ref w) = self.reshape_worker {
    w.cancel_before(self.reshape_generation);
}
```

这在阶段 2 的 App 层适配中一并完成。
