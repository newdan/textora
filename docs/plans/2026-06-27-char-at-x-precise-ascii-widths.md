# 修复 `char_at_x`/`char_x` 精度：用 `ascii_widths` 缓存替代启发式估算

## Context

当前 `char_at_x`/`char_x` 在 shap 数据不可用时（`flat_line.shaped == None`），对非 CJK 字符使用 `font_size * 0.55` 启发式估算宽度。`LayoutCtx` 中已有精确的 `ascii_widths: [f32; 128]` 缓存（每个可打印 ASCII 字符都通过 `shaper.grapheme_advance()` 实测），但该缓存没有传递给 `char_at_x`/`char_x`。

此外，`prepend_marker_to_line` 会销毁 shap 数据（`line.shaped = None; line.text_layout = None`），导致带块标记的行（标题 `#`、列表 `-`、引用 `>`）回退到启发式估算。

## 方案

在 `LazyLayout` 上构建并存储 `Arc<[f32; 128]>` 精确宽度表，传播到每个 `FlatLine`。`char_at_x`/`char_x` fallback 路径改为查表而非 `* 0.55`。CJK 字符继续使用 `font_size`（等宽方形，天然正确）。

不同 `font_size` 的行（标题/正文/代码块）通过 `font_size / body_font_size` 缩放。

## 修改步骤

### Step 1: `FlatLine` 添加宽度缓存字段

**文件**: `crates/markdown/src/layout/types.rs:635-646`

```rust
pub struct FlatLine {
    // ... 现有字段不变 ...
    /// 预计算的 ASCII 字符精确宽度（在 body_font_size 下测量）。
    pub ascii_widths: Option<Arc<[f32; 128]>>,
    /// font_size / body_font_size，用于缩放 ascii_widths。
    pub font_size_scale: f32,
}
```

### Step 2: `LazyLayout` 添加宽度缓存

**文件**: `crates/markdown/src/layout/types.rs:53-95`

在 `edit_ctx` 字段之后添加：
```rust
pub ascii_widths: Option<Arc<[f32; 128]>>,
pub body_font_size: f32,
```

### Step 3: `LazyLayout::new()` 初始化 body_font_size

**文件**: `crates/markdown/src/layout/types.rs:716-733`

添加 `ascii_widths: None, body_font_size: style.body_font_size`

### Step 4: 在 `build_flat_lines` 后传播宽度到各 FlatLine

**文件**: `crates/markdown/src/layout/types.rs:149`（`self.flat_lines = lines` 之后）

```rust
if let Some(ref aw) = self.ascii_widths {
    let bfs = self.body_font_size.max(1e-6);
    for fl in &mut self.flat_lines {
        fl.ascii_widths = Some(Arc::clone(aw));
        fl.font_size_scale = fl.font_size / bfs;
    }
}
```

### Step 5: 布局时构建 `ascii_widths`

在 `ensure_visible`、`ensure_all_blocks`、`ensure_precise_range` 的循环之前，若 `self.ascii_widths.is_none()` 且 shaper 可用，构建一次：

```rust
if self.ascii_widths.is_none() {
    if let Some(ref mut s) = shaper {
        self.ascii_widths = Some(Arc::new(build_ascii_widths(s, style.body_font_size)));
    }
}
```

提取 `LayoutCtx::new` 中的 ASCII 宽度测量逻辑为独立函数 `build_ascii_widths`：
```rust
fn build_ascii_widths(shaper: &mut shaping::Shaper, body_font_size: f32) -> [f32; 128] {
    let saved = shaper.font_size();
    shaper.set_font_size(body_font_size);
    let mut w = [0.0f32; 128];
    for c in 0x20..0x7f {
        let mut buf = [0u8; 4];
        let s = char::from_u32(c).unwrap().encode_utf8(&mut buf);
        w[c as usize] = shaper.grapheme_advance(s).unwrap_or(body_font_size * 0.55);
    }
    shaper.set_font_size(saved);
    w
}
```

### Step 6: 修改 `char_at_x` fallback

**文件**: `crates/markdown/src/layout/context.rs:52-65`

```rust
// fallback — 使用精确 ascii_widths 替代 0.55 估算
let font_size = flat_line.font_size;
let mut cum_x = 0.0f32;
let mut visual_char = 0usize;
for ch in text.chars() {
    let cw = if is_cjk_or_fullwidth(ch) {
        font_size
    } else if let Some(ref aw) = flat_line.ascii_widths {
        let b = ch as u32;
        if b < 128 { aw[b as usize] * flat_line.font_size_scale }
        else { font_size * 0.55 }
    } else {
        font_size * 0.55
    };
    if rel_x < cum_x + cw * 0.5 { return visual_char; }
    cum_x += cw;
    visual_char += 1;
}
visual_char
```

### Step 7: 修改 `char_x` fallback

**文件**: `crates/markdown/src/layout/context.rs:85-94`

```rust
let font_size = flat_line.font_size;
let mut cum_x = 0.0f32;
for (i, ch) in text.chars().enumerate() {
    if i >= visual_char { break; }
    let cw = if is_cjk_or_fullwidth(ch) {
        font_size
    } else if let Some(ref aw) = flat_line.ascii_widths {
        let b = ch as u32;
        if b < 128 { aw[b as usize] * flat_line.font_size_scale }
        else { font_size * 0.55 }
    } else {
        font_size * 0.55
    };
    cum_x += cw;
}
cum_x
```

### Step 8: HorizontalRule FlatLine 构造

**文件**: `crates/markdown/src/layout/types.rs:434-441`

给新字段设默认值 `ascii_widths: None, font_size_scale: 1.0`。

### Step 9: 测试适配

**文件**: `crates/markdown/src/layout/types.rs` 中的 `layout_with_cursor` 辅助函数

`ensure_precise_range` 调用前，在 `LazyLayout` 上构建 `ascii_widths`。需要在测试路径中也触发宽度表构建。

## 不修改的部分

- `prepend_marker_to_line`：继续清空 shap 数据（内容已变，旧数据无效），但 fallback 现在精确了
- 10 个调用点（selection/search/view）：无需修改，`ascii_widths` 已在 `FlatLine` 上

## 内存开销

- `LazyLayout`: +16 bytes
- 每 `FlatLine`: +16 bytes（Arc 指针 + f32 + padding）
- 共享数据: 512 bytes（`[f32; 128]`，整个布局只分配一次）
- 1000 行文档: ~16KB

## 验证

1. `cargo test` — 现有测试必须通过
2. 手动测试：WYSIWYG 模式下点击标题/列表/引用中的文本，光标应在字符间而非字符中间
3. 重点场景：`# 中文标题` — '#' 和 ' ' 宽度现在精确；`- 列表项` — '-' 和 ' ' 宽度精确
