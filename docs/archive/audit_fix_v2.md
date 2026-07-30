# 审计修复方案 v2

针对 `audit_fix.md` 第一轮修复后的复检结论。本文件接续 v1，
不替代它；v1 已勾选项不再重复。

修复时间窗：2026-05-29 之后。

## 0. 一句话结论

**core 层修复方向正确（S1/S2/s1/s2 已落）；app 层出现 N1/N2/N3 三个"伪实现"拼装，
不立刻处理会让阶段 5 的所有性能验收失真。** v1 的 M2/M4/M5/M6 大多没动。

## 1. v1 项复检

| 项 | v1 状态 | 现状 | 备注 |
|---|---|---|---|
| S1 file.rs 方向 | 待修 | ✅ | 重写为 `allocate_gap` + `commit_gap` 流式路径，行尾扫描合一遍 |
| S2 terminal_stubs / lsh stub | 待修 | ✅ | 删干净；`buffer/mod.rs` 缩到 11 行；`text_buffer.rs.deferred` 保留 |
| S3 detect_line_ending 双循环 | 待修 | ✅ | 单遍扫描；但函数被 `#[cfg(test)]` 掉，存在割裂（见 N6） |
| M1 shaping cache LRU | 待修 | 🟡 部分 | 改用 `hashlink::LruCache`，cap 4096；**key 仍是 `String`，不是 `(cluster, font_size, attrs_hash)` 复合 key**——同一字体不同字号会互相覆盖，错误结果 |
| M2 Buffer 复用 | 待修 | ❌ | `Shaper::shape` 还是每次 `Buffer::new(&mut self.font_system, metrics)`，cosmic-text line cache 完全不工作 |
| M3 Shaper Default 静默吞错 | 待修 | ❌ | `Default for Shaper` 仍含 `Self::new().unwrap_or(...)` 死分支 |
| M4 Atlas O(N) LRU | 待修 | ❌ | `access_order: Vec<GlyphKey>` + `retain` + `remove(0)`；引入 hashlink 依赖却没用进 atlas |
| M5 atlas oversized 缓存 | 待修 | ❌ | 仍重复创建 + 丢弃 page |
| M6 app expect/unwrap | 待修 | ❌ | `app.rs::init_window` 仍 `expect("failed to create window")` 等 4 处 |
| M7 抽 create_gpu | 待修 | ✅（半成品） | `gpu.rs::create_gpu_context` 写好返回 `Result`，但 `app.rs::init_window` 完全没切到这个函数 |
| M8 sys 并入 icu.rs | 待修 | 🟡 部分 | 3 个函数搬入 `mod sys`，但 `icu_detect_renaming_suffix` / `icu_add_renaming_suffix` 缺；fmt 缩进不对（见 N5） |
| s1 编译警告 | 待修 | ✅ | `cargo build` 0 warnings |
| s2 contains_null 抽象 | 待修 | ✅ | 删了 |
| s3 plans.md §7 checklist | 待修 | ❌ | 未更新 |

待办：M1（key 改复合）、M2、M3、M4、M5、M6、M8（补两个函数）、s3。

---

## 2. 新发现严重问题

### N1 🔴 `app::DocumentView::from_file` 重新引入第二条文件路径

**位置**：`crates/app/src/document_view.rs:27-37`

```rust
pub fn from_file(path: &Path, visible_rows: usize) -> Result<Self, String> {
    let content = std::fs::read_to_string(path)?;                       // 整文件入 String
    let lines: Vec<String> = content.lines().map(String::from).collect(); // 切成 Vec<String>
    ...
}
```

**问题**：
1. 内存峰值 = 文件大小 × 3（raw read + String + Vec<String>），50 MB 文件直接超 plans.md §5 RSS < 150 MB 门槛
2. `core::file::load_file` 是修好的零拷贝路径，**app 中无人调用**（grep 确认）
3. `lines()` 丢弃行尾符，保存时无法保留 CRLF/LF（违反 plans.md 阶段 8 的 EOL 保留要求）
4. 与 plans.md §4.1 "上层只依赖 trait + GapBuffer" 设计相悖
5. 阶段 5 验收 `viewport_only_shapes_visible_lines` / `bench_open_50mb_*` 全部基于 GapBuffer，现在 Vec<String> 对不上

**修复**：
- `DocumentView` 持有 `GapBuffer`（来自 `core::file::load_file`），不持有 `Vec<String>`
- `visible_lines()` 改成按行号惰性切片：用 `simd::lines_fwd` 找到行起止 byte offset，再 `read_forward` 取 `&[u8]`，shape 时按需转 UTF-8
- `from_file` 直接调 `core::file::load_file(path)?` 拿到 `(GapBuffer, FileMetadata)`，在 `DocumentView` 里持有

**接口示意**：
```rust
pub struct DocumentView {
    buffer: GapBuffer,
    line_index: LineIndex,        // 见 N1.b
    viewport: Viewport,
    metadata: FileMetadata,
    file_path: Option<PathBuf>,
}

impl DocumentView {
    pub fn visible_lines(&self) -> impl Iterator<Item = &[u8]> { ... }
}
```

#### N1.b 行索引（衍生）

按行号取行需要：要么每次 `lines_fwd` 全文扫到目标行（O(file_size)），要么建行索引。

**起步方案**：建一个 `Vec<usize>`（每个元素是行起 byte offset），加载完成时一遍 `lines_fwd` 填好。50 MB ASCII 约 100 万行 = 8 MB 索引（< RSS 5%），可接受。

后续若要支持插入/删除：换成 piece-tree 的行 cache 或 `BTreeMap`。先简单。

**验收**：
- `cargo bench bench_open_50mb_ascii` P95 < 80 ms
- `RSS_after_open(50MB) - RSS_baseline < 60 MB`
- 50 MB CJK 文件能跳到任意行，跳行 < 1 ms

---

### N2 🔴 每帧重 shape 全部可见行 + 持续 redraw

**位置**：`crates/app/src/app.rs:243-299`（`shape_visible_lines`）和 `app.rs:452-456`（`about_to_wait`）

```rust
fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
    if let Some(window) = &self.window {
        window.request_redraw();   // 无条件，每个空闲循环都触发
    }
}
```

```rust
fn shape_visible_lines(&mut self) -> Vec<GlyphVertex> {
    for (i, line) in visible.iter().enumerate() {
        let shaped = match text.shaper.shape(line) {  // 每帧每行 shape
```

**问题**：
1. `about_to_wait` 无条件 `request_redraw()` → 永远 100% 重绘 idle 场景。plans.md 阶段 3 验收 idle CPU < 0.5% 直接报废
2. 每帧 shape：50 行 × 60 fps = 3000 次 shape/秒；叠加 M2 未修（`Buffer::new` 每次），cosmic-text line cache 一次都用不到
3. 行内容不变时根本不需要重新 shape

**修复**：
- 引入"脏"标记：`needs_redraw: bool`，仅在 `Resized/Scroll/Edit/Resumed` 时置位
- `about_to_wait` 改成无操作；`request_redraw` 只在事件回调里触发
- shape 结果按行 cache：`HashMap<u64 /* line content hash */, Vec<GlyphCluster>>`，行未变直接复用
- 脏行算法：内容变化（generation） / 视口变化时局部失效

**验收**：
- 空闲 30 s：Activity Monitor `edit-plus` 进程 CPU 平均 < 1%
- 滚动 1 行触发 1 次 shape（仅新进入视口的那行），不是全部行

---

### N3 🔴 atlas 是空壳，根本没光栅化 glyph

**位置**：`crates/app/src/app.rs:130-158`（`init_text`）和 `app.rs:271-295`（`shape_visible_lines` 内构造 GlyphSlot）

```rust
// init_text 里
let atlas_texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
    size: wgpu::Extent3d { width: 1, height: 1, ... },         // 1×1 纹理
    ...
});
gpu.queue.write_texture(..., &[255u8, 255, 255, 255], ...);    // 一个白像素

// shape_visible_lines 里
for cluster in &shaped.clusters {
    let advance = cluster.advance.max(1.0);
    let slot = GlyphSlot {
        x: 0, y: 0,
        width: advance.ceil() as u32,                          // 当矩形宽度
        height: LINE_HEIGHT.ceil() as u32,
        ...
    };
    glyph_positions.push((slot, x_cursor, y_base));
}
```

**问题**：
1. atlas 是 1×1 白像素纹理，根本没 glyph bitmap
2. 渲染出来是一堆**白色矩形条**，不是文字
3. plans.md 阶段 4 验收 "光栅化正确（含 emoji/CJK）"+ golden image SSIM ≥ 0.99 完全过不了
4. `render::GlyphAtlas` / `GlyphRenderer` / `GlyphVertex` 都写了，但 app 这层完全绕过去自己拼了一套

**修复（拆三步）**：

**N3.a swash 接入**：在 `shaping` crate 加 `swash = "0.1"` 依赖（cosmic-text 已经间接依赖），用 `ScaleContext` 把 cosmic-text 给的 `(font_id, glyph_id, font_size, subpixel_phase)` 光栅化为 alpha mask（Vec<u8>，宽×高）。

**N3.b atlas 真接进来**：
- `app::TextState` 持有 `GlyphAtlas`（`render::GlyphAtlas`），动态 1024×1024 纹理（首发 1 页够用）
- 渲染流程：
  ```
  for cluster in shaped.clusters:
      key = GlyphKey { glyph_id, font_size, subpixel_phase }
      slot = atlas.get(key) or
             { rasterize -> swash mask -> upload -> atlas.insert -> slot }
      tex_coords = slot 在 atlas 内的归一化 UV
      位置 = pen + cluster.x_offset + slot.bearing_x
      append vertex
  ```
- atlas 上传走 `queue.write_texture` 或 staging buffer

**N3.c shader 改 alpha 采样**：
- atlas 纹理格式从 `Rgba8UnormSrgb` 换成 `R8Unorm`（alpha mask 就够）
- fragment shader：`color = vertex_color * vec4(1, 1, 1, sample(atlas).r)`
- 现有 `BlendState::ALPHA_BLENDING` 不变

**验收**：
- 真渲染出 "Hello, edit+ — 世界 👨‍👩‍👧"
- `app/tests/render_smoke.rs::render_hello_to_png` 与 golden image SSIM ≥ 0.99
- 首帧 atlas 占用 < 256 KB

**注意**：N3 工作量最大（半天-1 天），是阶段 4 真正的核心。M2（Buffer 复用）也要在这一步顺手做掉。

---

## 3. 新发现中等问题

### N4 🟡 `icu.rs::sys` 缺两个函数

**位置**：`crates/core/src/icu.rs:609,619`

```rust
#[cfg(edit_icu_renaming_auto_detect)]
let suffix = sys::icu_detect_renaming_suffix(&scratch_outer, icu.libicuuc);
...
#[cfg(edit_icu_renaming_auto_detect)]
let name = sys::icu_add_renaming_suffix(&scratch, name, &suffix);
```

`mod sys` 里只搬了 `LibIcu / load_icu / load_library / get_proc_address` 4 个，缺 `icu_detect_renaming_suffix` 和 `icu_add_renaming_suffix`。原始定义在 `edit/crates/edit/src/sys/unix.rs:430,488`。

cfg 在 macOS 不启用所以编译过；一旦 build.rs 设置（Linux 自定义 ICU 后缀场景），立刻 broken。

**修复**：从 `edit/sys/unix.rs:430-540` 抄两个函数到 `core/src/icu.rs::mod sys`，连同它们依赖的 `BString / BVec / arena_format`。

---

### N5 🟡 `icu.rs::mod sys` 缩进 fmt 不一致

**位置**：`crates/core/src/icu.rs:14-67`

```
$ cargo fmt --check
Diff in icu.rs:12:
 mod sys {
-use std::ffi::{CStr, c_char, c_void};
+    use std::ffi::{CStr, c_char, c_void};
```

`fmt --check` exit 0（仅警告），但下次 `cargo fmt` 自动改 → 一个未提交 diff。CLAUDE.md "每次提交保证编译过 + fmt 干净"。

**修复**：直接 `cargo fmt --all`，提一次。

---

### N6 🟡 `detect_line_ending` 与 `scan_line_endings` 双轨

**位置**：`crates/core/src/file.rs:54-81`（test-only fn）vs `file.rs:161-177`（生产 fn）

`detect_line_ending` 仅 `#[cfg(test)]`，代码逻辑与 `scan_line_endings` 重复一遍。测试不该自己写第二份业务实现。

**修复**：删 `detect_line_ending`，测试直接调 `scan_line_endings(bytes, &mut has_lf, &mut has_cr, &mut has_crlf)` 然后断言 `(has_lf, has_crlf, has_cr)`。或者把 `scan_line_endings` 重构成返回 `LineEnding`，生产路径调它，测试也调它。

---

### N7 🟢 `Viewport::scroll_up` 没 clamp

**位置**：`crates/app/src/viewport.rs:49-51`

```rust
pub fn scroll_up(&mut self, delta: usize) {
    self.scroll_line = self.scroll_line.saturating_sub(delta);
    // 缺 self.clamp();
}
```

正常路径下 `saturating_sub` 够用，但若 viewport 因 resize 变得比文档大、之前 scroll 到底，再 scroll_up 不会触发底端 clamp，与 `scroll_down/scroll_to/resize` 行为不一致。

**修复**：末尾加 `self.clamp();` 一行。

---

### N8 🟢 hashlink 没进 workspace.dependencies

**位置**：`crates/shaping/Cargo.toml`

`shaping` 直接 `hashlink = "0.10"`；workspace.dependencies 里没列。`render` 也该用 hashlink（M4），到时会再写一份。统一管版本更稳。

**修复**：把 `hashlink = "0.10"` 加进 workspace.dependencies；shaping/render 都 `hashlink.workspace = true`。

---

### N9 🟢 plans.md §7 抄码 checklist 与现实严重不符

| 文件 | checklist 标记 | 实际 |
|---|---|---|
| `cell.rs` | `n` | 抄了（合法工具，应改 `y`） |
| `lsh/cache.rs` / `lsh/highlighter.rs` / `lsh/definitions.rs` | `y` | 全部 deferred 到阶段 13 |
| `core/src/file.rs` | 不在表里 | 新写（应作为"自研"列入） |
| `app/src/cli.rs` | 不在表里 | 新写 |
| `app/src/gpu.rs` | 不在表里 | 新写 |
| `app/src/viewport.rs` | 不在表里 | 新写 |
| `app/src/document_view.rs` | 不在表里 | 新写（且 N1 要重写） |

**修复**：plans.md §7 把 lsh/* 备注改成 "defer to stage 13"；新增"自研文件"小节列 file.rs/cli.rs/gpu.rs/viewport.rs/document_view.rs。

---

## 4. 整体方向问题

### D1 阶段越线

plans.md 把阶段 4（cosmic-text 接入 + 静态文本渲染）和阶段 5（只读显示文件）切开是有原因的——
阶段 4 的核心是 **"光栅化正确，golden image SSIM ≥ 0.99"**。

当前实现跳过 N3（真 atlas + glyph 光栅化），用伪 atlas 拼了个能跑的版本，
直接进入了"显示一个文件"的形式，但显示的是白矩形条。这是 CLAUDE.md "三轮还没修好该推翻"
的反模式：先把假的能跑版本拼上，把问题往后推。

### D2 修复建议

**冻结 app 层往前推**，按下面顺序回到正轨：

| 优先级 | 项 | 估时 |
|---|---|---|
| P0 | M2（Buffer 复用） + N3（真 atlas + swash 光栅化） | 6–8 h |
| P0 | N1（DocumentView 用 GapBuffer + LineIndex） | 3–4 h |
| P0 | N2（脏标记 + shape 行 cache） | 2 h |
| P1 | M6（app 错误处理切到 gpu.rs） | 30 min |
| P1 | M4 + M5（atlas O(1) LRU + oversized） | 1 h |
| P1 | M1（cache 复合 key） + M3（删 Default） | 30 min |
| P2 | N4 + N5 + N6 + N7 + N8 + N9 | 1 h |

合计约 14–18 小时。可分 3 天做。

---

## 5. 验收清单

修完后逐条勾。

### 5.1 不准退化的 v1 验收（重跑）

```sh
# 必须空输出
rg "use crate::(framebuffer|cell::|vt|tui|input|terminal_stubs|HighlightKind|Highlighter|Framebuffer|Clipboard|Language|IndexedColor)" crates/core/src/

# 必须 0 warnings
cargo build --workspace 2>&1 | grep -i warning

# 必须全绿
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

### 5.2 v2 新增门槛

```
[ ] N1: app 层无 std::fs::read_to_string 直接读用户文件
        → rg "fs::read_to_string|read_to_string\(" crates/app/src/ 输出空
[ ] N1: DocumentView 持有 GapBuffer，不持 Vec<String>
[ ] N1: 50 MB ASCII 文件加载后 RSS 增量 < 60 MB（实测 / `ps -o rss`）
[ ] N2: idle 30s 平均 CPU < 1%（Activity Monitor）
[ ] N2: 滚动 1 行只触发新行 shape，rg/插桩验证
[ ] N3: 屏幕真显示文字（非白矩形）
[ ] N3: app/tests/render_smoke 与 golden 比较 SSIM ≥ 0.99
[ ] M2: Shaper 复用 Buffer（grep "Buffer::new" in shape() = 0）
[ ] M4: atlas LRU O(1)（hashlink::LinkedHashMap 进 GlyphAtlas）
[ ] M6: app::init_window 无 expect/unwrap，全部 ? 上抛
[ ] M8: cfg(edit_icu_renaming_auto_detect) 下能编译过
[ ] N9: plans.md §7 checklist 已对齐现实
```

### 5.3 阶段 4 / 阶段 5 真验收（plans.md 抄过来跑）

```
[ ] cargo bench bench_open_50mb_ascii P95 < 80 ms
[ ] cargo bench bench_open_50mb_cjk P95 < 200 ms
[ ] cargo bench bench_scroll_60s_60fps 丢帧 < 1%
[ ] 手动 M5.1–M5.7 全部通过（docs/manual_test_protocol.md §5）
```

---

## 6. 验收记录

待修复后填入。每勾一条记下日期 + 实测数字。

```
[ ] v1 验收重跑（日期：____）
[ ] v2 §5.2 新增门槛（日期：____）
[ ] 阶段 4 真验收（日期：____）
[ ] 阶段 5 真验收（日期：____）
```
