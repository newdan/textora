# 提交审查: 2026-06-14 ~ 2026-06-15

审查范围: `994dbd21` → `b7e42ead` (约 35 个提交, 2 天)

---

## 1. Tab Bar — 全面修复与重构 (9ef17033, b7e42ead)

**改动**: 14 文件, +522/-205 (修复); 6 文件, +136/-14 (质量改进)

9ef17033 是这个周期最大的单体修复，解决了 tab bar 的 12 个渲染/交互问题：

| 问题 | 方案 |
|------|------|
| 层级混乱，按钮被 tab 覆盖 | 固定区域（箭头、下拉菜单、+按钮）先画，中间滑动区 clip |
| 标脏符号不一致（sidebar 用 `*`，tab 用 `●`） | 统一用 `*`，pin 用 `|` |
| 下拉菜单高亮使用对钩 | 改为高亮效果（与 macOS 约定一致） |
| 关闭按钮用 Rect 拼 X，视觉效果差 | 直接用 `dl.text()` 画 `x` 字符 |
| 左右箭头用 Rect 拼三角形，锯齿明显 | 改用 `FillTriangle` GPU 原语 |
| Pinned tab 未固定在左侧 | 排查 layout 坐标计算，pinned 不受 scroll 影响 |
| 按钮区背景与 tab 区混为一体 | 加深按钮区背景色 `darken(gutter_bg, 0.75)` |
| 关闭按钮 hover 有背景填充，太重 | 移除填充，仅加深 `x` 颜色 |
| 新建/下拉按钮宽度不统一 | `icon_btn_w = 20dpi` |
| Fade gradient 画在内容之前，文字溢出 | 移到 clip 内部内容之后绘制 |
| winit resumed 回调 panic | `catch_unwind` 包裹 |
| 拖入多文件后 tab 不显示 | dock 重建逻辑修复 |

b7e42ead 后续质量改进：

- 提取 `is_tab_in_clip()` 消除 hit.rs/state.rs 重复逻辑
- 移除 non-pinned 循环中的死代码 `if entry.pinned { continue; }`
- 修正 FillTriangle 裁剪：分别检查四条边，而非 bounding box
- 清理过时注释
- 新增 8 个单元测试（clip 检查、三角形裁剪、dock 重建）

**评估**: 改动大但结构清晰。12 个问题一次性修复有风险，但从测试覆盖（+159 行测试）和质量改进 commit 看，作者做了后续审视。建议关注 FillTriangle 裁剪修正在实际渲染中的表现。

---

## 2. CJK 标点双击断词 (d0280d0c)

**改动**: 3 文件, +154/-14

**根因**: `WORD_CLASSIFIER` 是 256 项 ASCII 表，所有 UTF-8 字节 ≥128 默认归类为 Word。中文标点（，。、；：''【】等）3 字节序列的首字节 >128 → 不被识别为分隔符 → 双击选词跨越中文标点。

**修复**:
- 引入 `unicode_categories` crate
- 新增 `byte_class()`: ASCII 用查表，非 ASCII 解码 UTF-8 码点后用 Unicode General Category 判断
- `skip_class` 按 UTF-8 字符长度推进，避免续字节误分类
- `word_select` 起始 offset 对齐到 UTF-8 字符边界
- 新增 4 个 CJK/日文标点断词测试

**评估**: 修复正确且必要。`unicode_categories` 依赖很轻量。向后兼容性好 — ASCII 路径完全不变。

---

## 3. 双击/三击选词后鼠标抖动导致选区缩水 (a95cf4db)

**改动**: 1 文件, +267/-5

**根因**: `handle_cursor_moved` 不感知 `click_count`，双击选中整词后任何微小鼠标抖动都会触发它，把 cursor 从词尾拉回点击位置，选区缩水为一个字符。

**修复**: `handle_cursor_moved` 根据 click_count 使用不同粒度：
- `click_count == 1`: 字符级（原逻辑不变）
- `click_count == 2`: 词级，snap 到词边界
- `click_count >= 3`: 行级，snap 到行边界

拖拽方向决定 anchor 端（向右拖 anchor=词/行首，向左拖 anchor=词/行尾），与 VS Code/Sublime Text 行为一致。

新增 5 个测试覆盖 regression 和拖拽扩展场景。

**评估**: 设计合理，方向性逻辑（drag 方向决定 anchor）需要确保和已知编辑器行为一致。测试覆盖良好。

---

## 4. Tab 键尊重文件缩进设置 (5825fc3c)

**改动**: 7 文件, +72/-56

**修复内容**:
- Tab 键根据 buffer 的 `tab_size` 和 `indent_with_tabs` 插入，不再硬编码 4 空格
- `is_cjk_char`: 补全 CJK 标点/全角符号范围（U+3000..U+303F, U+FF01..U+FF5E 等）
- `pick_char_width`: 简化逻辑确保 tab 跨行一致
- `ws_cluster_advance`: 全角空格 U+3000 给 2 列宽
- reshape fallback: Tab 字符给 4*ascii_w 对标主路径
- `Shaper::col_width()`: 首次调用量 `'a'` 实际 advance 并缓存，替代 `font_size*0.6` 估算
- `render_pipeline_tests.rs` 中 `col_width` 从 `font_size * 0.6` 改为 `shaper.col_width()`

**评估**: 多个小修复聚合在一个 commit，主题稍分散。`col_width()` 缓存机制是正确的优化。Tab 缩进修复解决了硬编码 4 空格的问题。

---

## 5. Search Bar UX Overhaul (364054b8)

**改动**: 12 文件, +602/-142

搜索栏视觉翻新 + 交互改进：
- `search_bar.rs`: +497/-? 大幅重写渲染逻辑
- 新增 `search_state.rs` (+77 行): 搜索结果导航状态
- 按钮交互 + 滚动定位
- 新增加 `theme.rs` 搜索颜色定义

**评估**: 最大的搜索栏改动。无独立测试文件。建议后续加测试覆盖搜索状态机。

---

## 6. Settings Popup Menu (92db1fdc, 8a068a81, 1afae003 等)

**改动**: ~12 文件, +471/-25

新增功能链条（设计文档 → 实现 → 迭代修复）：
- 设置弹出菜单：主题模式、行号、换行等开关
- View mode 子菜单（Sidebar/Tabs）
- macOS 系统菜单 Settings 子菜单
- Checkmark 替代背景高亮表示激活项
- 分隔线颜色 + 文字对齐修复

**评估**: 功能完整，有设计文档 + 实现计划 + 多次迭代修复，流程规范。

---

## 7. 大规模架构重构 (54047afc, 162f617e, 16d24f31)

### 文件拆分 (54047afc)
**改动**: 19 文件, +6677/-6431

`app.rs` (3345 行) → 拆为 7 个子模块：
- `app_dispatch.rs` (1385 行)
- `app_reshape.rs` (628 行)
- `app_scroll.rs` (195 行)
- `app_search.rs` (124 行)
- `app_tab.rs` (445 行)
- `app_window.rs` (706 行)

`document_view/mod.rs` 拆出 `edit.rs`, `selection.rs`, `visible.rs`
`sidebar/` 拆出 `state.rs`, `types.rs`, `layout.rs`, `menu.rs`, `widget_tests.rs`

### Tab bar 移入 widgets/ (16d24f31)
`ui/src/tab_bar/` → `ui/src/widgets/tab_bar/`，与其他 widget 目录结构统一

### Autoscroll 逻辑移入 TabBarWidget (162f617e)
**改动**: 14 文件, +607/-243

Sidebar 间距、关闭按钮、右键菜单圆角、autoscroll 逻辑从 `workspace.rs` 移入 `TabBarWidget`。

**评估**: 拆分是必要的（3000+ 行文件不可维护），但 6677 行 diff 的纯移动操作 review 难度大。建议后续确保所有 `pub` 可见性正确且无循环引用。

---

## 8. Popup Menu 细节打磨

一系列小提交逐步改进弹出菜单：

| Commit | 改动 |
|--------|------|
| 8d1a7887 | 右键菜单加分隔线 + 阴影圆角 |
| 1edb212b | 阴影用 clip 限制在菜单区域内，修复右下角溢出 |
| e053bd43 | 去掉弹出菜单阴影（最终决定） |
| f00b876f | 激活项用 checkmark 替代背景高亮 |
| 95507184 | 分隔线颜色 + 文字对齐 |
| c92196aa | checkmark 颜色匹配文字 + word wrap 实际生效 |
| 64641d9d | sidebar settings menu + close button hit area |

**评估**: 设计迭代过程可见 — 加了阴影又去掉，加了高亮又改为 checkmark。每个 commit 很小，容易 review。

---

## 9. Bug 修复合集 (a95322ed)

**改动**: 7 文件, +166/-7

"修复多个功能缺陷" — commit 消息为中文笼统描述。具体包含：
- `app.rs` +24: 未详述
- `input.rs` +40: 输入处理修复
- `workspace.rs` +104: 工作区修复
- `icu.rs`: 分行逻辑修复

**评估**: 改动分散在多个文件但使用笼统 commit 消息。建议拆分或写更具体的 body。

---

## 10. IME 预编辑修复 (9e1f66a6)

**改动**: 6 文件, +98/-12

修复 IME 预编辑光标位置和文字遮盖问题，涉及 `render_pipeline.rs` (+63) 的渲染逻辑。

---

## 11. 测试恢复与清理 (df697823, 4ed563c0)

- `df697823`: 恢复 `app_tests.rs`（44 个测试），+1125 行
- `4ed563c0`: 统一 workspace deps，归档旧文档，重命名测试文件（去掉 `test_` 前缀）

---

## 12. 渲染优化 (994dbd21, 21940cd2)

- 渲染循环不再依赖 placeholder VL 估算作为范围上限
- 补充 placeholder VL 估算偏差验证和保守边界测试

---

## 总体评估

### 亮点
- Tab bar 修复非常彻底，12 个问题一次性解决并补充了测试
- CJK 断词修复从根因（ASCII 表）入手，设计正确
- 鼠标抖动选区缩水是很 subtle 的 UX bug，修复方案（多级粒度）合理
- 大规模文件拆分使代码库更可维护
- 设置菜单功能有完整的设计→实现→迭代链条

### 关注点
- `9ef17033` 改动量大 (14 文件, +522/-205)，建议在真实使用中验证所有 12 个修复
- `a95322ed` commit 消息过于笼统，无法从 log 了解改了什么
- Search bar 重写 (+497) 无测试
- 54047afc 的 6677 行纯重构 diff 难以完整 review
- 264 行 `mouse.rs` 测试在模块末尾，可考虑抽出为独立测试文件

### 未提交变更
当前工作区有一个未暂存修改：`crates/shaping/src/lib.rs` (+7/-1)
