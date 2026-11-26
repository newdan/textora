//! 产品 chrome 与编辑器共享的单帧借用 API。

use crate::editor_runtime::PaintedEditorBounds;
use crate::gpu::GpuError;
use crate::render_state::{GpuState, TextState};
use render::GlyphVertex;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct RenderResources {
    pub text: Option<TextState>,
    pub gpu: Option<GpuState>,
    pub frame_cache: crate::frame_cache::FrameCache,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EmptyFrameTarget {
    Surface,
    MultisampledWhenTextAvailable,
}

impl EmptyFrameTarget {
    fn uses_multisampling(self, has_text: bool, has_vertices: bool) -> bool {
        has_text && (has_vertices || self == Self::MultisampledWhenTextAvailable)
    }
}

#[derive(Clone, Copy)]
pub enum WindowCornerClip {
    Preserve,
    Synchronize(Option<f32>),
}

pub struct SurfaceFrameOptions {
    pub background: [f32; 4],
    pub empty_frame_target: EmptyFrameTarget,
    pub window_corner_clip: WindowCornerClip,
}

pub struct SurfaceFrameSubmission {
    pub present_returned_at: Instant,
    pub submit_and_present: Duration,
    pub preparation: Option<SurfaceFramePreparation>,
}

pub struct SurfaceFramePreparation {
    pub acquire_started_at: Instant,
    pub acquire: Duration,
    pub encode: Duration,
}

impl RenderResources {
    /// 提交已按产品顺序组合的顶点，并返回 GPU 队列提交与呈现的耗时。
    /// 缺少 GPU 或无法取得当前表面纹理时，返回 `Ok(None)`。
    pub fn submit_surface_frame(
        &mut self,
        vertices: &[GlyphVertex],
        options: SurfaceFrameOptions,
    ) -> Result<Option<Duration>, GpuError> {
        Ok(self
            .submit_surface_frame_with_timing(vertices, options, false)?
            .map(|submission| submission.submit_and_present))
    }

    pub fn submit_surface_frame_with_timing(
        &mut self,
        vertices: &[GlyphVertex],
        options: SurfaceFrameOptions,
        measure_preparation: bool,
    ) -> Result<Option<SurfaceFrameSubmission>, GpuError> {
        let Some(gpu) = self.gpu.as_mut() else {
            return Ok(None);
        };
        if let WindowCornerClip::Synchronize(radius) = options.window_corner_clip {
            gpu.ctx.set_window_corner_radius(radius)?;
        }
        let acquire_started_at = measure_preparation.then(Instant::now);
        let output = match gpu.ctx.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            _ => return Ok(None),
        };
        let acquire_finished_at = acquire_started_at.map(|started_at| (started_at, Instant::now()));
        let surface_view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu.ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("editor frame encoder"),
        });
        let (attachment_view, resolve_target) = if options
            .empty_frame_target
            .uses_multisampling(self.text.is_some(), !vertices.is_empty())
        {
            (&gpu.ctx.msaa_view, Some(&surface_view))
        } else {
            (&surface_view, None)
        };
        let text = self.text.as_mut().filter(|_| !vertices.is_empty()).map(|text| {
            upload_frame_vertices(text, gpu, vertices);
            &*text
        });
        encode_surface_pass(
            &mut encoder,
            attachment_view,
            resolve_target,
            options.background,
            text,
            vertices.len() as u32,
        );

        let commands = encoder.finish();
        let submit_started_at = Instant::now();
        gpu.ctx.queue.submit(std::iter::once(commands));
        output.present();
        let present_returned_at = Instant::now();
        Ok(Some(SurfaceFrameSubmission {
            present_returned_at,
            submit_and_present: present_returned_at - submit_started_at,
            preparation: acquire_finished_at.map(|(acquire_started_at, encode_started_at)| {
                SurfaceFramePreparation {
                    acquire_started_at,
                    acquire: encode_started_at - acquire_started_at,
                    encode: submit_started_at - encode_started_at,
                }
            }),
        }))
    }
}

fn upload_frame_vertices(text: &mut TextState, gpu: &GpuState, vertices: &[GlyphVertex]) {
    let vertex_count = vertices.len() as u32;
    if vertex_count > text.vertex_capacity {
        let capacity = vertex_count.next_power_of_two();
        text.vertex_buffer = gpu.ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("editor frame vertex buffer"),
            size: (capacity as usize * std::mem::size_of::<GlyphVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        text.vertex_capacity = capacity;
    }
    gpu.ctx.queue.write_buffer(&text.vertex_buffer, 0, bytemuck::cast_slice(vertices));
}

fn encode_surface_pass(
    encoder: &mut wgpu::CommandEncoder,
    attachment_view: &wgpu::TextureView,
    resolve_target: Option<&wgpu::TextureView>,
    background: [f32; 4],
    text: Option<&TextState>,
    vertex_count: u32,
) {
    let [red, green, blue, alpha] = background.map(f64::from);
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("editor frame pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: attachment_view,
            depth_slice: None,
            resolve_target,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color { r: red, g: green, b: blue, a: alpha }),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    });
    if let Some(text) = text {
        pass.set_pipeline(text.renderer.pipeline());
        pass.set_bind_group(0, &text.bind_group, &[]);
        pass.set_vertex_buffer(0, text.vertex_buffer.slice(..));
        pass.draw(0..vertex_count, 0..1);
    }
}

/// Coverage curves calibrated against native GDI+ text on a 233-glyph,
/// 10-family corpus (same theme colors, 18.75 px): the shader applies
/// `coverage^(1/gamma)`, so light surfaces use `coverage^0.71` to match GDI+
/// ink mass, dark surfaces use `coverage^0.25`.
const LIGHT_BACKGROUND_GAMMA: f32 = 1.3;
const LIGHT_BACKGROUND_CONTRAST: f32 = 0.0;
const DARK_BACKGROUND_GAMMA: f32 = 4.0;
const DARK_BACKGROUND_CONTRAST: f32 = 0.0;
/// macOS has no calibrated native baseline yet; keep raw coverage until it is
/// measured against CoreText output.
const MACOS_BACKGROUND_GAMMA: f32 = 1.0;
const DARK_BACKGROUND_LUMINANCE_THRESHOLD: f32 = 0.5;

/// 帧组合和提交错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    InvalidEditorRect,
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidEditorRect => {
                formatter.write_str("editor rect must be finite and non-negative")
            }
        }
    }
}

impl std::error::Error for RenderError {}

/// 按 underlay、editor、overlay 顺序组合产品 chrome 和编辑器内容的短生命周期对象。
pub struct EditorFrame {
    product_underlay: ui::DrawList,
    editor_draw_list: ui::DrawList,
    product_overlay: ui::DrawList,
    text_measure: ui::NoopMeasure,
    theme: ui::Theme,
    dpi: f32,
    ui_shaper: Option<Arc<Mutex<shaping::Shaper>>>,
    editor_vertex_batches: Vec<EditorVertexBatch>,
    painted_editor_bounds: PaintedEditorBounds,
}

struct EditorVertexBatch {
    bounds: ui::Rect,
    vertices: Vec<GlyphVertex>,
}

impl EditorFrame {
    pub(crate) fn new_for_backend(
        theme: ui::Theme,
        dpi: f32,
        ui_shaper: Option<Arc<Mutex<shaping::Shaper>>>,
        painted_editor_bounds: PaintedEditorBounds,
    ) -> Self {
        Self {
            product_underlay: ui::DrawList::new(),
            editor_draw_list: ui::DrawList::new(),
            product_overlay: ui::DrawList::new(),
            text_measure: ui::NoopMeasure,
            theme,
            dpi,
            ui_shaper,
            editor_vertex_batches: Vec::new(),
            painted_editor_bounds,
        }
    }

    pub fn with_layout_context<T>(
        &mut self,
        layout: impl FnOnce(&mut ui::LayoutCtx<'_>) -> T,
    ) -> T {
        let Some(ui_shaper) = self.ui_shaper.as_ref() else {
            let mut context = ui::LayoutCtx {
                measure: &mut self.text_measure,
                ui_measure: None,
                theme: &self.theme,
                dpi: self.dpi,
            };
            return layout(&mut context);
        };
        let mut ui_shaper = ui_shaper
            .lock()
            .expect("UI shaper mutex must not be poisoned by another layout callback");
        let mut text_measure = crate::measure_adapter::MeasureFromShaper(&mut ui_shaper);
        let mut context = ui::LayoutCtx {
            measure: &mut text_measure,
            ui_measure: None,
            theme: &self.theme,
            dpi: self.dpi,
        };
        layout(&mut context)
    }

    pub fn with_paint_context<T>(&mut self, paint: impl FnOnce(&mut ui::PaintCtx<'_>) -> T) -> T {
        paint_with_context(
            &mut self.product_overlay,
            &self.theme,
            self.dpi,
            self.ui_shaper.as_ref(),
            paint,
        )
    }

    /// 绘制必须位于编辑器内容下方的产品背景与固定 chrome。
    pub fn with_underlay_paint_context<T>(
        &mut self,
        paint: impl FnOnce(&mut ui::PaintCtx<'_>) -> T,
    ) -> T {
        paint_with_context(
            &mut self.product_underlay,
            &self.theme,
            self.dpi,
            self.ui_shaper.as_ref(),
            paint,
        )
    }

    pub fn paint_editor(&mut self, editor_rect: ui::Rect) -> Result<(), RenderError> {
        self.paint_editor_with(editor_rect, |_| ()).map(|_| ())
    }

    pub fn paint_editor_with<T>(
        &mut self,
        editor_rect: ui::Rect,
        paint: impl FnOnce(&mut ui::PaintCtx<'_>) -> T,
    ) -> Result<Option<T>, RenderError> {
        if !is_valid_rect(editor_rect) {
            return Err(RenderError::InvalidEditorRect);
        }
        if editor_rect.w == 0.0 || editor_rect.h == 0.0 {
            return Ok(None);
        }
        self.painted_editor_bounds.set(Some(editor_rect));
        let mut result = None;
        let theme = &self.theme;
        let dpi = self.dpi;
        let mut ui_shaper = self.ui_shaper.as_ref().map(|shaper| {
            shaper.lock().expect("UI shaper mutex must not be poisoned by another paint callback")
        });
        self.editor_draw_list.clip(editor_rect, |draw_list| {
            let mut context = ui::PaintCtx {
                list: draw_list,
                theme,
                dpi,
                global_alpha: 1.0,
                shaper: ui_shaper.as_deref_mut(),
            };
            result = Some(paint(&mut context));
        });
        Ok(result)
    }

    /// 接收编辑器已经按物理屏幕坐标生成的顶点，确保它们与产品 chrome 共用一帧。
    pub fn paint_editor_vertices(
        &mut self,
        editor_rect: ui::Rect,
        vertices: impl IntoIterator<Item = GlyphVertex>,
    ) -> Result<(), RenderError> {
        if !is_valid_rect(editor_rect) {
            return Err(RenderError::InvalidEditorRect);
        }
        if editor_rect.w == 0.0 || editor_rect.h == 0.0 {
            return Ok(());
        }
        self.painted_editor_bounds.set(Some(editor_rect));
        let vertices = vertices.into_iter().collect();
        self.editor_vertex_batches.push(EditorVertexBatch { bounds: editor_rect, vertices });
        Ok(())
    }

    pub fn drain_into(
        &mut self,
        screen: ui::Screen,
        resources: &mut RenderResources,
        vertices: &mut Vec<GlyphVertex>,
    ) {
        if let (Some(text), Some(gpu)) = (resources.text.as_mut(), resources.gpu.as_ref()) {
            let gamma = gamma_for_theme(&self.theme);
            text.update_gamma_if_changed(&gpu.ctx.queue, gamma);
        }
        crate::paint_backend::drain_into(
            std::mem::take(&mut self.product_underlay),
            screen,
            resources.text.as_mut(),
            resources.gpu.as_ref(),
            vertices,
        );
        for batch in self.editor_vertex_batches.drain(..) {
            crate::paint_backend::append_clipped_triangles(
                vertices,
                &batch.vertices,
                batch.bounds,
                &screen,
            );
        }
        crate::paint_backend::drain_into(
            std::mem::take(&mut self.editor_draw_list),
            screen,
            resources.text.as_mut(),
            resources.gpu.as_ref(),
            vertices,
        );
        crate::paint_backend::drain_into(
            std::mem::take(&mut self.product_overlay),
            screen,
            resources.text.as_mut(),
            resources.gpu.as_ref(),
            vertices,
        );
    }
}

fn gamma_for_theme(theme: &ui::Theme) -> render::GammaUniform {
    if cfg!(target_os = "macos") {
        return render::GammaUniform { contrast: 0.0, gamma: MACOS_BACKGROUND_GAMMA };
    }
    let background = theme.editor.background;
    let luminance = 0.299 * background[0] + 0.587 * background[1] + 0.114 * background[2];
    if luminance < DARK_BACKGROUND_LUMINANCE_THRESHOLD {
        render::GammaUniform { contrast: DARK_BACKGROUND_CONTRAST, gamma: DARK_BACKGROUND_GAMMA }
    } else {
        render::GammaUniform { contrast: LIGHT_BACKGROUND_CONTRAST, gamma: LIGHT_BACKGROUND_GAMMA }
    }
}

fn paint_with_context<T>(
    list: &mut ui::DrawList,
    theme: &ui::Theme,
    dpi: f32,
    ui_shaper: Option<&Arc<Mutex<shaping::Shaper>>>,
    paint: impl FnOnce(&mut ui::PaintCtx<'_>) -> T,
) -> T {
    let mut ui_shaper = ui_shaper.map(|shaper| {
        shaper.lock().expect("UI shaper mutex must not be poisoned by another paint callback")
    });
    let mut context =
        ui::PaintCtx { list, theme, dpi, global_alpha: 1.0, shaper: ui_shaper.as_deref_mut() };
    paint(&mut context)
}

fn is_valid_rect(rect: ui::Rect) -> bool {
    [rect.x, rect.y, rect.w, rect.h].into_iter().all(f32::is_finite)
        && rect.x >= 0.0
        && rect.y >= 0.0
        && rect.w >= 0.0
        && rect.h >= 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    fn theme() -> ui::Theme {
        ui::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark())
    }

    #[test]
    fn empty_frame_multisampling_preserves_product_policy() {
        let direct_clear = EmptyFrameTarget::Surface;
        let multisampled_clear = EmptyFrameTarget::MultisampledWhenTextAvailable;

        assert!(!direct_clear.uses_multisampling(true, false));
        assert!(multisampled_clear.uses_multisampling(true, false));
        for target in [direct_clear, multisampled_clear] {
            assert!(target.uses_multisampling(true, true));
            assert!(!target.uses_multisampling(false, false));
            assert!(!target.uses_multisampling(false, true));
        }
    }

    #[test]
    fn headless_frame_submission_reports_no_presentation() {
        let mut resources = RenderResources {
            text: None,
            gpu: None,
            frame_cache: crate::frame_cache::FrameCache::new(),
        };

        let submission = resources.submit_surface_frame(
            &[],
            SurfaceFrameOptions {
                background: [0.0; 4],
                empty_frame_target: EmptyFrameTarget::Surface,
                window_corner_clip: WindowCornerClip::Synchronize(None),
            },
        );

        assert!(matches!(submission, Ok(None)));
    }

    #[test]
    fn gamma_follows_the_editor_background_luminance() {
        let dark = ui::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let light = ui::Theme::from_definition(&ui::theme::ThemeDefinition::default_light());

        if cfg!(target_os = "macos") {
            assert_eq!(gamma_for_theme(&dark).gamma, MACOS_BACKGROUND_GAMMA);
            assert_eq!(gamma_for_theme(&light).gamma, MACOS_BACKGROUND_GAMMA);
            return;
        }
        assert_eq!(gamma_for_theme(&dark).gamma, DARK_BACKGROUND_GAMMA);
        assert_eq!(gamma_for_theme(&light).gamma, LIGHT_BACKGROUND_GAMMA);
        assert_eq!(gamma_for_theme(&light).contrast, LIGHT_BACKGROUND_CONTRAST);
        assert_eq!(gamma_for_theme(&dark).contrast, DARK_BACKGROUND_CONTRAST);
    }

    fn frame(
        theme: ui::Theme,
        dpi: f32,
        ui_shaper: Option<Arc<Mutex<shaping::Shaper>>>,
    ) -> EditorFrame {
        EditorFrame::new_for_backend(theme, dpi, ui_shaper, Rc::new(Cell::new(None)))
    }

    fn editor_triangle(color: [f32; 4]) -> [GlyphVertex; 3] {
        [
            GlyphVertex { position: [0.25, 0.5], tex_coords: [0.0, 0.0], color },
            GlyphVertex { position: [0.3, 0.4], tex_coords: [1.0, 0.0], color },
            GlyphVertex { position: [0.2, 0.4], tex_coords: [0.0, 1.0], color },
        ]
    }

    #[test]
    fn layout_and_paint_contexts_are_only_borrowed_inside_callbacks() {
        let theme = theme();
        let mut frame = frame(theme, 2.0, None);

        let layout_dpi = frame.with_layout_context(|context| context.dpi);
        let paint_dpi = frame.with_paint_context(|context| {
            context.list.fill(ui::Rect::new(1.0, 2.0, 3.0, 4.0), [1.0; 4]);
            context.dpi
        });

        assert_eq!(layout_dpi, 2.0);
        assert_eq!(paint_dpi, 2.0);
    }

    #[test]
    fn product_text_is_shaped_into_the_frame_draw_list() {
        let theme = theme();
        let shaper = shaping::Shaper::new().expect("system fonts should create a UI shaper");
        let mut frame = frame(theme, 1.0, Some(Arc::new(Mutex::new(shaper))));

        frame.with_paint_context(|context| {
            context.text(12.0, 24.0, 14.0, [1.0; 4], "Workspace");
        });

        assert!(
            frame
                .product_overlay
                .cmds
                .iter()
                .any(|command| matches!(command, ui::DrawCmd::TextLayout { .. })),
            "product chrome text must produce a shaped draw command"
        );
    }

    #[test]
    #[cfg(not(feature = "ci-no-fonts"))]
    fn product_layout_context_measures_text_with_the_frame_shaper() {
        let theme = theme();
        let shaper = shaping::Shaper::new().expect("system fonts should create a UI shaper");
        let mut frame = frame(theme, 1.0, Some(Arc::new(Mutex::new(shaper))));

        let measured_width =
            frame.with_layout_context(|context| context.measure.measure("标题预编辑", 24.0));

        assert!(measured_width > 0.0, "product text input layout must use shaped text widths");
    }

    #[test]
    fn product_editor_and_overlay_layers_compose_in_one_frame() {
        let theme = theme();
        let mut frame = frame(theme, 1.0, None);

        frame.with_paint_context(|context| {
            context.list.fill(ui::Rect::new(0.0, 0.0, 10.0, 10.0), [0.0; 4]);
        });
        frame
            .paint_editor(ui::Rect::new(24.0, 36.0, 400.0, 300.0))
            .expect("finite editor rect should paint");
        frame.with_paint_context(|context| {
            context.list.fill(ui::Rect::new(5.0, 5.0, 6.0, 6.0), [1.0; 4]);
        });
    }

    #[test]
    fn invalid_or_zero_editor_rects_are_safe() {
        let theme = theme();
        let mut frame = frame(theme, 1.0, None);

        assert_eq!(
            frame.paint_editor(ui::Rect::new(-1.0, 0.0, 10.0, 10.0)),
            Err(RenderError::InvalidEditorRect)
        );
        frame.paint_editor(ui::Rect::ZERO).expect("zero rect should skip safely");
    }

    #[test]
    fn editor_vertices_are_submitted_before_product_chrome() {
        let theme = theme();
        let mut frame = frame(theme, 1.0, None);
        frame.with_paint_context(|context| {
            context.list.fill(ui::Rect::new(10.0, 10.0, 5.0, 5.0), [1.0; 4]);
        });
        frame
            .paint_editor_vertices(ui::Rect::new(20.0, 20.0, 80.0, 60.0), editor_triangle([0.0; 4]))
            .expect("finite editor rect should accept editor vertices");

        let mut resources = RenderResources {
            text: None,
            gpu: None,
            frame_cache: crate::frame_cache::FrameCache::new(),
        };
        let mut vertices = Vec::new();
        frame.drain_into(ui::Screen::new(100.0, 100.0), &mut resources, &mut vertices);

        assert_eq!(vertices.first().expect("editor vertex must be retained").position, [0.25, 0.5]);
        assert_eq!(vertices.len(), 9, "one editor triangle plus one product quad");
    }

    #[test]
    fn editor_vertex_submission_records_the_runtime_input_bounds() {
        let painted_editor_bounds = Rc::new(Cell::new(None));
        let mut frame =
            EditorFrame::new_for_backend(theme(), 1.0, None, Rc::clone(&painted_editor_bounds));
        let editor_bounds = ui::Rect::new(20.0, 30.0, 800.0, 600.0);

        frame
            .paint_editor_vertices(editor_bounds, std::iter::empty())
            .expect("finite editor bounds should accept an empty vertex submission");

        assert_eq!(painted_editor_bounds.get(), Some(editor_bounds));
    }

    #[test]
    fn editor_vertices_are_clipped_to_the_product_owned_editor_bounds() {
        let mut frame = frame(theme(), 1.0, None);
        let editor_bounds = ui::Rect::new(20.0, 20.0, 60.0, 60.0);
        let crossing_top_edge = [
            GlyphVertex { position: [-0.5, 0.8], tex_coords: [0.0, 0.0], color: [1.0; 4] },
            GlyphVertex { position: [0.5, 0.4], tex_coords: [1.0, 0.0], color: [1.0; 4] },
            GlyphVertex { position: [-0.5, 0.4], tex_coords: [0.0, 1.0], color: [1.0; 4] },
        ];
        frame
            .paint_editor_vertices(editor_bounds, crossing_top_edge)
            .expect("finite editor bounds should clip crossing vertices");

        let mut resources = RenderResources {
            text: None,
            gpu: None,
            frame_cache: crate::frame_cache::FrameCache::new(),
        };
        let screen = ui::Screen::new(100.0, 100.0);
        let clip_top_ndc = screen.rect_to_ndc(editor_bounds)[2];
        let mut vertices = Vec::new();
        frame.drain_into(screen, &mut resources, &mut vertices);

        assert!(!vertices.is_empty());
        assert!(
            vertices.iter().all(|vertex| vertex.position[1] <= clip_top_ndc),
            "editor vertices must not invade product chrome above the editor bounds"
        );
    }

    #[test]
    fn explicit_editor_layer_is_between_product_underlay_and_overlay() {
        let theme = theme();
        let mut frame = frame(theme, 1.0, None);
        let underlay_color = [0.1, 0.2, 0.3, 1.0];
        let editor_color = [0.4, 0.5, 0.6, 1.0];
        let overlay_color = [0.7, 0.8, 0.9, 1.0];

        frame.with_underlay_paint_context(|context| {
            context.list.fill(ui::Rect::new(0.0, 0.0, 10.0, 10.0), underlay_color);
        });
        frame
            .paint_editor(ui::Rect::new(20.0, 20.0, 80.0, 60.0))
            .expect("editor layer should divide product paint phases");
        frame
            .paint_editor_vertices(
                ui::Rect::new(20.0, 20.0, 80.0, 60.0),
                editor_triangle(editor_color),
            )
            .expect("editor layer should accept vertices");
        frame.with_paint_context(|context| {
            context.list.fill(ui::Rect::new(30.0, 30.0, 5.0, 5.0), overlay_color);
        });

        let mut resources = RenderResources {
            text: None,
            gpu: None,
            frame_cache: crate::frame_cache::FrameCache::new(),
        };
        let mut vertices = Vec::new();
        frame.drain_into(ui::Screen::new(100.0, 100.0), &mut resources, &mut vertices);

        assert_eq!(vertices.len(), 15, "two product quads should surround one editor triangle");
        assert_eq!(vertices[0].color, underlay_color);
        assert_eq!(vertices[6].color, editor_color);
        assert_eq!(vertices[9].color, overlay_color);
    }
}
