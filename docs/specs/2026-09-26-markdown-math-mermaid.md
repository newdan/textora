# Markdown 公式与 Mermaid 原生渲染

## 目标与边界

在 textora 原生 Markdown 视图中渲染 `$…$` 行内数学、`$$…$$` 展示数学以及 `mermaid` 围栏。保留源码、编辑、选择和撤销语义；错误或不支持的表达式退回可读源码。这里的 LaTeX 指数学表达式，不是完整 TeX 文档、任意宏包或 TikZ。

## 选型依据

- RaTeX：`ratex-parser`、`ratex-layout`、`ratex-svg`，优先同版本 0.1.14；SVG 开启 standalone/embed-fonts，并嵌入字形路径，避免依赖网页 CSS 或用户安装 KaTeX 字体。
- Mermaid：`mermaid-rs-renderer` 0.3.1，关闭 CLI/PNG 默认功能，直接取得 SVG。
- resvg：将上述 SVG 转成预乘 RGBA；只接受引擎生成的内存 SVG，不加载外部资源。
- 这些 Rust 实现仍较新，不能宣称与官方 KaTeX/Mermaid 完全兼容。采用版本锁定、引擎隔离和实际测试约束风险。若注册表可用版本/API 与文档不同，实施者先报告事实再调整。
- 备选 MiTeX+Typst 需要完整排版依赖；官方 JS 引擎需要 JS/浏览器运行时，均不作为本次默认路径。

来源：https://docs.rs/ratex-svg/latest/ratex_svg/ 、https://docs.rs/ratex-layout/latest/ratex_layout/ 、https://docs.rs/mermaid-rs-renderer/latest/mermaid_rs_renderer/ 。

## 架构

1. `textora-markdown` 内封装公式和图表引擎，输出尺寸、数学基线、不可变 RGBA 图像及结构化错误。引擎不依赖 app 状态。
2. `ui::core::paint` 定义纯数据 `RasterImage` 与图像绘制命令；appkit-shell 负责上传与绘制，维持 DrawList 顺序和嵌套裁剪。禁止 ui 依赖 app、DocumentView 或 Workspace。
3. parser 保留 math 事件及字节范围，builder 保留相应语义；Mermaid 只接管语言标识为 mermaid 的围栏。
4. layout 使用真实图像宽高，行内公式参与换行、基线与行高；展示公式和图表作为块。复用现有 source projection，不通过伪造空格或仅覆盖文字制造显示效果。
5. WYSIWYG 光标或选区进入对应公式/图表时展开源码，离开后恢复预览。错误保留完整源码，不能吞掉内容。
6. 缓存按源码、类型、颜色/主题和字号区分；保持有界，重复绘制不重复解析引擎。尺寸变化只重算必要几何。GPU 图像缓存需保持资源有效期，不破坏帧缓存。

数学 SVG 按 72 DPI 解释其 `pt` 根尺寸，使请求字号、像素尺寸和基线保持一致；Mermaid 保持 SVG 默认 96 DPI。GPU 使用独立 4096×4096 RGBA 图集，不占用 R8 字形图集。超过引擎尺寸预算时直接显示源码；同帧 GPU 图集满载时显示可读错误提示，原有点击展开源码的交互保留。数学字形内嵌；Mermaid 中文沿用系统 CJK 字体。

## 验收

- 常见分式、根式、上下标、矩阵可见；公式两侧中文/英文不重叠，行高合理。
- Mermaid 至少验证流程图、时序图及中文标签；图表适配正文宽度。
- 行内代码和其他代码围栏中的数学符号保持字面量。
- 无效公式/图表、编辑中的未完成语法均可继续阅读和编辑。
- 滚动裁剪、主题/字号更新、源码范围和点击定位具有回归测试。
- 无浏览器、Node 或系统 TeX 安装要求；数学字形随依赖嵌入。
- 运行相关 crate 测试、编译，并执行 `./scripts/verify.sh`。如基线已有失败，记录原始证据，不能将未通过验证描述为通过。
