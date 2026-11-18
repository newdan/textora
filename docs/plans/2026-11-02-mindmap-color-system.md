# 思维导图配色逻辑 P0 实施计划

> **执行方式：** 本计划对应候选方案，尚未接线；获得确认后按子任务逐项实施。P1 只保留候选，不在 P0 顺手改 16 套方案的色相。

日期：2026-11-02  
状态：待评审  
规格：`designs/notora-interface-theme/project/mindmap-colors.md`  
目标形态：`designs/notora-interface-theme/project/components/MindmapNode/preview.html`

**目标：** 把“一支分支色同时画线、边框、标题、选中”拆成线/墨/纸三个角色；让状态卡面随方案派生；让选中不再借分支色；守住深色分支标题与浅色状态标题的 4.5:1。

**架构：** `ui::theme::mindmap` 仍只提供纯数据；分支方案从 `Vec<[f32;4]>` 升为带 `line` / `ink` 的结构，卡面透明度随方案保存。`markdown::mmf::canvas` 只消费角色，不再在渲染层用魔法 alpha 混合角色；`ui::widgets::mindmap_style_panel` 用同一组角色画预览。文件级 `theme` 字段与 16 个稳定 ID 不变。

**技术栈：** Rust、wgpu、现有 `textora-ui` / `textora-markdown` 主题与画布测试。

## 全局约束与已确认决策

- P0 不改 16 套方案的六支线色，只派生 ink 与卡面；改色相属于 P1。
- 分支标题目标 5.0:1，状态/根标题 ≥ 4.5:1；线、选中环、优先级边框 ≥ 3:1。按未舍入值比较。
- 深色分支 ink 必须朝白派生，不能继续沿用“线色即标题色”；浅色朝黑派生。
- 状态墨按浅/深模式固定；状态卡面 = 墨 8% 叠该方案画布，不再复制 `default_light/default_dark` 的固定 fill。
- 选中使用 `focus-ring` 2px 外环、离卡 2px + accent 8% 水洗；hover 用 +4pp 水洗与 2px 边框；分支底色仍是 line 14%。
- 根节点改为“墨块”，不复用第 1 支分支色；P0 可以先保留现有 root，P1 再统一，避免一次改两层视觉。
- 每个子任务最多修改三个文件；跨层字段接完后统一编译，任何提交前必须编译通过。
- 不改全局 gamma、不改布局、不新增文件级主题 ID、不做用户自定义配色。

## 子任务 1：主题数据与派生（2 文件）

文件：

- `crates/ui/src/theme/mindmap.rs`：新增分支角色结构、ink 派生、surface 透明度、状态卡面派生。
- `crates/ui/src/theme_file.rs`：TOML `branch_palette` 仍按 hex 列表解析，转换成新结构。

- [ ] 先补失败测试：深色“夜航”分支标题在 18% 水洗上低于 4.5；浅色“doing”在状态卡面上低于 4.5；深色状态卡 fill 与方案卡面不等。
- [ ] 定义 `MindmapBranchStyle { line, ink }`（surface 用 line + alpha 表达，不存合成色）；`MindmapCanvasTheme` 保存 `surface_alpha`（0.14）。
- [ ] `ink` 在线性 RGB 里朝黑/白插值到 5.0:1；派生结果参与 `gamma_correct`，alpha 不参与。
- [ ] 状态样式由“墨 + 8% 画布”构造，五种状态统一走同一函数；priority 仍只有 border/accent。
- [ ] TOML 覆盖仍接受 `["#RRGGBB", …]`，解析后派生 ink/surface；旧主题文件无需迁移。
- [ ] 执行 `cargo test -p textora-ui theme:: -- --test-threads=1` 与 `cargo test -p textora-ui theme_file -- --test-threads=1`。

## 子任务 2：画布消费角色（1 文件）

文件：`crates/markdown/src/mmf/canvas.rs`。

- [ ] 先补/调整绘制断言：分支卡 fill 是 line 的水洗、text 是 ink、border 是 line；选中环颜色等于 `focus_ring` 而不是分支色；hover 只增加水洗与边框宽度。
- [ ] `get_node_style`：分支层写 fill/border/text/accent 四个角色；status 整卡覆盖；priority 只改 border/accent；named 最后覆盖。
- [ ] 删除渲染层的 `BRANCH_TINT_FILL_ALPHA`、`SELECTION_FILL_ALPHA` 魔法值，改用方案/常量语义命名。
- [ ] 选中：外环 `focus_ring`、水洗 accent 8%；hover：`surface_alpha + 0.04`、边框 2px；连接线继续用 branch line。
- [ ] 文本选区、拖拽无效、控制点继续用现有 `selection` / `drag_invalid` / line，不另造颜色。
- [ ] 执行 `cargo test -p textora-markdown mmf:: -- --test-threads=1`。

## 子任务 3：风格面板预览（1 文件）

文件：`crates/ui/src/widgets/mindmap_style_panel.rs`。

- [ ] `MindmapStyleOption` 带入分支 `line` / `ink` / `surface_alpha`；缩略图按“水洗卡面 + 线边 + 墨字/墨块”绘制，不再实心填充分支色。
- [ ] 摘要色板保留六支 line 的色条；选中卡的外框仍用界面 `accent_text`，与画布选中语义分开。
- [ ] 先补缩略图颜色断言（或最小绘制快照），确保“看到的就是打开后的”。
- [ ] 执行 `cargo test -p textora-ui mindmap_style_panel -- --test-threads=1`。

## 子任务 4：跨方案对比度验收（1–2 文件）

- [ ] 新增遍历测试：16 套方案 × 浅/深 × 六支分支 × 五种状态 × 四级优先级。
- [ ] 断言：分支/状态/根标题 ≥ 4.5；线、选中环、优先级边框 ≥ 3；第 7 支与第 1 支不完全相同。
- [ ] 运行时口径：先合成 alpha，再在线性空间算亮度；源 sRGB 审计另记，不混用。
- [ ] 执行 `cargo fmt --all -- --check`、`cargo check --workspace`、`./scripts/verify.sh`。未实际通过不得写成完成。

## P1（本轮不落地）

- 落地“纸墨 / 墨夜”默认对与 16 套方案重算；根节点统一为墨块。
- 优先级加文字标记（P0/P1…），不再只靠颜色。
- 第 7 支周期变体；风格面板摘要与画布同步。
- 若采用新默认对，先决定 `paper` 的兼容映射，不破坏旧文件 `theme = "paper"`。

## P2（另立需求）

- 统一 `Theme.mindmap`（TOML）与内置 scheme 的颜色所有权；现在 TOML 颜色覆盖在视图里不生效。
- 决定思维导图是否跟随应用浅/深模式自动配对；当前规格是文件固定视觉。
- 把思维导图色导出为独立 token 家族，仍不并入应用浅/深 token。

## 验收与回滚

- 验收以设计系统规格的门槛为准，不以“看起来更清楚”代替数值。
- P0 单独提交，回滚只需还原 1–3 个子任务；不混入 P1 的色相调整。
- 提交说明写清：改的是角色派生，不是换配色方案。
