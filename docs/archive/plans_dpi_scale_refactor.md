# DPI Scale & Settings 重构计划

## 背景

1. Tab 下拉菜单（overflow menu）里内容不可见 — 颜色在 dark/light 主题下与背景
   接近；硬编码像素值未做 DPI 缩放
2. `tab_bar` 模块里 `screen_w` / `screen_h` / `dpi_scale` 散落在各函数参数表
   中，不一致
3. `Settings` 目前是 `App::settings` 私有字段，通过 `&Settings` 到处传递，
   与 `RenderContext` 里也有一份

目标：统一入口、消除散落参数、补上 DPI 缩放。

---

## Phase 1：引入 TabBarCtx，收拢 tab_bar 参数

### 目标
把所有需要 `dpi_scale` / `screen_w` / `screen_h` 的 tab_bar 函数统一为
`ctx: &TabBarCtx` 参数，不再单传裸 `f32`。

同时修复 overflow menu DPI 缩放缺失的问题。

### 涉及函数（tab_bar.rs）

| 函数                    | 当前签名                                           | 改后                         |
|-------------------------|---------------------------------------------------|------------------------------|
| `tab_bar_height`        | `(dpi_scale: f32)`                                | `(settings: &Settings)`      |
| `layout_tabs`           | `(..., screen_w, screen_h, dpi_scale, ...)`       | `(..., ctx: &TabBarCtx, ...)` |
| `tab_bar_text_positions`| `(..., screen_w, screen_h, dpi_scale, ...)`       | `(..., ctx: &TabBarCtx, ...)` |
| `build_overflow_menu`   | `(..., screen_w, screen_h)`                       | `(..., ctx: &TabBarCtx)`     |
| `overflow_menu_text_positions` | `(..., screen_w, _screen_h)`                | `(..., ctx: &TabBarCtx)`     |

### TabBarCtx 定义

```rust
// crates/app/src/tab_bar.rs
pub struct TabBarCtx<'a> {
    pub settings: &'a Settings,
    pub screen_w: f32,
    pub screen_h: f32,
}
```

### 具体改动

#### tab_bar.rs

1. 新增 `TabBarCtx<'a>` struct
2. `tab_bar_height` — 参数从 `dpi_scale: f32` 改为 `settings: &Settings`，
   内部 `32.0 * settings.dpi_scale`
3. `layout_tabs` — 去掉 `screen_w` / `screen_h` / `dpi_scale` 三个参数，
   改为 `ctx: &TabBarCtx`
4. `tab_bar_text_positions` — 同样去掉三个参数，改为 `ctx: &TabBarCtx`
5. `build_overflow_menu` — 去掉 `screen_w` / `screen_h`，加 `ctx: &TabBarCtx`；
   内部所有硬编码像素值乘 `ctx.settings.dpi_scale`（30px、8px、4px、
   230px、2px gap 等）
6. `overflow_menu_text_positions` — 去掉 `screen_w` / `_screen_h`，
   加 `ctx: &TabBarCtx`；`8.0` 乘 `ctx.settings.dpi_scale`

#### workspace.rs

- `update_tab_layout` — 构造 `TabBarCtx` 传给 `layout_tabs`
- `current_tab_bar_height` — 从 `dpi_scale: f32` 改为 `settings: &Settings`
- `open_overflow_menu` — 构造 `TabBarCtx` 传给 `build_overflow_menu`

#### app.rs

- `current_tab_bar_height()` — 从 `self.settings.dpi_scale` 改为
  `&self.settings`
- 所有 `tab_bar::tab_bar_height(self.settings.dpi_scale)` →
  `tab_bar::tab_bar_height(&self.settings)`
- `layout_tabs` 调用 — 构造 `TabBarCtx`
- `overflow_menu_text_positions` 调用 — 构造 `TabBarCtx`
- `tab_bar_text_positions` 调用 — 构造 `TabBarCtx`

### 影响文件
- `crates/app/src/tab_bar.rs`
- `crates/app/src/workspace.rs`
- `crates/app/src/app.rs`

### 验证
```bash
cargo test -p edit-plus-app --lib -- overflow_menu
cargo check -p edit-plus-app
```

---

## Phase 2：Settings 全局化

### 目标
把 `Settings` 从 `App` 的私有字段提升为 thread-local 全局单例，
消除到处传递 `&Settings` 的模式。

### 方案

```rust
// crates/app/src/settings.rs

use std::cell::{RefCell, Ref, RefMut};

thread_local! {
    static SETTINGS: RefCell<Settings> = RefCell::new(Settings::new());
}

impl Settings {
    pub fn get() -> Ref<'static, Self> {
        SETTINGS.with(|s| s.borrow())
    }
    pub fn get_mut() -> RefMut<'static, Self> {
        SETTINGS.with(|s| s.borrow_mut())
    }
    /// 初始化/替换全局 Settings（用于测试或首次配置）
    pub fn init(s: Settings) {
        SETTINGS.with(|cell| *cell.borrow_mut() = s);
    }
}
```

选用 `RefCell` 而非 `RwLock` 的理由：winit 事件循环是单线程的，渲染
（只读 borrow）和事件处理（可变 borrow）不交织。

### 替换规则

| 旧写法                                    | 新写法                          |
|-------------------------------------------|---------------------------------|
| `self.settings.xxx`                       | `Settings::get().xxx`           |
| `self.settings.set_font_size(n)`          | `Settings::get_mut().set_font_size(n)` |
| `&self.settings` (传给函数参数)            | 删除参数，函数内部直接 `Settings::get()` |
| `app.settings.xxx` (测试中)              | `Settings::get().xxx`           |

### 具体改动

#### app.rs（84 处 `self.settings`）

- 删除 `settings: Settings` 字段
- `App::new()` 中把默认 Settings 写入全局
- 所有 `self.settings.xxx` 替换为 `Settings::get().xxx`
- 所有 `self.settings.set_xxx(...)` 替换为 `Settings::get_mut().set_xxx(...)`
- `apply_scale` 调用改为 `Settings::get_mut().apply_scale(...)`

#### workspace.rs（10 处）

- 删除函数中 `settings: &Settings` 参数，内部改为 `Settings::get()`
- 涉及：
  - `open_file`
  - `new_empty_tab`
  - `update_tab_layout`（Phase 1 已改为 ctx，这里删 settings 引用）
  - `current_tab_bar_height`

#### render_pipeline.rs（19 处）

- `RenderContext` 删除 `settings` 字段
- 所有 `ctx.settings.xxx` → `Settings::get().xxx`

#### search_bar.rs（8 处）

- 所有 `ctx.settings.xxx` → `Settings::get().xxx`

#### 测试

- `app.rs` 测试中 `app.settings.xxx` → `Settings::get().xxx`
- `workspace.rs` 测试中 `settings: &Settings` 构造 → `Settings::init(...)`
- `search_bar.rs` 测试同样调整

### 影响文件
- `crates/app/src/settings.rs`（加全局实现）
- `crates/app/src/app.rs`（84 处替换）
- `crates/app/src/workspace.rs`（10 处替换）
- `crates/app/src/render_pipeline.rs`（19 处替换 + 删字段）
- `crates/app/src/search_bar.rs`（8 处替换）
- `crates/app/src/mouse.rs`（如涉及）

### 验证
```bash
cargo check -p edit-plus-app
cargo test -p edit-plus-app --lib
```

---

## Phase 3：清理冗余 settings 字段

Phase 2 完成后 `RenderContext.settings` 和 `TabBarCtx.settings` 变成
冗余——它们可以直接调 `Settings::get()`。

### 具体改动

#### RenderContext
- 删除 `settings: &'a Settings` 字段
- 构造处删掉对应赋值
- 内部使用处删掉 `ctx.settings.` 前缀（Phase 2 已改为 `Settings::get()`，
  此步为清理）

#### TabBarCtx
- 同样删除 `settings` 字段
- `tab_bar` 内部所有 `ctx.settings.dpi_scale` → `Settings::get().dpi_scale`

### 影响文件
- `crates/app/src/render_pipeline.rs`
- `crates/app/src/tab_bar.rs`
- `crates/app/src/app.rs`
- 所有 RenderContext / TabBarCtx 构造处

### 验证
```bash
cargo check -p edit-plus-app
cargo test -p edit-plus-app --lib
```

---

## 注意事项

1. 每个 Phase 完成后确保 `cargo check` 通过再进下一步
2. `RefCell` borrow 规则：渲染中不可调用 `get_mut()`，事件处理中
   确保 render 不持有 borrow
3. 测试中需注意 `Settings::init()` 调用顺序，避免 borrow 冲突
