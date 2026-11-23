# P3-20：消除 Settings 中所有 unsafe 的详细计划

> 日期：2026-06-12
> 范围：`crates/ui/src/settings.rs` 的 `get()`、`get_mut()`、`get_static()` 三个方法
> 目标：消除所有 3 处 unsafe，0 处新增 unsafe

---

## 当前 unsafe 分布

| 方法 | unsafe 形式 | 原因 |
|------|------------|------|
| `get()` | `transmute(Ref<'_>) → Ref<'static>` | 延长 thread_local 内 RefCell borrow 生命周期 |
| `get_mut()` | `transmute(RefMut<'_>) → RefMut<'static>` | 同上 |
| `get_static()` | `&*(s.as_ptr() as *const Settings)` | 绕过 RefCell 直接返回裸指针引用 |

---

## 可行性分析

### `get()` / `get_mut()` → 去掉 transmute

当前签名：
```rust
pub fn get() -> Ref<'static, Self> { ... }
pub fn get_mut() -> RefMut<'static, Self> { ... }
```

改为：
```rust
pub fn get() -> Ref<'_, Self> { SETTINGS.with(|s| s.borrow()) }
pub fn get_mut() -> RefMut<'_, Self> { SETTINGS.with(|s| s.borrow_mut()) }
```

**风险评估**：所有 162 个 `Settings::get()` 调用点均使用内联模式（如 `.line_height`、`.dpi_scale`），Ref 在表达式结束时即 drop。无代码将 `Ref` 存入长期变量。`'_` 生命周期足够，安全。

### `get_static()` → 废除，改用 `get()`

36 个调用点分三类：

| 类别 | 模式 | 数量 | 替换方案 |
|------|------|------|----------|
| A: 内联字段访问 | `get_static().field` | ~20 | `get().field` |
| B: 传入函数作 `&Settings` | `func(get_static(), ...)` | ~12 | `func(&*get(), ...)` |
| C: 存入局部变量 | `let s = get_static();` | ~4 | `let s = get();`（`Ref` 实现了 `Deref<Target=Settings>`） |

**Category C 详细**（`workspace.rs:239,253,459`）：
```rust
let settings = Settings::get_static();
let visible_rows = settings.visible_rows(...);
```
改为：
```rust
let settings = Settings::get();
let visible_rows = settings.visible_rows(...); // Ref 自动 Deref 为 &Settings
```
`Ref<'_, Settings>` 实现了 `Deref<Target=Settings>`，`.visible_rows()` 调用透明通过。

---

## 执行步骤

### Step 1: 重写 `get()` 和 `get_mut()`
- 去掉 `transmute`，直接返回 `SETTINGS.with(|s| s.borrow())`
- 返回值类型改为 `Ref<'_, Settings>` / `RefMut<'_, Settings>`
- 验证 `cargo check -p edit-plus-ui`

### Step 2: 系统替换 `get_static()` 调用点
文件依次处理：

| 文件 | 调用点 | 类型 |
|------|--------|------|
| `events.rs` | L158, L213 | B — 改 `&*get()` |
| | L258, L261, L373, L440-L470 | A — 改 `get().field` |
| `app_renderer.rs` | L157, L421, L465 | A |
| | L190, L514, L546 | B |
| `app.rs` | L157-158, L184, L273-274, L1253, L1276, L1712, L1874, L2403, L2405, L2468 | A |
| | L678 | B |
| `workspace.rs` | L239, L253, L459 | C — 改 `get()` 存变量 |
| | L738 | A |
| | L832, L842, L857 | B |

### Step 3: 删除 `get_static()` 定义
- 删除 `settings.rs` 中的 `pub fn get_static()` 方法体
- 标记为 `#[deprecated]` 过渡一回合，或直接删除

### Step 4: 验证
```bash
cargo check --workspace
cargo test -p edit-plus-ui --lib
cargo test -p edit-plus-app --lib
```

---

## 不改动的边界

| 项 | 原因 |
|----|------|
| `thread_local! + RefCell` 全局模式 | 替换为 `OnceLock` 需支持运行时可变，`OnceLock<RefCell<>>` 与当前 `thread_local!` 等价且 `init()` 的多次调用能力丢失 |
| `Settings::init()` | 保留不动，`thread_local!` 模式支持多次替换 |

---

## 预期结果

- `settings.rs` 中 unsafe 从 3 处降为 0 处
- `get_static()` 方法完全移除
- 所有调用点改用安全的 `get()`/`get_mut()`
- 编译 + 测试全绿，无功能变更
