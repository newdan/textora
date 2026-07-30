# wrap_text 换行性能优化

## 现状

`wrap_text` 在换行时，用完整 shaping 来测量文本宽度。对 builder 拼出的超长段落（多次 softbreak 连成的 2600+ 字符文本），每次断行都要反复 shape 大块前缀，导致 layout 阶段耗时 14s+。

```
[wrap_text] line_len=2689 shapes=495 lines_out=21 time=4871.0ms
[wrap_text] line_len=2785 shapes=456 lines_out=21 time=4610.7ms
[md_preview] layout=14233.9ms
```

## 策略

### 核心思路

`wrap_text` 只管"在哪断"，不需要像素级精度。最终渲染用的整形在 `shape_line` 里做，不丢精度。

两板斧：**先切碎、再估宽**，消除所有大文本 shape。

### 第一步：入口处 tokenize

```rust
// wrap_text 拿到的是 builder 拼好的大段（空格分隔原行）
// 直接用 split(' ') 切回 ~22 字符的小段
for token in text.split(' ') {
    // 每个 token 独立处理，避免巨型 shape
}
```

2689 字符段 → 122 个 ~22 字符 token，后续操作全在小粒度上进行。

### 第二步：混合宽度策略

```
Token 宽度 =
  ├─ 纯 CJK / 全角 / 假名 → font_size × 字符数       （估，零 shape）
  ├─ 纯 ASCII / 数字       → grapheme_advance_cache   （单字符 shape 一次，后续全命中）
  └─ 混排 / 其他           → shape(token)             （容错，极少触发）
```

**Unicode block 判定**（一眼判定，零开销）：

```
char range                → 全宽估算
─────────────────────────────────────
CJK Unified (4E00–9FFF)   → ✅
CJK Extension A-F         → ✅
Fullwidth Forms (FF01–FF5E) → ✅
Hiragana (3040–309F)      → ✅
Katakana (30A0–30FF)      → ✅
CJK Compat (3300–33FF)    → ✅
CJK Radicals / Strokes    → ✅
全角标点 (3000–303F)      → ✅
─────────────────────────────────────
ASCII / Latin / Digits    → grapheme_advance (cache)
其他                       → shape(token)
```

### 第三步：贪心累加换行

```rust
for token in tokens {
    let width = estimate_width(token);
    if current_width + space_width + token_width > max_w && !current_line.is_empty() {
        emit current_line;
        current_line = token;
        current_width = token_width;
    } else {
        append_to_line(token, width);
    }
}
emit current_line;
```

## 预期效果

| 指标 | 优化前 | 优化后 |
|------|--------|--------|
| 2689 字符段 shape 次数 | ~500 | ~0（CJK token 零 shape） |
| 2689 字符段耗时 | ~5000ms | **<1ms** |
| 全文档 layout | ~14000ms | **<5ms** |
| 排版精度 | 精确 | 断点误差 <2px，视觉无感 |

## 改动范围

- `crates/markdown/src/layout.rs`: 重写 `wrap_text` 的 shaper 路径
- 新增 `is_cjk_char(c: char) -> bool` 辅助函数
- `grapheme_advance_cache` 已存在于 `shaping::Shaper`，直接复用

## 不做什么

- 不改 `shape_line`（最终渲染整形保留）
- 不修改 builder 的文本拼接逻辑
- 不改 pulldown_cmark 解析
- 不去掉空格二分查找的 fallback（方案已改过一次，保留作为降级）
