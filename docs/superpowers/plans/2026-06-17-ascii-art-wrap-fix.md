# 代码块 ASCII 树状图显示错乱修复 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复代码块中 ASCII 树状图 box-drawing 字符（`├│└─`等）显示错乱——因 `shape_line()` 未设置等宽字体族，导致 shaper 用比例字体 measure/shape 代码块文本，advance 错乱。

**Architecture:** `shape_line()` 新增 `font_family: Option<&str>` 参数。代码块调用方传入 `code_font_family`，段落/列表调用方传入 `body_font_family`。函数内部 set shaper 字体族后再 shape，`UiTextLayout::from_shaped` 也传入实际 font_family（替换硬编码 `None`）。

**Tech Stack:** Rust，`crates/markdown/src/layout.rs`，依赖 cosmic-text Shaper

---

### Task 1: `shape_line` 加 `font_family` 参数

**Files:**
- Modify: `crates/markdown/src/layout.rs:749-788`

- [ ] **Step 1: 修改函数签名和内部逻辑**

**Before** (lines 749-755):
```rust
fn shape_line(
    text: &str,
    font_size: f32,
    weight: shaping::Weight,
    style: shaping::Style,
    shaper: Option<&mut Shaper>,
) -> (Option<shaping::ShapedRun>, Option<Arc<ui::core::text_layout::UiTextLayout>>) {
    let Some(shaper) = shaper else {
        return (None, None);
    };
```

**After**:
```rust
fn shape_line(
    text: &str,
    font_size: f32,
    weight: shaping::Weight,
    style: shaping::Style,
    font_family: Option<&str>,
    shaper: Option<&mut Shaper>,
) -> (Option<shaping::ShapedRun>, Option<Arc<ui::core::text_layout::UiTextLayout>>) {
    let Some(shaper) = shaper else {
        return (None, None);
    };
```

**Before** (line 766):
```rust
    shaper.set_font_size(font_size);
    shaper.set_font_weight(weight);
    shaper.set_font_style(style);
```

**After** — 在 set_font_style 之后加 set_font_family：
```rust
    shaper.set_font_size(font_size);
    shaper.set_font_weight(weight);
    shaper.set_font_style(style);
    if let Some(family) = font_family {
        shaper.set_font_family(Some(family));
    }
```

**Before** (line 781, `UiTextLayout::from_shaped` 的 font_family 参数):
```rust
    let text_layout = shaped.as_ref().map(|s| {
        Arc::new(ui::core::text_layout::UiTextLayout::from_shaped(
            text,
            font_size,
            None,          // ← 硬编码 None
            weight,
            style,
            s.clone(),
        ))
    });
```

**After** — 传入实际的 font_family：
```rust
    let text_layout = shaped.as_ref().map(|s| {
        Arc::new(ui::core::text_layout::UiTextLayout::from_shaped(
            text,
            font_size,
            font_family.map(|s| s.to_string()),
            weight,
            style,
            s.clone(),
        ))
    });
```

- [ ] **Step 2: 构建检查**

```bash
cargo build -p markdown 2>&1 | head -30
```

预期：编译报错——三个调用点缺少新参数。下一步逐个修复。

---

### Task 2: 更新代码块调用点

**Files:**
- Modify: `crates/markdown/src/layout.rs:398-402`

- [ ] **Step 1: 代码块传入 `code_font_family`**

**Before** (lines 398-402):
```rust
                let (shaped, text_layout) = shape_line(
                    line_text, font_size,
                    shaping::Weight::NORMAL, shaping::Style::Normal,
                    ctx.shaper.as_deref_mut(),
                );
```

**After**:
```rust
                let (shaped, text_layout) = shape_line(
                    line_text, font_size,
                    shaping::Weight::NORMAL, shaping::Style::Normal,
                    ctx.style.code_font_family.as_deref(),
                    ctx.shaper.as_deref_mut(),
                );
```

- [ ] **Step 2: 构建检查**

```bash
cargo build -p markdown 2>&1 | head -20
```

预期：还有两个调用点报错。

---

### Task 3: 更新段落调用点

**Files:**
- Modify: `crates/markdown/src/layout.rs:555-559`

- [ ] **Step 1: 段落/标题传入 `body_font_family`**

**Before** (lines 555-559):
```rust
            let (shaped, text_layout) = shape_line(
                w, font_size,
                shaping::Weight::NORMAL, shaping::Style::Normal,
                ctx.shaper.as_deref_mut(),
            );
```

**After**:
```rust
            let (shaped, text_layout) = shape_line(
                w, font_size,
                shaping::Weight::NORMAL, shaping::Style::Normal,
                ctx.style.body_font_family.as_deref(),
                ctx.shaper.as_deref_mut(),
            );
```

- [ ] **Step 2: 构建检查**

```bash
cargo build -p markdown 2>&1 | head -20
```

预期：还有 1 个调用点报错（列表项，line 722）。

---

### Task 4: 更新列表调用点并构建通过

**Files:**
- Modify: `crates/markdown/src/layout.rs:722`

- [ ] **Step 1: 找到 line 722 附近的 shape_line 调用**

```bash
grep -n "shape_line" crates/markdown/src/layout.rs
```

预期输出包含 line 722。读取该行：

```bash
sed -n '720,726p' crates/markdown/src/layout.rs
```

**Before** (lines 720-726，列表项文本 layout):
```rust
        let (shaped, text_layout) = shape_line(
            w, font_size,
            shaping::Weight::NORMAL, shaping::Style::Normal,
            ctx.shaper.as_deref_mut(),
        );
```

**After**:
```rust
        let (shaped, text_layout) = shape_line(
            w, font_size,
            shaping::Weight::NORMAL, shaping::Style::Normal,
            ctx.style.body_font_family.as_deref(),
            ctx.shaper.as_deref_mut(),
        );
```

- [ ] **Step 2: 构建通过**

```bash
cargo build -p markdown 2>&1 | head -10
```

预期：编译通过，零 warning。

---

### Task 5: 更新测试

**Files:**
- Modify: `crates/markdown/src/layout.rs:1179-1235` — 现有 `shape_line_*` 测试

- [ ] **Step 1: 更新测试中的 shape_line 调用，加 font_family 参数**

所有 `shape_line(...)` 调用的测试（lines 1180, 1188, 1196, 1210, 1223, 1231）都需要加 `None` 作为 font_family 参数（测试不需要特定字体族）。

**示例** — `shape_line_no_shaper_returns_none` (line 1180):
```rust
// Before
let (shaped, layout) = shape_line("hello", 14.0, shaping::Weight::NORMAL, shaping::Style::Normal, None);
// After
let (shaped, layout) = shape_line("hello", 14.0, shaping::Weight::NORMAL, shaping::Style::Normal, None, None);
```

**批处理** — 用 sed 替换所有测试中的 `shape_line` 调用：

```bash
# 在测试区域内（line 1176 之后），给所有 shape_line 调用的最后一个参数前插入 None,
sed -i '' '1176,$s/shaping::Style::Normal, None)/shaping::Style::Normal, None, None)/g' crates/markdown/src/layout.rs
```

验证修改正确：

```bash
grep -n "shape_line" crates/markdown/src/layout.rs | grep -v "^.*fn shape_line"
```

预期：所有调用的参数数量一致（6 个参数）。

如果 sed 不够精确，手动检查每处：

- Line 1180: `shape_line("hello", 14.0, shaping::Weight::NORMAL, shaping::Style::Normal, None);`
  → `shape_line("hello", 14.0, shaping::Weight::NORMAL, shaping::Style::Normal, None, None);`

- Line 1188: `shape_line("", 14.0, shaping::Weight::NORMAL, shaping::Style::Normal, Some(&mut shaper));`
  → `shape_line("", 14.0, shaping::Weight::NORMAL, shaping::Style::Normal, None, Some(&mut shaper));`

- Line 1196: `shape_line("hello", 14.0, shaping::Weight::NORMAL, shaping::Style::Normal, Some(&mut shaper));`
  → `shape_line("hello", 14.0, shaping::Weight::NORMAL, shaping::Style::Normal, None, Some(&mut shaper));`

- Line 1210: `shape_line("test", 14.0, shaping::Weight::NORMAL, shaping::Style::Normal, Some(&mut shaper));`
  → `shape_line("test", 14.0, shaping::Weight::NORMAL, shaping::Style::Normal, None, Some(&mut shaper));`

- Line 1223: `shape_line("hello", 14.0, shaping::Weight::NORMAL, shaping::Style::Normal, Some(&mut shaper));`
  → `shape_line("hello", 14.0, shaping::Weight::NORMAL, shaping::Style::Normal, None, Some(&mut shaper));`

- Line 1231: `shape_line("hello", 14.0, shaping::Weight::NORMAL, shaping::Style::Normal, Some(&mut shaper));`
  → `shape_line("hello", 14.0, shaping::Weight::NORMAL, shaping::Style::Normal, None, Some(&mut shaper));`

- [ ] **Step 2: 添加 box-drawing 字符回归测试**

在 `shape_line_*` 测试区域末尾新增：

```rust
    #[test]
    fn shape_line_box_drawing_chars_get_consistent_advance() {
        // Box-drawing characters in a monospace font should have
        // consistent advance widths, preserving ASCII art alignment.
        let mut font_system = shaping::FontSystem::new();
        let mut shaper = shaping::Shaper::new(&mut font_system);
        let (shaped, layout) = shape_line(
            "├── mod.rs        # Widget trait",
            14.0,
            shaping::Weight::NORMAL,
            shaping::Style::Normal,
            Some("monospace"),
            Some(&mut shaper),
        );
        // Should produce a shaped result
        assert!(shaped.is_some(), "box-drawing line should shape successfully");
        assert!(layout.is_some(), "box-drawing line should produce text layout");
        // With monospace font, all box-drawing glyph advances should be equal
        let shaped = shaped.unwrap();
        let box_advances: Vec<f32> = shaped.clusters.iter()
            .filter(|c| {
                let ch = "├── mod.rs        # Widget trait"
                    .get(c.byte_range.clone())
                    .and_then(|s| s.chars().next());
                ch.map_or(false, |ch| matches!(ch, '├' | '─' | '└' | '│'))
            })
            .map(|c| c.advance)
            .collect();
        if box_advances.len() >= 2 {
            let first = box_advances[0];
            for &a in &box_advances[1..] {
                assert!((a - first).abs() < 0.5,
                    "box-drawing advances should be consistent: {} vs {}", first, a);
            }
        }
    }
```

- [ ] **Step 3: 运行测试**

```bash
cargo test -p markdown 2>&1
```

预期：全部通过。若 `shape_line_box_drawing_chars_get_consistent_advance` 因缺少字体而失败（在 CI 或无字体环境中），标记 `#[ignore]`。

- [ ] **Step 4: Commit**

```bash
git add crates/markdown/src/layout.rs
git commit -m "$(cat <<'EOF'
fix(markdown): pass font_family to shape_line for correct code block shaping

shape_line() was using the shaper's current font family (typically the
proportional body font) to shape code block text. This caused box-drawing
characters and spaces to have incorrect advance widths, breaking ASCII
tree diagram alignment.

Add font_family parameter to shape_line():
- Code blocks → code_font_family (monospace)
- Paragraphs/headings → body_font_family (sans-serif)
- UiTextLayout::from_shaped now receives the actual font_family

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: 手动验证

- [ ] **Step 1: 完整构建**

```bash
cargo build 2>&1 | tail -5
```

- [ ] **Step 2: 用代码块树状图目测验证**

打开包含以下内容的 markdown 文件：

````markdown
### 文件树

```
widgets/sidebar/
├── mod.rs        # Widget trait 实现
├── state.rs      # SidebarState + SidebarAction
├── layout.rs     # SidebarLayoutItem 计算
├── paint.rs      # 绘制逻辑（整合 widget 动画层 + 旧 chrome 绘制）
├── types.rs      # SidebarInput, SidebarCfg 等
└── persistent.rs # SidebarPersistent
```

### 混合内容

```
src/
├── main.rs          # 入口
├── app.rs           # App 结构体
│   └── window.rs    # 窗口管理
└── utils/
    ├── fs.rs        # 文件系统工具
    └── math.rs      # 数学工具
```
````

检查要点：
- `├──` 等 box-drawing 字符正确渲染（非 tofu/乱码）
- 多行 `├──` 的 `─` 宽度一致（不因字体 fallback 而长短不一）
- `#` 注释列垂直对齐
- 文件名和 `#` 之间的空格间距与源码一致
- 无意外换行或文字重叠

---

## 改动总结

| 改动 | 位置 | 量 |
|------|------|-----|
| `shape_line` 签名加 `font_family` 参数 | `layout.rs:749-755` | +1 参数 |
| `shape_line` 内部 set font_family | `layout.rs:766` 后 | +3 行 |
| `UiTextLayout::from_shaped` 传 font_family | `layout.rs:781` | 改 1 行 |
| 代码块调用点传 `code_font_family` | `layout.rs:398-402` | +1 行 |
| 段落调用点传 `body_font_family` | `layout.rs:555-559` | +1 行 |
| 列表调用点传 `body_font_family` | `layout.rs:720-726` | +1 行 |
| 测试更新（6 处）| `layout.rs:1179-1231` | 各 +1 参数 |
| 新增 box-drawing 测试 | `layout.rs` | +35 行 |

改动仅限一个文件（`layout.rs`）。`shape_line` 是内部函数，对外 API 不受影响。
