# Golden Image Tests

本目录存放渲染 golden image，用于 SSIM 比对验证。

## 目录结构

```
tests/golden/
├── README.md                       # 本文件
├── hello_edit_plus.png             # 阶段 4：静态文本渲染 golden
├── hello_edit_plus_2x.png          # 阶段 4：Retina 2× golden（可选）
└── ...
```

## 生成方式

Golden image 由 `render_smoke` 测试在 headless wgpu 模式下生成：

```bash
# 生成 golden（首次 / 基线更新）
RENDER_GOLDEN_UPDATE=1 cargo test -p edit-plus-app render_hello_to_png

# 验证 golden（CI / 回归检查）
cargo test -p edit-plus-app render_hello_to_png
```

生成脚本会：
1. 用 headless wgpu 创建 800×600 纹理
2. 渲染 "Hello, edit+ — 世界 👨‍👩‍👧" 到纹理
3. 读回像素 → 编码为 PNG
4. 与本目录下的 golden 文件做 SSIM 比对

## SSIM 阈值

| 场景 | 阈值 |
|---|---|
| 同平台同 GPU | ≥ 0.99 |
| 跨平台（macOS Intel vs Apple Silicon） | ≥ 0.95 |

## 更新 Golden

当以下任一条件变化时，需要重新生成 golden：
- 字体版本更新
- 渲染 shader 修改
- atlas 布局算法变更
- cosmic-text 版本升级

更新步骤：
1. `RENDER_GOLDEN_UPDATE=1 cargo test -p edit-plus-app render_hello_to_png`
2. `git diff tests/golden/` 确认差异合理
3. 提交新的 PNG

## 当前状态

- [ ] `hello_edit_plus.png` — 阶段 4 完成后生成
