# WYSIWYG 代码块光标几何修复设计

## 问题

Markdown WYSIWYG 代码块激活后，光标会落入字符内部。无语言标记的 fenced code block 同样复现，因此问题与语法高亮无关。

当前代码块文本在渲染时使用字体 shaping 得到真实 glyph advance，但代码行没有把 `ShapedRun` 保存到布局结果。`FlatLine` 缺少 shaping 几何后，caret 和 hit-test 会回退到按字号估算字符宽度。真实宽度与估算宽度的差异逐字符累积，最终使光标偏离字符边界。

## 方案

在 Markdown 代码块布局阶段，使用代码字体、代码字号和代码字重为每条已物化代码行生成真实 `ShapedRun`，并保存到 `LaidOutLine`。

现有 `LazyLayout::build_flat_lines` 已会把 `LaidOutLine.shaped` 复制到 `FlatLine`。因此 `grapheme_x`、`grapheme_at_x`、caret 定位和 hit-test 会自然共享同一组 glyph advance，无需增加代码块专用坐标补偿。

布局保持惰性：只 shaping 已进入物化范围的代码块，不预先处理整篇文档的全部代码行。渲染输出、激活规则、源码投影和 app/ui 分层均不改变。

## 测试

新增回归测试覆盖无语法高亮的 fenced code block：

1. 激活代码行后，目标 source byte 的 caret x 等于使用代码字体得到的真实 shaping 边界。
2. 在该 caret 屏幕位置执行 hit-test，返回相同的 source byte。

随后运行定向测试、`cargo fmt --all -- --check`、markdown crate 测试；若修改影响超过局部模块，再运行 `./scripts/verify.sh`。

## 非目标

- 不修改代码块的折叠或激活交互。
- 不调整语法高亮颜色或分段策略。
- 不用固定等宽单元或新的魔法比例补偿光标。
