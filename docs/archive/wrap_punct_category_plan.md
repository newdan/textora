# is_punct_char Unicode Category 改造 — 审计报告

> 日期：2026-06-09
> 提交：3fa7a3c (category), 91a3661 (loop fix)

---

## 一、需求实现度

| 需求 | 状态 | 说明 |
|------|------|------|
| 用 Unicode General Category 替代手工码点枚举 | ✅ | Pe/Pf/Po/Pi/Ps/Pd 全部覆盖 |
| 开括号/开引号允许在行首 | ✅ | Ps + Pi → false |
| 闭括号/闭引号/其他标点禁止在行首 | ✅ | Pe + Pf + Po → true |
| 破折号允许在行首 | ✅ | Pd → false（`—` `〜` 放行） |
| 连续标点不回退到一半 | ✅ | while 循环持续回退 |
| 新增依赖 `unicode_categories` | ✅ | 0.1.1，零开销纯表 |

**结论：需求全部实现。**

---

## 二、is_punct_char 完整性审查

### 2.1 已覆盖类别

| 类别 | 含义 | 字符示例 | 行为 | 正确？ |
|------|------|----------|------|--------|
| Ps | Open | `(` `[` `{` `「` `『` `【` `《` `〈` `（` | `false` | ✅ |
| Pe | Close | `)` `]` `}` `」` `』` `】` `》` `〉` `）` | `true` | ✅ |
| Pi | Initial quote | `"` `'` `«` | `false` | ✅ |
| Pf | Final quote | `"` `'` `»` | `true` | ✅ |
| Po | Other | `、` `。` `，` `：` `；` `！` `？` `…` `·` `％` `′` `″` etc. | `true` | ✅ |
| Pd | Dash | `-` `–` `—` `〜` `～` `〰` | `false` | ✅ |

### 2.2 未覆盖（非标点）类别 — 正确放行

| 类别 | 示例 | 行为 | 正确？ |
|------|------|------|--------|
| Pc | `_` underscore | `false` | ✅ 下划线在行首没问题 |
| Sc | `$` `¥` `€` | `false` | ✅ 货币符号在行首没问题 |
| Sk | `^` `ˋ` | `false` | ✅ |
| So | `©` `™` `〽` | `false` | ✅ |

### 2.3 边界 case

| 字符 | Unicode | 旧行为 | 新行为 | 影响 |
|------|---------|--------|--------|------|
| `—` | U+2014 Pd | `true`（枚举） | `false`（Pd） | 破折号允许在行首——更合理 |
| `〜` | U+FF5E Pd | `true`（枚举） | `false`（Pd） | 同上 |
| `〰` | U+3030 Pd | `true`（枚举） | `false`（Pd） | 同上 |
| `·` | U+00B7 Po | `true`（枚举） | `true`（Po） | 一致 |
| `％` | U+FF05 Po | `true`（枚举） | `true`（Po） | 一致 |
| `‰` | U+2030 Po | 未覆盖→`false` | `true`（Po） | **新增覆盖** |
| `′` `″` | U+2032/2033 Po | 未覆盖→`false` | `true`（Po） | **新增覆盖** |
| `※` | U+203B Po | 未覆盖→`false` | `true`（Po） | **新增覆盖** |
| `〽` | U+303D So | 未覆盖→`false` | `false`（So） | 一致 |

---

## 三、punct while 循环审查

### 3.1 正确性

```
while break_at > start {
    // 从 break_at 向前扫描连续标点
    // 如能吞下 → break（标点留在当前行）
    // 如不能吞 → break_at -= 1（回退一个字符，继续循环）
    // 如 break_at 处无标点 → break（终止）
}
```

**终止性**：`break_at` 每轮递减，`break_at > start` 有下界 → 必定终止。

**正确性**：回退后重新扫描，不会漏掉被回退到的标点字符。

### 3.2 性能

- 内层扫描：O(连续标点数)，标点数通常 1~5，最多 ~20
- 外层循环：最多迭代 连续标点数 次
- 总复杂度：O(N²)，N 为连续标点数
- 实际：N ≤ 5 时 < 15 次 `is_punctuation()` 调用 → **可忽略**
- 极端：N=20 时 ~210 次调用 → 仍远小于主循环开销

**结论：不存在性能缺陷。**

### 3.3 已知局限性

**L1：极端短行** — 回退可能产生极短的行。

示例：`你好！！world`，viewport=80px。
- 回退后：`你` (24px) / `好！！` (72px) / `world` (50px)
- 第一行仅 24px，仅为视口的 30%

**风险等级**：低。仅在极端视口宽度+标点密集时发生，且"标点不在行首"优先于"行宽美观"。

**缓解方式**（未实施，可后续考虑）：回退前检查当前行宽是否已 `< viewport * 0.3`，如是则放弃回退。

**L2：break_at == start** — 若行首就是标点，无法回退。

示例：某行恰好从 `。」` 开始。
- `while break_at > start` → break_at == start → 不进入循环
- 标点出现在行首

**风险等级**：极低。这要求前一行恰好断在标点前（候选逻辑会避免）。实际几乎不会发生。

---

## 四、测试覆盖审计

### 4.1 已有测试（标点相关）

| 测试 | 文件 | 覆盖内容 |
|------|------|----------|
| `ascii_punct_not_alone_at_line_start` | layout.rs | ASCII 标点不出现在行首 |
| `cjk_fullwidth_punct_then_latin_boundary` | render_pipeline_tests | 全角标点后 Latin 边界 |
| `cjk_mixed_punct_several_transitions` | render_pipeline_tests | 多个！！标点回退行为 |
| `latin_to_cjk_via_fullwidth_punct` | render_pipeline_tests | Latin→CJK 经全角标点 |
| `cjk_fullwidth_colon_no_artificial_break_after_punct` | render_pipeline_tests | 冒号不触发假断行 |
| `cjk_comma_between_cjk_no_break` | render_pipeline_tests | CJK 逗号回退 |
| `cjk_consecutive_punct_period_quote` | render_pipeline_tests | **新增** 。”连续标点 |
| `cjk_punct_quote_not_at_line_start` | render_pipeline_tests | **新增** ”不在行首 |

### 4.2 测试缺口

| 优先级 | 场景 | 当前覆盖 | 建议 |
|--------|------|----------|------|
| P1 | `is_punct_char` 直接单元测试 | ❌ 无 | 加 `is_punct_char_category_tests` |
| P2 | Pe 闭括号 `」` `』` `】` 不回退 | ❌ 无 | ——实际上它们会被回退，这是正确行为。应测"它们回退" |
| P2 | Pi 开引号 `"` `「` 不在行首被误伤 | ❌ 无 | 验证 `"` 在行首被允许 |
| P3 | 长标点串 `！！！！！`（5+ 字符） | ❌ 无 | 验证性能+正确性 |
| P3 | 混合开闭 `）文字（` 边界 | ❌ 无 | 开括号在行首 ok，闭括号回退 |
| P3 | 纯符号行（如分隔线 `---`） | ❌ 无 | Pd 类不影响 |
| P4 | `reshape_worker` 中的 `is_punct` | ⚠️ 仅 ASCII | reshape_worker 用 `ch.is_ascii_punctuation()`，不受影响 |

### 4.3 reshape_worker 说明

`reshape_worker.rs` 中的 `is_punct` 是独立逻辑：
```rust
is_punct: ch.is_ascii_punctuation(),
```
仅检查 ASCII 标点——不受此改动影响。`process_fallback` 路径不涉及 CJK 标点处理。**无漏洞**。

---

## 五、漏洞检查

| 检查项 | 结果 | 说明 |
|--------|------|------|
| 标点回退死循环 | ✅ 无 | while 有明确终止条件 |
| 标点越过 start 回退 | ✅ 无 | `break_at > start` guard |
| is_punctuation 误判非标点为标点 | ✅ 无 | `ch.is_punctuation_*()` 来自 Unicode 官方数据 |
| is_punctuation 漏判标点 | ✅ 无 | Pe/Pf/Po 覆盖全部标点类别 |
| 回退后 break_x 计算错误 | ✅ 无 | `trimmed_width(start, break_at)` 始终用当前 break_at |
| trim leading whitespace 吞掉标点 | ✅ 无 | `ws_arr` 检查，标点不是空白 |
| cand_cjk/ws 跳过标点候选 | ✅ 已处理 | 两个候选都有 `is_punct` guard |

---

## 六、总结

| 维度 | 评分 | 说明 |
|------|------|------|
| 需求实现 | ✅ 10/10 | 全部实现 |
| 测试覆盖 | 🟡 7/10 | 核心场景覆盖，缺 `is_punct_char` 单元测试 |
| 正确性 | ✅ 9/10 | 回退循环正确，极端短行是已知 tradeoff |
| 性能 | ✅ 10/10 | O(N²) 但 N 极小，无影响 |
| 代码质量 | ✅ 9/10 | ~10 行替代 ~25 行，清晰 |

**建议后续**：加 `is_punct_char` 直接单元测试（P1），覆盖 Ps/Pi 放行、Pe/Pf/Po 禁止的关键字符。
