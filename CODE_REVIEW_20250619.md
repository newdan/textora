> Status: superseded
> Date: 2026-06-19
> Superseded by: `docs/plans-code-quality-remediation-overview.md`
> Note: filename contains the legacy 2025-style date and is retained for link compatibility.

# 代码审查 — 最近 12 小时提交 (2026-06-19)

共 **12** 个 commit，时间范围 06-18 22:11 ~ 06-19 09:41。

---

## 概览

| # | Commit | 类型 | 文件数 | 风险 |
|---|--------|------|--------|------|
| 1 | `707428f` fix(preview): trigger redraw on selection drag | 修复 | 1 | 低 |
| 2 | `dc37c77` feat(ui): TOC面板优化 | 功能 | 4 | 中 |
| 3 | `a26e65a` fix(preview): TOC heading jump correction | 修复 | 1 | 中 |
| 4 | `0a7ad92` fix: 菜单背景改用sidebar_bg | 修复 | 1 | 低 |
| 5 | `d899a7d` fix: TOC hover效果和点击修复 | 修复 | 4 | 低 |
| 6 | `a81670b` fix: 菜单背景色调亮 | 修复 | 1 | 低 |
| 7 | `c08331e` fix: 菜单圆角改小(8→4) | 修复 | 1 | 低 |
| 8 | `9bf7ca2` fix: 菜单圆角回退8px | 修复 | 1 | 低 |
| 9 | `465ebfc` fix: sidebar按钮hover颜色 | 修复 | 1 | 低 |
| 10 | `fb0c8f4` feat: 深色主题暖黑配色 | 功能 | 6 | 中 |
| 11 | `7b683f4` fix(sidebar): 裁剪列表items防止溢出 | 修复 | 7 | 中 |
| 12 | `5f15989` fix: 补充 popup_menu 测试字段 | 修复 | 1 | 低 |

---

## 需要修改的项

### 1. [中] 空注释残留 — `sidebar/mod.rs` 无对应代码

**位置**: `crates/ui/src/widgets/sidebar/mod.rs` (commit `7b683f4`)

在 `on_event` 的 `MouseDown` 处理中新增了一行注释：
```rust
// 0.1) 滚动条优先：拦截 thumb 拖拽、翻页点击
```
但其后没有任何滚动条处理代码。注释描述的是 TOC 面板的行为（commit `dc37c77` 中实现），sidebar 当前没有 scrollbar widget。

**建议**: 删除该注释，或补全对应的 sidebar 滚动条逻辑。注释悬空容易误导后续维护者。

---

### 2. [低] 冗余 MouseUp 分支 — `sidebar/mod.rs`

**位置**: `crates/ui/src/widgets/sidebar/mod.rs` (commit `7b683f4`)

```rust
Event::MouseUp { px, py, button } if *button == MouseButton::Left => {
    None
}
Event::MouseUp { .. } => None,
```

两个分支都返回 `None`，第一个分支毫无必要。引入原因看起来是调试/测试残留。

**建议**: 删除第一个分支（Left button guard），只保留通用分支。

---

### 3. [低] 圆角参数反复变动 — `popup_menu/types.rs`

**位置**: `crates/ui/src/widgets/popup_menu/types.rs`

- commit `c08331e`: `radius: 8→4`, `outer_radius: radius→radius+border`
- commit `9bf7ca2`: `radius: 4→8`, `border: 1.5→1.0`, `outer_radius: radius+border→radius`

最终状态 `radius=8, border=1.0` 的视觉效果与最初只有 border 从 1.5 改为 1.0 的差异。commit `c08331e` 中 `radius+border` 修复（边框圆角均匀宽度）在回退时丢失了。

**建议**: 考虑 `fill_rounded(outer, border_color, radius + border)` 保持边框视觉均匀。当前 outer 和 inner 同半径，border 在四角会略粗。

---

### 4. [低] Theme 字段补全分散在多个提交中

以下 commit 都在补测试 Theme 构造器的缺失字段：
- `0a7ad92` 加了 `border_subtle`（非 struct 字段，局部变量）
- `d899a7d` 加了 `toc_hover_text_color`
- `fb0c8f4` 加了 `sidebar_item_active_fg`, `sidebar_item_hover_bg`
- `7b683f4` 又加了一次 `sidebar_item_active_fg`, `sidebar_item_hover_bg`
- `5f15989` 加了 `toc_hover_text_color`

每个新增字段应该在其引入的 commit 中一并更新所有测试构造器。`toc_hover_text_color` 在 `d899a7d` 引入，但测试补丁分别出现在 `fb0c8f4`、`7b683f4`（dock/widget 测试）和 `5f15989`（popup_menu 测试）。

**建议**: 后续新增 Theme 字段时，确保同一 commit 内更新所有 `Theme { .. }` 构造点（共约 6 处：`theme.rs::test_theme()`、`dock.rs tests`、`widget.rs tests`、`popup_menu/mod.rs tests`、`editor_host.rs tests`、`sidebar/widget_tests.rs`）。建议考虑引入 `Theme::test_default()` 辅助函数统一构造，避免每个测试文件手写结构体字面量。

---

### 5. [低] `toc_on_scroll` 中 downcast 查找脆弱

**位置**: `crates/app/src/ui_shell.rs` (commit `dc37c77`)

```rust
for child in self.dock.children.iter_mut() {
    if let Some(toc) = child.widget.as_any_mut().downcast_mut::<TocWidget>() {
        toc.set_scroll_y(self.toc_scroll_y);
        break;
    }
}
```

通过遍历 dock children + downcast 来传递滚动状态，耦合了 widget 具体类型。

**建议**: 如果 dock 后续重构，考虑让 TocWidget 从 `UiShell` 的 `toc_scroll_y()` 方法读取状态（在 `build_dock` 时已传入）。目前可用，但值得加注释说明 fallback 路径的意图。

---

### 6. [中] `pending_heading_jump` 状态与手动滚动的竞态

**位置**: `crates/app/src/md_preview.rs` (commit `a26e65a`)

逻辑链：
1. `scroll_to_heading()` 设置 `pending_heading_jump = Some(index)`
2. 下一帧 precision pass 后，用修正后的 heading 位置更新 `scroll_y`
3. 如果用户在此期间手动滚动了，`apply_scroll_delta()` 清除 `pending_heading_jump = None`

问题：如果 precision pass 触发的帧恰好也是用户滚动触发的同一帧，执行顺序取决于 `apply_scroll_delta` 和 render 的调用先后。当前代码在 precision pass 中 `take()` 了 pending，不会重复执行。但如果 `apply_scroll_delta` 先清除 pending，precision pass 就不会修正位置。

**建议**: 在 `scroll_to_heading` 调用处确认先 `apply_scroll_delta` 再 render，或使用 frame counter 而非 Option 来追踪是否还在 pending 窗口内。当前实现在常规使用中应该正确，因为鼠标滚轮事件和 TOC 点击不会同时发生，但加一个注释说明假设会更好。

---

### 7. [信息] 无视觉回归测试

所有 color/token 值调整（commit `fb0c8f4`, `0a7ad92`, `a81670b`, `dc37c77` 等）均无自动化截图对比或色值断言。

**建议**: 不强制本次修改，但考虑为关键 UI 场景（sidebar、menu、TOC 面板）添加基础的 theme color 快照测试。

---

## 专项评估

### Code Correctness

- **良好**: `ListWidget::item_rect` 将 `scroll_offset` 内置到坐标计算中，`hit_row`/`hit_close_btn` 同步移除了外部传入的 `scroll_offset` 参数。所有调用点和测试均一致更新，未见遗漏。
- **良好**: `pending_heading_jump` 的修正逻辑在 precision pass 中用 `take()` 消费，保证单次执行。
- **问题 #6 已覆盖**: 同帧内手动滚动与 heading jump 的执行顺序依赖。

### 项目约定

- **主题色系统**: `sidebar_item_active_fg`、`sidebar_item_hover_bg`、`toc_hover_text_color` 等新字段遵循现有命名惯例（`{component}_{variant}_{property}`），类型统一为 `[f32; 4]`，访问器模式一致。✅
- **测试 Theme 构造**: 多个测试文件中 Theme 使用结构体字面量手写（~40 字段），而非通过 `test_theme()` 辅助函数。字段新增时容易遗漏（已发生 3 次补丁 commit）。**建议**统一使用 `test_theme()` 或 Builder 模式。
- **Widget trait 实现**: `PushClip`/`PopClip` 配对使用正确，遵循了 `paint()` 中成对 push/pop 的惯例。✅

### 性能

- **`ListWidget` 可见行过滤** (commit `7b683f4`): 从绘制全部 items 改为只绘制 `first_visible..last_visible` 范围内的行。O(n)→O(visible) 的绘制复杂度改善，在 tabs 数量大时（如 100+ 文件）有明显收益。✅
- **`MouseMove` 触发 redraw** (commit `d899a7d`): 每次鼠标移动都 `RequestRedraw`。对于 TOC hover 效果这是必要的，但会以显示器刷新率持续重绘。当前实现合理，若后续发现 CPU 占用高可考虑 dirty-region 优化。无阻塞问题。
- **`toc_on_scroll` downcast 遍历** (commit `dc37c77`): 每次滚轮事件遍历 dock children 做 downcast。dock children 数量通常 < 10，开销可忽略。✅

### 测试覆盖

| 维度 | 状态 | 说明 |
|------|------|------|
| 单元测试 | ✅ | `list.rs` 测试期望值同步更新（cmd count、可见行数），`widget_tests.rs` 新增 `scroll_moves_items_and_hit_follows` |
| 边界测试 | ✅ | 空列表、overflow 裁剪、separator/header 不可点击、close button hit_pad clip 均有覆盖 |
| 集成测试 | ⚠️ | 无端到端测试验证完整滚动+渲染链路 |
| 视觉回归 | ❌ | 大量 color 值调整无自动化快照对比 |

**建议**: 至少为 `claude_dark()` / `claude_light()` 的完整 Theme 输出添加一个 equality snapshot test，防止后续 color token 意外漂移。

### 安全性

本批提交均为客户端 GUI 渲染代码（Rust、无网络、无用户数据持久化），**未发现安全问题**。无 SQL 拼接、命令注入、XSS、或敏感数据处理。

---

## 整体评价

- **正面**: 12 个 commit 粒度细，每个 commit 单一目的，revert/调整类 commit（c08331e→9bf7ca2）如实记录了迭代过程。
- **正面**: 大部分代码改动有对应测试更新（`list.rs` 测试期望值同步修正、`widget_tests.rs` 新增滚动测试）。
- **正面**: `ListWidget` 可见行过滤带来实际性能改善；PushClip/PopClip 替代 sidebar 层的手动 clip，职责更清晰。
- **改进空间**: 少量调试残留（空注释、冗余 match 分支），Theme 字段补丁可更及时地随字段引入一起更新。
- **改进空间**: 缺少视觉回归测试和 Theme snapshot test。
