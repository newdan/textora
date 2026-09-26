//! Self-contained math and Mermaid rasterization for Markdown layout.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use ratex_types::color::Color;
use ratex_types::math_style::MathStyle;
use ui::core::paint::RasterImage;

const MAX_SOURCE_BYTES: usize = 16 * 1024;
const MAX_SVG_BYTES: usize = 8 * 1024 * 1024;
const GPU_ATLAS_SIDE: u32 = 4096;
const GPU_IMAGE_GUTTER: u32 = 1;
const MAX_IMAGE_SIDE: u32 = GPU_ATLAS_SIDE - 2 * GPU_IMAGE_GUTTER;
const MAX_IMAGE_PIXELS: u64 = (MAX_CACHE_BYTES / RGBA_CHANNELS) as u64;
const MIN_FONT_SIZE: f32 = 4.0;
const MAX_FONT_SIZE: f32 = 256.0;
const MATH_PADDING_RATIO: f64 = 0.15;
const MATH_SVG_DPI: f32 = 72.0;
const MERMAID_SVG_DPI: f32 = 96.0;
const MAX_CACHE_ENTRIES: usize = 64;
const MAX_CACHE_BYTES: usize = 32 * 1024 * 1024;
const RGBA_CHANNELS: usize = 4;
const LUMINANCE_THRESHOLD: u32 = 128_000;
const RED_LUMINANCE_WEIGHT: u32 = 299;
const GREEN_LUMINANCE_WEIGHT: u32 = 587;
const BLUE_LUMINANCE_WEIGHT: u32 = 114;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EmbeddedKind {
    InlineMath,
    DisplayMath,
    Mermaid,
}

#[derive(Clone, Copy, Debug)]
pub struct EmbeddedRequest<'a> {
    pub kind: EmbeddedKind,
    pub source: &'a str,
    pub font_size: f32,
    pub foreground: [u8; 4],
    pub background: [u8; 4],
}

#[derive(Clone, Debug)]
pub struct EmbeddedImage {
    pub image: Arc<RasterImage>,
    pub baseline: f32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EmbeddedError {
    InvalidRequest(String),
    InvalidSource(String),
    Render(String),
}

impl std::fmt::Display for EmbeddedError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRequest(message) => {
                write!(formatter, "invalid embedded request: {message}")
            }
            Self::InvalidSource(message) => write!(formatter, "invalid embedded source: {message}"),
            Self::Render(message) => write!(formatter, "embedded render failed: {message}"),
        }
    }
}

impl std::error::Error for EmbeddedError {}

#[derive(Clone, Eq, PartialEq)]
struct CacheKey {
    kind: EmbeddedKind,
    source: String,
    font_size_bits: u32,
    foreground: [u8; 4],
    background: [u8; 4],
}

type CachedRender = Result<EmbeddedImage, EmbeddedError>;
type ImageCache = VecDeque<(CacheKey, CachedRender)>;

static IMAGE_CACHE: OnceLock<Mutex<ImageCache>> = OnceLock::new();
static SYSTEM_FONTS: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();

pub fn render_embedded(request: EmbeddedRequest<'_>) -> Result<EmbeddedImage, EmbeddedError> {
    validate_request(&request)?;
    let key = CacheKey {
        kind: request.kind,
        source: request.source.to_owned(),
        font_size_bits: request.font_size.to_bits(),
        foreground: request.foreground,
        background: request.background,
    };
    let cache = IMAGE_CACHE.get_or_init(|| Mutex::new(VecDeque::new()));
    if let Ok(mut entries) = cache.lock()
        && let Some(rendered) = cache_hit(&mut entries, &key)
    {
        return rendered;
    }

    let rendered = render_uncached(&request);
    if let Ok(mut entries) = cache.lock() {
        if let Some(existing) = cache_hit(&mut entries, &key) {
            return existing;
        }
        entries.push_back((key, rendered.clone()));
        trim_cache(&mut entries);
    }
    rendered
}

fn cache_hit(entries: &mut ImageCache, key: &CacheKey) -> Option<CachedRender> {
    let index = entries.iter().position(|(candidate, _)| candidate == key)?;
    let entry = entries.remove(index).expect("matched cache index must exist");
    let rendered = entry.1.clone();
    entries.push_back(entry);
    Some(rendered)
}

fn validate_request(request: &EmbeddedRequest<'_>) -> Result<(), EmbeddedError> {
    if request.source.trim().is_empty() {
        return Err(EmbeddedError::InvalidSource("empty embedded source".into()));
    }
    if request.source.len() > MAX_SOURCE_BYTES {
        return Err(EmbeddedError::InvalidRequest("embedded source is too long".into()));
    }
    if !request.font_size.is_finite()
        || request.font_size < MIN_FONT_SIZE
        || request.font_size > MAX_FONT_SIZE
    {
        return Err(EmbeddedError::InvalidRequest("font size is out of range".into()));
    }
    Ok(())
}

fn render_uncached(request: &EmbeddedRequest<'_>) -> CachedRender {
    match request.kind {
        EmbeddedKind::InlineMath | EmbeddedKind::DisplayMath => render_math(request),
        EmbeddedKind::Mermaid => render_mermaid(request),
    }
}

fn render_math(request: &EmbeddedRequest<'_>) -> CachedRender {
    let nodes = ratex_parser::parser::Parser::new(request.source)
        .parse()
        .map_err(|error| EmbeddedError::InvalidSource(error.to_string()))?;
    let color = Color::new(
        request.foreground[0] as f32 / 255.0,
        request.foreground[1] as f32 / 255.0,
        request.foreground[2] as f32 / 255.0,
        request.foreground[3] as f32 / 255.0,
    );
    let mut layout_options = ratex_layout::LayoutOptions::default().with_color(color);
    if request.kind == EmbeddedKind::InlineMath {
        layout_options = layout_options.with_style(MathStyle::Text);
    }
    let layout_box = ratex_layout::layout(&nodes, &layout_options);
    let font_size = request.font_size as f64;
    let padding = font_size * MATH_PADDING_RATIO;
    let svg_options =
        ratex_svg::SvgOptions { font_size, padding, embed_glyphs: true, ..Default::default() };
    let display_list = ratex_layout::to_display_list(&layout_box);
    let svg = ratex_svg::render_to_svg_with_color_syntax(
        &display_list,
        &svg_options,
        ratex_svg::SvgColorSyntax::Rgb,
    );
    let baseline = (layout_box.height * font_size + padding) as f32;
    rasterize_math_svg(&svg, request.background, baseline)
}

fn render_mermaid(request: &EmbeddedRequest<'_>) -> CachedRender {
    let options = mermaid_options(request);
    let svg = mermaid_rs_renderer::render_strict(request.source, options)
        .map_err(|error| EmbeddedError::InvalidSource(error.to_string()))?;
    let image = rasterize_svg(&svg, request.background, 0.0)?;
    Ok(EmbeddedImage { baseline: image.image.height() as f32, ..image })
}

fn mermaid_options(request: &EmbeddedRequest<'_>) -> mermaid_rs_renderer::RenderOptions {
    let mut options = mermaid_rs_renderer::RenderOptions::default();
    let foreground = format!(
        "#{:02x}{:02x}{:02x}",
        request.foreground[0], request.foreground[1], request.foreground[2]
    );
    options.theme.font_size = request.font_size;
    options.theme.text_color = foreground.clone();
    options.theme.primary_text_color = foreground.clone();
    options.theme.pie_title_text_color = foreground.clone();
    options.theme.pie_section_text_color = foreground.clone();
    options.theme.pie_legend_text_color = foreground;
    options.theme.background = "none".into();

    let [red, green, blue, _] = request.foreground;
    let luminance = u32::from(red) * RED_LUMINANCE_WEIGHT
        + u32::from(green) * GREEN_LUMINANCE_WEIGHT
        + u32::from(blue) * BLUE_LUMINANCE_WEIGHT;
    if luminance >= LUMINANCE_THRESHOLD {
        apply_dark_mermaid_surface(&mut options.theme);
    }
    options
}

fn apply_dark_mermaid_surface(theme: &mut mermaid_rs_renderer::Theme) {
    theme.primary_color = "#242b38".into();
    theme.secondary_color = "#303949".into();
    theme.tertiary_color = "#1b2230".into();
    theme.primary_border_color = "#65758b".into();
    theme.line_color = "#aebbd0".into();
    theme.edge_label_background = theme.primary_color.clone();
    theme.cluster_background = theme.secondary_color.clone();
    theme.cluster_border = theme.primary_border_color.clone();
    theme.sequence_actor_fill = theme.primary_color.clone();
    theme.sequence_actor_border = theme.primary_border_color.clone();
    theme.sequence_actor_line = theme.line_color.clone();
    theme.sequence_note_fill = theme.secondary_color.clone();
    theme.sequence_note_border = theme.primary_border_color.clone();
    theme.sequence_activation_fill = theme.tertiary_color.clone();
    theme.sequence_activation_border = theme.primary_border_color.clone();
}

fn rasterize_svg(svg: &str, background: [u8; 4], baseline: f32) -> CachedRender {
    rasterize_svg_with_dpi(svg, background, baseline, MERMAID_SVG_DPI)
}

fn rasterize_math_svg(svg: &str, background: [u8; 4], baseline: f32) -> CachedRender {
    rasterize_svg_with_dpi(svg, background, baseline, MATH_SVG_DPI)
}

fn rasterize_svg_with_dpi(svg: &str, background: [u8; 4], baseline: f32, dpi: f32) -> CachedRender {
    if svg.len() > MAX_SVG_BYTES {
        return Err(EmbeddedError::Render("generated SVG is too large".into()));
    }
    let mut options = resvg::usvg::Options { dpi, ..resvg::usvg::Options::default() };
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    if svg.contains("<text ") {
        options.fontdb = Arc::clone(SYSTEM_FONTS.get_or_init(|| {
            let mut fonts = resvg::usvg::fontdb::Database::new();
            fonts.load_system_fonts();
            Arc::new(fonts)
        }));
    }
    let tree = resvg::usvg::Tree::from_str(svg, &options)
        .map_err(|error| EmbeddedError::Render(error.to_string()))?;
    let width = checked_dimension(tree.size().width())?;
    let height = checked_dimension(tree.size().height())?;
    if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return Err(EmbeddedError::Render("image pixel area exceeds limit".into()));
    }
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| EmbeddedError::Render("cannot allocate image pixels".into()))?;
    if background[3] != 0 {
        pixmap.fill(resvg::tiny_skia::Color::from_rgba8(
            background[0],
            background[1],
            background[2],
            background[3],
        ));
    }
    resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pixmap.as_mut());
    let image = RasterImage::new(width, height, pixmap.take())
        .map_err(|error| EmbeddedError::Render(format!("invalid raster image: {error:?}")))?;
    Ok(EmbeddedImage { image: Arc::new(image), baseline })
}

fn checked_dimension(dimension: f32) -> Result<u32, EmbeddedError> {
    if !dimension.is_finite() || dimension <= 0.0 || dimension > MAX_IMAGE_SIDE as f32 {
        return Err(EmbeddedError::Render("image dimension exceeds limit".into()));
    }
    Ok(dimension.ceil() as u32)
}

fn trim_cache(entries: &mut ImageCache) {
    let mut byte_count: usize = entries
        .iter()
        .map(|(_, rendered)| rendered.as_ref().map_or(0, |image| image.image.pixels().len()))
        .sum();
    while entries.len() > MAX_CACHE_ENTRIES || byte_count > MAX_CACHE_BYTES {
        if let Some((_, rendered)) = entries.pop_front() {
            byte_count -= rendered.as_ref().map_or(0, |image| image.image.pixels().len());
        }
    }
}

#[cfg(test)]
mod embedded_tests {
    use super::*;

    fn request(kind: EmbeddedKind, source: &str) -> EmbeddedRequest<'_> {
        EmbeddedRequest {
            kind,
            source,
            font_size: 24.0,
            foreground: [240, 230, 220, 255],
            background: [0, 0, 0, 0],
        }
    }

    fn assert_visible(image: &EmbeddedImage) {
        assert!(image.image.width() > 0 && image.image.height() > 0);
        assert_eq!(
            image.image.pixels().len(),
            image.image.width() as usize * image.image.height() as usize * RGBA_CHANNELS
        );
        assert!(image.image.pixels().chunks_exact(RGBA_CHANNELS).any(|pixel| pixel[3] > 0));
    }

    #[test]
    fn embedded_math_structures_have_visible_pixels() {
        for source in [r"\frac{1}{2}", r"\sqrt{x^2_1}", r"\begin{matrix}a&b\\c&d\end{matrix}"] {
            let image = render_embedded(request(EmbeddedKind::DisplayMath, source))
                .expect("common mathematics should rasterize");
            assert_visible(&image);
            assert!(image.baseline > 0.0 && image.baseline <= image.image.height() as f32);
        }
    }

    #[test]
    fn embedded_math_bitmap_and_baseline_use_requested_pixel_size() {
        let source = "x";
        let request = request(EmbeddedKind::InlineMath, source);
        let nodes = ratex_parser::parser::Parser::new(source)
            .parse()
            .expect("test expression should parse");
        let layout_options = ratex_layout::LayoutOptions::default().with_style(MathStyle::Text);
        let layout_box = ratex_layout::layout(&nodes, &layout_options);
        let font_size = f64::from(request.font_size);
        let padding = font_size * MATH_PADDING_RATIO;
        let expected_width = (layout_box.width * font_size + 2.0 * padding).ceil() as u32;
        let expected_height =
            ((layout_box.height + layout_box.depth) * font_size + 2.0 * padding).ceil() as u32;
        let expected_baseline = (layout_box.height * font_size + padding) as f32;

        let rendered = render_embedded(request).expect("inline math should rasterize");
        assert_eq!(rendered.image.width(), expected_width);
        assert_eq!(rendered.image.height(), expected_height);
        assert!((rendered.baseline - expected_baseline).abs() < 0.01);
    }

    #[test]
    fn embedded_mermaid_svg_keeps_default_css_dpi() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="72pt" height="72pt"><rect width="72pt" height="72pt" fill="red"/></svg>"#;
        let rendered = rasterize_svg(svg, [0, 0, 0, 0], 0.0)
            .expect("Mermaid SVG should retain CSS pixel conversion");
        assert_eq!(rendered.image.width(), 96);
        assert_eq!(rendered.image.height(), 96);
    }

    #[test]
    fn embedded_mermaid_diagrams_have_visible_pixels() {
        for source in ["flowchart LR\nA[开始] --> B[结束]", "sequenceDiagram\nAlice->>Bob: 你好"]
        {
            let image = render_embedded(request(EmbeddedKind::Mermaid, source))
                .expect("supported Mermaid syntax should rasterize");
            assert_visible(&image);
        }
    }

    #[test]
    fn embedded_invalid_sources_return_errors() {
        assert!(render_embedded(request(EmbeddedKind::InlineMath, r"\frac{1}{")).is_err());
        assert!(render_embedded(request(EmbeddedKind::Mermaid, "not a diagram")).is_err());
    }

    #[test]
    fn embedded_cache_reuses_identical_images_and_separates_render_settings() {
        let source = r"\sqrt{x}";
        let first = render_embedded(request(EmbeddedKind::InlineMath, source))
            .expect("valid math should render");
        let second = render_embedded(request(EmbeddedKind::InlineMath, source))
            .expect("cached math should render");
        assert!(Arc::ptr_eq(&first.image, &second.image));
        assert_eq!(first.image.id(), second.image.id());

        let mut changed = request(EmbeddedKind::InlineMath, source);
        changed.foreground = [255, 0, 0, 255];
        let recolored = render_embedded(changed).expect("recolored math should render");
        assert!(!Arc::ptr_eq(&first.image, &recolored.image));

        changed.font_size = 36.0;
        let resized = render_embedded(changed).expect("resized math should render");
        assert!(!Arc::ptr_eq(&recolored.image, &resized.image));
    }

    #[test]
    fn embedded_rejects_oversized_input_and_font() {
        let enormous = "x".repeat(100_000);
        assert!(render_embedded(request(EmbeddedKind::InlineMath, &enormous)).is_err());
        let mut invalid = request(EmbeddedKind::InlineMath, "x");
        invalid.font_size = f32::INFINITY;
        assert!(render_embedded(invalid).is_err());
    }

    #[test]
    fn embedded_chinese_mermaid_label_changes_raster_pixels() {
        let source = "flowchart LR\nA[中文标签]";
        let svg = mermaid_rs_renderer::render_strict(
            source,
            mermaid_rs_renderer::RenderOptions::default(),
        )
        .expect("Chinese Mermaid label should parse");
        assert!(svg.contains("中文标签"));
        let labeled =
            rasterize_svg(&svg, [0, 0, 0, 0], 0.0).expect("Chinese label should rasterize");
        let blank = rasterize_svg(&svg.replace("中文标签", ""), [0, 0, 0, 0], 0.0)
            .expect("same diagram without label should rasterize");
        assert_eq!(
            (labeled.image.width(), labeled.image.height()),
            (blank.image.width(), blank.image.height())
        );
        assert_ne!(
            labeled.image.pixels(),
            blank.image.pixels(),
            "Chinese glyphs must affect pixels"
        );
    }

    #[test]
    fn embedded_dark_theme_mermaid_uses_contrasting_node_fill() {
        let dark_request = request(EmbeddedKind::Mermaid, "flowchart LR\nA[文字]");
        let image = render_embedded(dark_request).expect("Mermaid should rasterize");
        let dark_pixels = image
            .image
            .pixels()
            .chunks_exact(RGBA_CHANNELS)
            .filter(|pixel| pixel[3] > 0 && pixel[0] < 128 && pixel[1] < 128 && pixel[2] < 128)
            .count();
        let light_pixels = image
            .image
            .pixels()
            .chunks_exact(RGBA_CHANNELS)
            .filter(|pixel| pixel[3] > 0 && pixel[0] >= 128 && pixel[1] >= 128 && pixel[2] >= 128)
            .count();
        assert!(dark_pixels > light_pixels, "light label needs a dark diagram surface");
    }

    #[test]
    fn embedded_image_side_fits_gpu_atlas() {
        assert_eq!(MAX_IMAGE_SIDE, 4094);
    }

    #[test]
    fn embedded_failed_parse_is_cached_once() {
        let source = r"\frac{cache_failure}{";
        for _ in 0..2 {
            assert!(render_embedded(request(EmbeddedKind::InlineMath, source)).is_err());
        }
        let entries = IMAGE_CACHE
            .get()
            .expect("render call initializes cache")
            .lock()
            .expect("test cache mutex should not be poisoned");
        assert_eq!(entries.iter().filter(|(key, _)| key.source == source).count(), 1);
    }

    #[test]
    fn embedded_rejects_oversized_svg_before_allocation() {
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"1\"/>",
            MAX_IMAGE_SIDE + 1
        );
        assert!(rasterize_svg(&svg, [0, 0, 0, 0], 0.0).is_err());
    }

    #[test]
    fn embedded_foreground_changes_actual_math_pixels() {
        let source = "x+1";
        let original =
            render_embedded(request(EmbeddedKind::InlineMath, source)).expect("math should render");
        let mut recolored = request(EmbeddedKind::InlineMath, source);
        recolored.foreground = [255, 0, 0, 255];
        let red = render_embedded(recolored).expect("recolored math should render");
        assert_ne!(original.image.pixels(), red.image.pixels());
        assert!(
            red.image
                .pixels()
                .chunks_exact(RGBA_CHANNELS)
                .any(|pixel| { pixel[3] > 0 && pixel[0] > pixel[1] && pixel[0] > pixel[2] })
        );
    }

    #[test]
    fn embedded_error_has_a_readable_description() {
        let error = render_embedded(request(EmbeddedKind::Mermaid, "not a diagram"))
            .expect_err("invalid Mermaid syntax should fail");
        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn embedded_parallel_requests_share_one_raster_identity() {
        const WORKERS: usize = 8;
        const SOURCE: &str = r"\sqrt{parallel_cache_identity_2026}";
        let gate = Arc::new(std::sync::Barrier::new(WORKERS));
        let threads: Vec<_> = (0..WORKERS)
            .map(|_| {
                let gate = Arc::clone(&gate);
                std::thread::spawn(move || {
                    gate.wait();
                    render_embedded(request(EmbeddedKind::DisplayMath, SOURCE))
                        .expect("parallel render should succeed")
                        .image
                        .id()
                })
            })
            .collect();
        let identities: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().expect("render worker must finish"))
            .collect();
        assert!(identities.iter().all(|identity| *identity == identities[0]));
    }
}
