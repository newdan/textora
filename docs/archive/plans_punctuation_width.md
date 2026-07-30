# 标点符号最小宽度（Punctuation Minimum Width）实施方案

## 1. 背景与目标
在支持比例字体（窄字符如 `t/l/i` 保持较窄宽度）的前提下，英文标点（如 `:`、`,`、`'`）由于字形本身极窄，会导致代码可读性下降，甚至在光标定位和高亮时显得局促。
**目标**：保持普通字符（包含窄字母）的原始排版宽度，仅针对“过窄的标点符号”实施最低宽度保护（Minimum Width Padding），使其在视觉上更为舒展，并在内部坐标映射中占用更合理的空间。

## 2. 架构设计

根据 `AGENTS.md` 中的架构约定，我们将在底层的排版和几何计算层进行修改，而不影响业务层的逻辑。
*   **依赖模块**：`crates/shaping`（假设这是负责调用 HarfBuzz/CosmicText 的地方），以及 `crates/ui/render_geom`（负责 `byte→pixel` 映射的 `AdvanceCacheEntry` 计算）。
*   **核心思路**：**Post-shaping Adjustment（排版后修正）**。
    在排版引擎（Shaping）生成了一行文本的 `Glyphs`（字形列表）之后、存入缓存（`AdvanceCacheEntry`）之前，插入一道“标点符号宽度补齐”的处理工序。

## 3. 详细实施阶段 (Phases)

### 第一阶段：在 UI 配置中增加最小标点宽度定义
在 `crates/ui/settings` 中，定义标点符号的最小宽度占比（相对于 `em_width`）。

```rust
// crates/ui/src/settings.rs
pub struct FontSettings {
    // ...
    /// 标点符号的最小宽度比例（例如 0.5 表示至少占据半个标准字符宽度）
    pub min_punctuation_width_ratio: f32, 
}

impl Default for FontSettings {
    fn default() -> Self {
        Self {
            // ...
            min_punctuation_width_ratio: 0.5, // 推荐设为 0.5，即半角宽度
        }
    }
}
```

### 第二阶段：在 Shaping 后置处理中补齐字宽
在调用底层 shaping 库获得字形数组后，遍历字形，识别标点并修改 `advance`（步进）和 `x_offset`（绘制偏移）。

利用项目中已有的 `unicode_categories` crate 来判断标点符号，或者优先针对 ASCII 标点。

```rust
// 伪代码，位于 crates/shaping 或 crates/ui/layout 中负责生成 AdvanceCacheEntry 的地方

use unicode_categories::UnicodeCategories;

pub fn apply_punctuation_padding(
    glyphs: &mut [Glyph], 
    text: &str, 
    em_width: f32, 
    min_ratio: f32
) {
    let min_punct_advance = em_width * min_ratio;

    for glyph in glyphs.iter_mut() {
        // 通过 glyph 的 byte_offset 获取对应的字符
        if let Some(ch) = text[glyph.byte_offset..].chars().next() {
            // 判定是否为标点符号 (也可以针对特定符号白名单如 ':', ',', '.', '\'')
            if ch.is_ascii_punctuation() || ch.is_punctuation() {
                if glyph.advance < min_punct_advance {
                    let extra_width = min_punct_advance - glyph.advance;
                    
                    // 1. 扩大该字形的占位宽度，影响后续字符的 x 坐标和光标映射
                    glyph.advance = min_punct_advance;
                    
                    // 2. 将字形在扩宽后的空间内【居中绘制】
                    // 假设原字形起始 x_offset 为 0，现在为了居中，向右平移 extra_width 的一半
                    glyph.x_offset += extra_width / 2.0; 
                }
            }
        }
    }
}
```

### 第三阶段：修改 `render_geom` 与 `AdvanceCacheEntry`
确保修改后的 `advance` 顺畅无误地写入到 `AdvanceCacheEntry` 缓存中。
*   `ui::render_geom` 在计算光标位置（`x_for_index`）和点击测试（`index_for_x`）时，**必须直接读取已被扩大的 `advance`**。
*   由于我们在上一阶段直接修改了 `glyph.advance`，只要 `AdvanceCacheEntry` 是根据修正后的 `glyphs` 数组来生成的，这一步通常自然兼容，无需大改。但需要增加单元测试确保光标不会卡在 `extra_width` 的空白处。

### 第四阶段：测试用例覆盖 (Testing)

针对 `ui::render_geom` 和 `ui::layout` 编写单元测试：
1. **普通窄字符测试**：验证 `'i'`、`'l'` 等字符的 `advance` 没有被拉长。
2. **标点拉伸测试**：给定一段包含 `a,b:c` 的文本，断言 `','` 和 `':'` 的 `advance` 至少为 `em_width * 0.5`。
3. **居中对齐测试**：断言标点符号的 `x_offset` 能够使其在新 `advance` 单元格中居中。
4. **光标点击边界测试（Edge Case）**：验证在这个变宽的标点符号的前半部分点击时，光标能正确落在标点前；在后半部分点击时，能落在标点后。

## 4. 该方案的优势
1. **保留了自然比例阅读**：只有因为字体缺陷或天生极窄的标点会被干预，`t/l/i` 仍然保持紧凑，不会有 VS Code 强制等宽那种丑陋的“大牙缝”。
2. **不破坏原有组件**：完全在底层数据流水线中解决问题，`ui::viewport`、`ui::decorations` 和 `crates/app` 层的业务逻辑无需做任何修改，对上层透明。
3. **视觉美观**：通过 `glyph.x_offset += extra_width / 2.0`，不仅修复了太窄的问题，还让标点符号优雅地“居中”在补宽后的空间里，提升了代码的呼吸感。
