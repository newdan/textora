# 字体渲染优化方案：亚像素排版偏移与动态对比度增强

本方案旨在进一步提升 `edit+` 在高分屏下的文字渲染质量，使其文字排版的平滑度（Kerning）和笔画粗细的自适应能力达到业界顶尖水平（如 Zed）。

## 1. 亚像素排版偏移缓存 (Subpixel Variant Caching)

### 痛点分析
当前 `edit+` 的文字渲染为了避免亚像素级别的采样模糊，在生成 Quad 顶点坐标时采用了强硬的整数吸附（`.round()`）。虽然保证了渲染边界的清晰锐利，但**吞噬了由于浮点数累加产生的小数点级别字距微调（Kerning/Tracking）**。长文本的字母间距可能会因此产生不均匀感。

### 解决方案
采用类似 Zed 的策略：**让底层字体栅格引擎生成带有微小偏移量的字形灰度图，并分类缓存。渲染管线在屏幕上严格进行整数对齐绘制，但由于所使用的字形本身就是带偏移版本的，从而实现了物理像素上极其平滑的字符间距表现。**

### 参考 Zed 源码实现
*   **文件路径**：`zed/crates/gpui/src/window.rs` (约 `3780` 行)
*   **逻辑说明**：
    Zed 定义了 `SUBPIXEL_VARIANTS_X` 和 `SUBPIXEL_VARIANTS_Y`（通常将一个像素分为 4 份）。
    在准备绘制 Glyph 时，首先计算量化后的原点坐标，然后截取小数部分生成一个 `Point<u8>`：
    ```rust
    let quantized_origin = Point::new(
        round_half_toward_zero(glyph_origin.x.0 * SUBPIXEL_VARIANTS_X as f32) / SUBPIXEL_VARIANTS_X as f32,
        // ...
    );
    let subpixel_variant = Point::new(
        (quantized_origin.x.fract() * SUBPIXEL_VARIANTS_X as f32) as u8,
        (quantized_origin.y.fract() * SUBPIXEL_VARIANTS_Y as f32) as u8,
    );
    ```
    随后将 `subpixel_variant` 存入 `RenderGlyphParams` 作为图集（Atlas）缓存 Key 的一部分。底层 CoreText 光栅化时，会利用这个变体参数生成含有精确小数位移的位图。

### `edit+` 实施计划
1.  在 `text_rasterize.rs` 的光栅化请求参数中引入 `subpixel_phase: (u8, u8)`。
2.  在 `shaping` 模块或底层的 Swash Rasterizer 接口中启用亚像素支持。
3.  在计算 `px` 和 `py` 顶点边界时，将浮点坐标拆分为 **整数坐标** 和 **小数相位**。
4.  根据小数相位（如取模 4 等份）从图集中获取相对应的变体，并用截断后的整数坐标绘制 Quad。

---

## 2. 动态对比度增强与 Gamma 自适应 (Dynamic Stem Darkening)

### 痛点分析
目前 `edit+` 对于文字的加粗/边缘增强效果（Stem Darkening）是硬编码在 Shader 中的。这意味着无论用户在浅色还是深色主题下，无论屏幕自身的色彩描述文件（Color Profile）如何，渲染的粗细增量都是恒定的，容易导致黑底白字过粗，或白底黑字发虚。

### 解决方案
根据文字的前景色与背景的对比亮度，在 GPU Fragment Shader 中实时、线性地计算并调整膨胀系数（Enhanced Contrast / Dilation），同时从全局或环境变量动态获取准确的 Gamma 转换比率。

### 参考 Zed 源码实现
*   **WGSL Shader 路径**：`zed/crates/gpui_wgpu/src/shaders.wgsl`
    ```wgsl
    fn light_on_dark_contrast(enhancedContrast: f32, color: vec3<f32>) -> f32 {
        let brightness = color_brightness(color);
        // 如果是白字（亮度高），增强系数趋近于 0；如果是黑字（亮度低），系数达到最大
        let multiplier = saturate(4.0 * (0.75 - brightness));
        return enhancedContrast * multiplier;
    }
    ```
*   **Rust 宿主参数配置路径**：`zed/crates/gpui_wgpu/src/wgpu_renderer.rs`
    通过读取环境变量 `ZED_FONTS_GRAYSCALE_ENHANCED_CONTRAST` 和 `ZED_FONTS_GAMMA`，初始化并构建 `GammaParams` Uniform Buffer，并传递给 GPU 混合器：
    ```rust
    let grayscale_enhanced_contrast = env::var("ZED_FONTS_GRAYSCALE_ENHANCED_CONTRAST")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0_f32)
        .max(0.0);
    ```

### `edit+` 实施计划
1.  修改 `crates/render/src/lib.rs` 中的 WGSL 逻辑，引入类似于 `light_on_dark_contrast` 的亮度插值算法，取代目前死板的 `mix` 硬编码。
2.  在 `GlyphRenderer` 或 `RenderState` 中引入并绑定一个新的 Uniform Buffer，负责向 Shader 传递配置项。
3.  在 `ui::Settings` 中暴露 Gamma 值和 Contrast 加成参数，支持用户通过配置文件修改。并在渲染循环前提取当前的主题前景色，传递给 Shader 动态决定最佳的膨胀权重。
