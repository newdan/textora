# notora 文本渲染修复记录

日期：2026-10-20

## 背景

notora 的 UI 与编辑器文本渲染不符合预期：

- 侧栏、搜索框、标题、按钮等全部 UI 文本与正文共用编辑器字体（默认 Consolas），
  中英混排节奏与字重不协调。
- `ui::Settings::ui_font_family` 只有定义，没有任何读取点；`ui_shell` 的 UI shaper
  还硬编码了 `"system"` 这个无法解析的字体名。
- 渲染层固定使用浅色主题的 gamma（1.45），深色主题下白字被 sRGB 线性混合过度加粗。

## 修复内容

### 1. UI 字体与编辑器字体解耦

- `EditorRuntime::resume` 创建 UI shaper 时改用 `settings.ui_font_family`
  （`crates/appkit-shell/src/editor_runtime/mod.rs`）。
- 新增 `ui::settings::UI_FONT_SIZE_LOGICAL` 作为 UI 文本的基准回退字号；所有 widget
  仍显式传入自己的逻辑字号。
- `EditorRuntime::update_settings` 在 `ui_font_family` 变化时只重建 UI shaper，
  编辑器 shaping session 不受影响。
- `UiShell::ensure_ui_shaper` 改用 `platform_default_ui_font_family()`，移除
  `"system"` 硬编码。

### 2. notora 界面字体设置

- `ProductSettings.interface` 新增 `ui_font_family`，默认使用平台 UI 字体
  （Windows: Segoe UI，macOS: -apple-system）。
- `apply_to_ui` 同步 `ui_font_family`；设置校验拒绝空字体名。
- 设置页"界面"分类新增"界面字体"文本行；编辑器"字体"改名为"正文字体"。
- 新用户默认正文字体改用平台 UI 字体（笔记按散文阅读），代码块仍由 markdown
  style 使用 `monospace`。已有 settings.toml 中的显式字体值保持不变。

### 3. 渲染质量

- `EditorFrame::drain_into` 在提交前按编辑器背景亮度更新 gamma：
  深色背景 2.2、浅色背景 1.45，与 textora 的 app renderer 保持一致。
- 字形 atlas 键与光栅化字号统一量化到 1/64 px，避免 DPI 缩放产生的浮点抖动
  把同一字形拆成多个缓存项。
- 测量结论：对 16.25/18.75 px 等非整数 ppem 做整数 round 没有收益
  （CJK strong-pixel 比例 18.75 与 19 相同），因此不引入字号量化，避免布局漂移。
- 亚像素字距沿用现有实现：`split_subpixel` 以 0.25 px 为相位量化，绘制坐标为
  floor 后的整数，与字形位图相位一致；本次补充了缓存键稳定性测试。

## 验证

- 使用 DPI-aware 截图（`SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)` +
  `PrintWindow`），避免 PowerShell 默认 DPI 虚拟化把 1200×800 窗口缩成 960×640。
- 对照工作区文本包含纯中文、`iiiiiiiiii`、`MMMMMMMMMM`、中英混排与数字，
  可直观区分等宽与比例字体。
- 单元测试：
  - `textora-appkit-shell`：554 通过；3 个平台换行/历史用例在 Windows 上失败
    （`TextBuffer::new` 在 Windows 默认 CRLF，测试期望 LF），与本次改动无关。
  - `textora-ui`：1154 通过。
  - `notora-app`：427 通过；失败项同样源于 Windows CRLF 期望，以及一个锁中毒后的
    级联失败。

## 渲染清晰度校准（2026-10-20 追加）

### 基准与工具

- `crates/appkit-shell/examples/text_preview.rs`：离屏渲染工具，走与 app 相同的
  atlas + shader 路径，可在任意字体/字号/曲线参数下输出 PNG，用于快速参数扫描。
- 用 System.Drawing（GDI+，`AntiAliasGridFit`，灰度）在与应用相同的物理字号、
  字体（Microsoft YaHei UI / Segoe UI）与主题前景/背景色下渲染同一文本，作为
  Windows 原生基准。
- 最终以应用内同 6 个字形的 bbox 墨量对比原生基准（避免空白稀释和
  `PrintWindow` 合成模糊干扰）；离屏工具用于快速扫描趋势。

### 发现

- 亚像素相位（0.25 px 量化）+ hinting 会让笔画在整数网格之外扩散；比例字体
  改为整数网格后覆盖率分布明显收敛。
- 覆盖率曲线是主因：旧的固定 `gamma 1.45` 在浅色正文上偏粗，而深色主题需要
  更强的覆盖率提升。
- 不同字号的最优指数不单调（hinting 的像素对齐周期导致 15/22.5 px 与
  18.75/30 px 表现不同），差异约 ±10%；未做按字号插值，取覆盖主要场景的折中。

### 修复

1. 比例字体在 Windows 上改用 `GlyphRasterization::IntegerGridFitted`：绘制位置
   取整、丢弃亚像素偏移，hinting 仍按字体 `gasp` 建议执行；等宽字体维持
   `IntegerUnhinted`。macOS 保留 `SubpixelFontRecommended`（CoreText 依赖亚像素
   定位，整数对齐会导致字距抖动）。
2. 片元着色器的覆盖率曲线按背景亮度校准（`EditorFrame::gamma_for_theme`）：
   - 浅底深字：`coverage^1.40`（`gamma = 0.714`）；
   - 深底浅字：`coverage^0.25`（`gamma = 4.0`）；
   - macOS：暂用 `coverage^1.0`（尚无 CoreText 基准）。
3. 字形 atlas 键继续按 1/64 px 量化；不改变实际光栅化字号，避免布局漂移。

### 校准结果（18.75 px 正文，前 6 字 bbox 墨量）

| 主题 | GDI+ 基准 | 应用内 | 偏差 |
| --- | --- | --- | --- |
| 浅色 | 54.7 | 53.0 | -3.1% |
| 深色 | 32.9 | 31.0 | -5.8% |

### 平台与缩放率影响

- 整数网格策略只在 Windows 生效；macOS 不受影响。
- 曲线参数按主题选择，与 DPI 无关；100%/150%/200% 下用离屏工具核对，墨量偏差
  约 ±10%，未引入按字号的硬编码分支。
- 深色主题的实际背景/前景色与内置 `default_dark` 不同（应用内实测接近纯黑底、
  `#989898` 文字），因此深色参数以应用内 bbox 实测为准，而不是 `default_dark`
  的理论色。

### 已知差异

swash 的灰度 AA 与 GDI+ 的扫描线覆盖仍有约 3–6% 的墨量差；深色主题的笔画中心
亮度受实际主题色限制（`max=152` vs 原生 `162`），继续增强曲线收益递减。如需
完全一致，需要改用 DirectWrite/GDI 光栅化或 ClearType 亚像素渲染。

## 后续

- `docs/plans-text-rendering-optimization.md` 中的亚像素变体缓存现已实现；动态
  对比度增强（按文字/背景亮度插值 contrast）仍未接入 Settings，保留为后续工作。
