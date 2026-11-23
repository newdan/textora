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
  （Windows: 微软雅黑，macOS: -apple-system）。
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
   - 浅底深字：`coverage^0.71`（`gamma = 1.3`，2026-10-21 多字体标定，见下文）；
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

## 字形失真修复（2026-10-20 追加）

### 复现

Windows 125% 缩放时正文为 15 × 1.25 = 18.75 px，界面为 14 × 1.25 = 17.5 px。
此前的 GridFit 实现把该小数直接交给 swash，而在 swash 0.1.19 中：

- `Scaling::size(ppem: f32)` 保留小数，字形轮廓按 18.75 缩放；
- 但 `skrifa::outline::HintingInstance::reconfigure` 用 `ppem as i32` **截断**为 18，
  TrueType 指令按 18 的网格对齐。

两者不一致时，密集汉字的平行笔画会被压到同一像素行。用离屏工具渲染"看"字（18.75 px）
可见"目"内两条横画合并成一条 2 px 粗线（只剩 4 条横画中的 3 条），"签、设、感"等复杂字
同样出现笔画粘连——这就是用户截图里箭头所指的"字形不对"。

### 修复

`GlyphRasterization::raster_em` 只对 `IntegerGridFitted` 取整到最近的整 ppem
（18.75 → 19，17.5 → 18），使 hinting 网格与轮廓缩放使用同一个 em；`IntegerUnhinted`
与 `SubpixelFontRecommended` 保持原字号，macOS 行为不变。

取整后的 18.75 px 渲染与 GDI+ 原生结果（`AntiAliasGridFit`，18.75 px）逐像素一致：
GDI+ 本身也把 18.75 px 当作 19 ppem 光栅化（18.75 与 19 原生渲染完全相同）。

### 验证

- 回归用例 `hinted_fractional_sizes_rasterize_on_the_whole_pixel_grid`：17.5 / 18.75 /
  16.75 px 的光栅化结果必须与取整后字号逐字节相同；修复前 18.75 与 19 的位图不同。
- 应用内截图（`textora-appkit-shell` 离屏工具与 notora 窗口）中，"看"字恢复 4 条横画，
  与 GDI+ 基准结构一致。

### 200 字随机样本对照（2026-10-21）

- 样本：从 270 个常用字中按固定种子抽取 200 字（`.screenshots/random200.txt`）。
- 双侧同网格渲染：应用走完整 shaping → hinting → atlas → shader 路径
  （`text_preview ... "sheet:50:26"`，新增 `@文件` 与格板模式），GDI+ 用
  `System.Drawing` + `AntiAliasGridFit` + Microsoft YaHei UI 18.75 px，前景/背景同色，
  每个字占 26 px 方格。
- 结果（18.75 px）：应用墨迹与 GDI+ 在 1 px 容差内**双向重合 100%**（最小 99.3%），
  逐字 NCC 均值 0.95；200 字无缺笔、粘连、错字或豆腐块。界面字号 17.5 px 下为 99.43%
  （最小 83.6%），差异来自 GDI+ 在该尺寸的 0.5 px 相位。
- 已知差异：应用保留 18.75 px 的小数 advance（DirectWrite 行为），GDI+ 将 advance
  取整为 19 px，整行宽度差约 1.3%；浅色覆盖率曲线已于 2026-10-21 重新标定为
  `coverage^0.71`（墨量比 0.99），见下一节。

## 多字体全面对照（2026-10-21）

### 语料与方法

- 语料 233 字（`.screenshots/corpus.txt`）：简单笔画、方形/全包围、圆弧/曲笔、
  复杂多笔、中文标点、大写、小写、数字与易混符号（1Il0O|`~!@#$%）、中英混排。
- 字体族 10 个：Microsoft YaHei UI / Microsoft YaHei / SimSun / SimHei / DengXian /
  Microsoft JhengHei / Noto Sans SC / Consolas / Segoe UI / Arial，各测 18.75 px 与
  17.5 px。应用侧走 `text_preview ... "sheet:50:32"`（每字 32 px 隔离格、按 advance
  居中），GDI+ 用 `System.Drawing` + `AntiAliasGridFit` + 同名（本地化名）字体。

### 结果（18.75 px，1 px 容差墨迹重合 / 灰度 NCC / 墨量比）

| 字体 | 重合 | NCC | 墨量 |
| --- | --- | --- | --- |
| Microsoft YaHei UI | 100.00% | 0.967 | 0.99 |
| SimSun | 99.95% | 0.992 | 0.93 |
| SimHei | 100.00% | 0.924 | 0.91 |
| DengXian | 100.00% | 0.995 | 0.93 |
| Microsoft JhengHei | 100.00% | 0.992 | 0.95 |
| Segoe UI | 99.80% | 0.880 | 0.98 |
| Noto Sans SC | 98.59% | 0.784 | 0.93 |
| Consolas / Arial（中文回退格） | 98.3% | 0.80 | CJK 1.48 |

### 覆盖率曲线重新标定

浅色主题原为 `coverage^1.40`（gamma 0.714），在 233 字、多字体语料上比 GDI+ 薄约 17%。
改为 `coverage^0.71`（`LIGHT_BACKGROUND_GAMMA = 1.3`）后：CJK 墨量比 0.83 → 0.99，
逐像素 MAE 30.8 → 21.1，NCC 0.947 → 0.969；拉丁字形墨量 0.87–1.03。macOS 与暗色主题
的曲线不变。

### 遗留差异（非缺笔/错字）

- GDI+ 对拉丁字体里的中文走 font linking（宋体），应用走脚本回退（微软雅黑 UI），
  与 DirectWrite 一致；这是有意的选择，不跟随 GDI+ 的宋体回退。
- 17.5 px（Windows 125% 的界面字号）在 GDI+ 中是 0.5 px 相位，整数栅格无法完全重合，
  1 px 容差重合 99.8%，NCC 0.81。
- Noto Sans SC 是可变字体，GDI+ 与 swash 的实例化有 1 px 级差异。
- 应用保留小数 advance（18.75），GDI+ 取整为 19，整行宽度差约 1.3%。

### 清晰度（边缘对比）标定

- 指标（18.75 px、233 字、雅黑 UI）：每字实心像素（`<120`）、过渡像素（`120–235`）、
  Laplacian 方差、与 GDI+ 的像素 MAE。
- 标定前（仅 gamma 1.3）：实心 45.8、过渡 58.2、Laplacian 7490、MAE 22.06；
  GDI+ 为 47.7 / 43.6 / 9915。
- 原因：swash 的覆盖率掩码比 GDI 的扫描转换铺开更多过渡像素；单纯提高 gamma 只会整体
  加粗，实心像素数不升反降、MAE 变差（实测 gamma 1.8/2.0 更差）。
- 修复：片元着色器在 gamma 之后加一条 S 曲线
  （`GLYPH_SHARPNESS = 0.2`，仅 Windows 生效，`platform_glyph_sharpness`），
  保持 50% 交点不动，把过渡像素推向 0/1。
- 标定后：实心 52.8、过渡 46.0、Laplacian 8597、MAE 20.94、NCC 0.968。过渡像素数与
  GDI+ 基本一致，Laplacian 差距由 −24% 收窄到 −13%。
- 残余差距来自 swash 与 GDI 光栅化器本身的抗锯齿形状差异；继续加大锐度会让实心像素
  超过 GDI+ 且 MAE 回升（0.4 时 MAE 28.0）。
- 暗色主题沿用同一 S 曲线（尚未用反色基准标定）；macOS 不启用。

### 界面字体设置不生效（2026-10-21）

- 现象：修改"界面字体"后侧栏、设置页文字毫无变化，始终是系统回退（微软雅黑）。
- 根因：widget 绘制文本时传 `font_family: None`，而 `UiTextLayout::new` 和
  `MeasureFromShaper::measure_with_font` 会无条件调用 `Shaper::set_font_family(None)`，
  把 UI shaper 已配置的字体族重置为 `Family::SansSerif`；字体库里的 `sans-serif`
  默认族是 "Fira Sans"（Windows 上不存在），最终落到脚本回退的微软雅黑。于是
  `EditorRuntime` 用 `settings.ui_font_family` 建好的 shaper 被每个控件覆盖掉。
- 修复：`None` 改为"继承 shaper 当前字体族"，只有显式给出 family 才覆盖；
  测量与绘制两条路径保持一致。
- 验证：界面字体设为 KaiTi 后，侧栏与设置页整体变成楷体（修改前无变化），
  默认界面字体的观感不变。`cargo test -p textora-ui` 通过。

### 默认界面字体改为微软雅黑（2026-10-21）

- `platform_default_ui_font_family` 在 Windows 上由 `Segoe UI` 改为 `微软雅黑`
  （本地化族名与 `Microsoft YaHei` 解析到同一字体）。Segoe UI 没有中文字形，
  之前中文一律走脚本回退，中英混排时拉丁与汉字分属两种字体；统一到微软雅黑后
  整个界面（含拉丁字符）使用同一族。
- 已安装用户的 `settings.toml` 中显式写有旧值，需要手动改成 `微软雅黑`（或删除该行）。
  macOS 仍为 `-apple-system`，Linux 仍为 `system-ui`。

## 后续

- `docs/plans-text-rendering-optimization.md` 中的亚像素变体缓存现已实现；动态
  对比度增强（按文字/背景亮度插值 contrast）仍未接入 Settings，保留为后续工作。
