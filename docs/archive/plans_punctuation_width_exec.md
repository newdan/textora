# 标点符号最小宽度 — 执行计划

## 代码现状与数据流

```
Shaper::shape_fast() / shape()
  → ShapedRun { clusters: Vec<GlyphCluster> }  // 每个 cluster 有 advance, x_offset, byte_range
    → compute_visual_lines(&clusters)           // 用原始 advance 计算换行断点
    → build_advance_cache_entries(&clusters)    // 用 advance 构建光标映射
    → 渲染循环 cluster_advance(cluster.advance) // 用 advance 放置字形
```

关键发现：
- `compute_visual_lines` 在 `reshape_worker.rs`（后台）和 `render_pipeline.rs`（前台）**两处**调用
- 后台 worker 只算换行断点，不做渲染——换行断点应使用**原始** advance（避免因 padding 导致过早换行）
- 前台 `render_pipeline.rs` 的 `shape_visible_lines` 在一次调用中完成：shape → 算 visual lines → 构建 advance cache → 渲染 → 填充缓存
- `CachedLine.cluster_data` 存储 advance，缓存命中时直接复用

## 实施步骤

### Step 1: 在 Settings 中增加配置项

**文件**: `crates/ui/src/settings.rs`

在 `Settings` struct 中增加字段：

```rust
/// 标点符号的最小宽度比例（相对于 font_size/em）。
/// 0.5 表示标点至少占半个 em 宽度，0.0 表示不启用。
pub min_punctuation_width_ratio: f32,
```

默认值 `0.5`。同时在 `set_font_size` 中无需特殊处理（它不改变 ratio）。

### Step 2: 实现 padding 函数

**文件**: `crates/ui/src/layout.rs`

新增函数 `apply_punctuation_padding`：

```rust
/// 对标点符号 glyph 进行最小宽度补齐。
/// 对于 advance 小于 em_width * min_ratio 的标点 cluster：
///   - 扩大 advance 到最小值
///   - 调整 x_offset 使字形在扩宽后的空间内居中
pub fn apply_punctuation_padding(
    clusters: &mut [shaping::GlyphCluster],
    line_bytes: &[u8],
    em_width: f32,
    min_ratio: f32,
)
```

实现要点：
- 遍历 clusters，通过 `byte_range` 从 `line_bytes` 取原始字节
- 用 `std::str::from_utf8` 解码，取首个 char
- 判断标点：`ch.is_ascii_punctuation() || ch.is_punctuation()`（`unicode_categories` 已在依赖中）
- 跳过 whitespace cluster（ws 的 advance 由 `ws_cluster_advance` 单独控制）
- 若 `advance < em_width * min_ratio`，计算 `extra = min_advance - advance`，设置 `advance = min_advance`，`x_offset += extra / 2.0`

### Step 3: 在渲染管线中接入

**文件**: `crates/app/src/render_pipeline.rs`

在 `shape_visible_lines` 函数中，**shape 之后、visual lines 确定之后、advance cache 构建之前**，调用 padding。

具体插入点（约 line 605，`compute_visual_lines` 调用之后）：

```rust
// 在 visual_lines 确定后，对 punctuation 进行最小宽度补齐
// 换行断点已用原始 advance 算好，此处只影响渲染和光标
let em_width = text.shaper.font_size();
let min_ratio = Settings::with(|s| s.min_punctuation_width_ratio);
if min_ratio > 0.0 {
    apply_punctuation_padding(&mut shaped.clusters, &line_bytes, em_width, min_ratio);
}
```

此插入点覆盖：
- `build_advance_cache_entries`（光标映射）
- 渲染循环（字形放置）
- 缓存填充 `CachedLine.cluster_data`（后续缓存命中自动使用 padded advance）

缓存命中路径无需修改：`CachedLine.cluster_data` 中已存储 padded 后的 advance。

**不需要修改** `reshape_worker.rs`——后台 worker 只计算换行断点，应使用原始 advance。

### Step 4: 测试

**文件**: `crates/ui/src/layout.rs`（在现有 `#[cfg(test)] mod tests` 块中新增）

| 测试 | 验证点 |
|------|--------|
| `punct_padding_comma_colon` | `,` 和 `:` 的 advance 被补齐到 `>= em * 0.5` |
| `punct_padding_narrow_letters_untouched` | `i`、`l`、`t` 等窄字母 advance 不受影响 |
| `punct_padding_x_offset_centering` | 补齐后 `x_offset == extra / 2.0` |
| `punct_padding_ratio_zero_disabled` | `min_ratio=0.0` 时不做任何修改 |
| `punct_padding_whitespace_skipped` | 空格 cluster 不被当作标点处理 |

**文件**: `crates/ui/src/render_geom.rs`（在现有 tests 中新增）

| 测试 | 验证点 |
|------|--------|
| `byte_to_x_with_padded_punctuation` | 模拟 padded 后的 cluster 数组，验证 `byte_to_x` 在标点前后半部分的映射正确 |

## 影响范围

- **不修改**: shaping crate, reshape_worker, viewport, decorations, app 业务层
- **轻量修改**: settings（+1 字段）、layout（+1 函数）、render_pipeline（+5 行调用）
- **向后兼容**: 默认 `min_punctuation_width_ratio = 0.5`，设为 `0.0` 可完全禁用

## 潜在风险

1. **视觉线宽不一致**: `compute_visual_lines` 返回的 `pixel_width` 基于原始 advance，实际渲染宽度略大。由于标点通常极窄（3-5px），差异在 2-4px 范围，不会导致内容溢出 viewport。
2. **x_offset 累加**: 如果某个 cluster 被多次调用 padding（理论上不会），x_offset 会错误累加。在调用处确保只在 shape 后调用一次。
3. **缓存失效**: padding 后 `content_hash` 不变，但缓存中的 `cluster_data` 存储了 padded advance。由于 padding 是确定性的（同样输入→同样输出），这不是问题。
