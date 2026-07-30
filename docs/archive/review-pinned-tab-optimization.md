# Pinned Tab 优化审查报告

## 一、需求覆盖度

### Gemini 原始需求 vs 实现

| 需求 | 状态 | 说明 |
|------|------|------|
| 1. `close_area` → 更小的右侧 padding | ✅ | `pinned_right_pad = 12.0 * dpi` |
| 2. `max_tab_w` 限制为 ~160px | ✅ | `pinned_max_tab_w = 160.0 * dpi` |
| 3. Pin indicator 间距改善 | ✅ | `pin_pad = 10.0 * dpi` 加入文本起点 |
| 4. Pin indicator 美化（pill shape） | ✅ | 三段式胶囊形 |

### 方案文档额外需求 vs 实现

| 需求 | 状态 | 说明 |
|------|------|------|
| 5. `pinned_min_tab_w` 降低 | ✅ | `30.0 * dpi`（原 40px） |
| 6. `pinned_total_width` 减 trailing gap | ✅ | `pinned_width - gap` |
| 7. clip 区域收紧 | ✅ | `4.0*dpi` → `3.0*dpi` |

**结论：两个文档的所有需求均已实现。**

---

## 二、测试覆盖度

### 现有测试（未随本次改动新增）

| 测试 | 覆盖范围 |
|------|----------|
| `disambig_*` (6 个) | 文件名消歧 |
| `indicator_*` (3 个) | TabIndicator 状态 |
| `context_menu_*` (4 个) | 右键菜单 |
| `max_scroll_*` / `scroll_*` (5 个) | 滚动行为 |
| `hit_test_*` (5 个) | 点击测试 |
| `is_tab_in_clip_*` (3 个) | clip 边界 |
| `mouse_*` / `autoscroll_*` (6 个) | 交互事件 |

### 缺失测试（本次改动未覆盖）

| 缺失测试 | 优先级 | 说明 |
|----------|--------|------|
| pinned tab 宽度 < 普通 tab 宽度 | **高** | 验证 `pinned_right_pad` / `pinned_min_tab_w` 生效 |
| `pinned_total_width` 无 trailing gap | **高** | 验证 pinned 末尾无多余 gap |
| pinned tab 不响应 close 按钮 | **中** | hit_test 中已有，但未显式验证 pinned 分支 |
| pinned + dirty 同时存在的文本起点 | **中** | 验证 `pin_pad + indicator_pad` 叠加正确 |
| 多 pinned tab separator 位置 | **低** | 纯视觉，难以单测 |
| 2x DPI 下宽度计算 | **低** | 需要不同 DPI 环境 |

**结论：现有测试均为已有功能，本次改动没有新增测试。建议至少补充前 2 个高优先级测试。**

---

## 三、代码冗余

### 问题 1：`is_pinned` 重复计算（layout.rs:307-319）

```rust
// 第一次：显式计算
let is_pinned = pinned_indices.contains(&i);
let (right_pad, eff_min, eff_max) = if is_pinned { ... };

// 第二次：push 到 tab_widths 时又调用
tab_widths.push(TabWidth {
    ...
    pinned: pinned_indices.contains(&i),  // 重复
});
```

**建议**：`pinned: is_pinned`

### 问题 2：`pinned_right` 重复计算（state.rs:268, 302）

```rust
// 第一次（pinned clip）
let pinned_right = layout.tabs[lp].rect_px.right() + 3.0 * dpi;

// 第二次（non-pinned clip）
.map(|lp| layout.tabs[lp].rect_px.right() + 3.0 * dpi)
```

两次计算完全相同。**建议**：提取为 `let pinned_right` 统一变量，在 `if let Some(lp)` 块内计算后复用。

### 问题 3：magic numbers 散落

`3.0 * dpi`、`10.0 * dpi`、`12.0 * dpi`、`30.0 * dpi`、`160.0 * dpi` 分别在 layout.rs 和 state.rs 中硬编码。其中 `3.0 * dpi`（separator 宽 2px + 1px 间距）和 `10.0 * dpi`（pin_pad）在两个文件中隐式耦合。

**建议**：在 `types.rs` 或 `layout.rs` 顶部定义常量：
```rust
const PIN_RIGHT_PAD: f32 = 12.0;  // multiplied by dpi at use site
const PIN_MIN_TAB_W: f32 = 30.0;
const PIN_MAX_TAB_W: f32 = 160.0;
const PIN_INDICATOR_PAD: f32 = 10.0;
const PIN_GROUP_SEP_PAD: f32 = 3.0; // separator width(2) + margin(1)
```

---

## 四、安全漏洞

**无安全漏洞。** 本次改动仅涉及：
- 布局数值计算（无溢出风险，`f32` 范围足够）
- 渲染坐标偏移（无用户输入注入）
- 不涉及文件 I/O、网络、unsafe 代码

---

## 五、性能缺陷

| 项目 | 评估 |
|------|------|
| pill shape 3 次 `dl.fill` vs 原来 1 次 | **可忽略**。tab 数量少（<50），每帧多 2 次 fill 无感知 |
| `pinned_indices.contains(&i)` 在循环内调用 | **已有行为**，本次未引入新开销 |
| `compute_disambiguation` 每帧调用 | **已有行为**，未改变 |

**无新增性能缺陷。**

---

## 六、总结

| 维度 | 评价 |
|------|------|
| 需求覆盖 | ✅ 100% — 两个文档所有需求均已实现 |
| 测试覆盖 | ⚠️ 缺少 pinned 专用测试，建议补充 2 个高优先级用例 |
| 代码冗余 | ⚠️ 3 处小冗余，不影响正确性，建议清理 |
| 安全性 | ✅ 无问题 |
| 性能 | ✅ 无问题 |
