//! 产品矩形内的共享编辑器绘制。

use render::GlyphVertex;
use std::time::Instant;
use ui::canvas::CanvasViewportSnapshot;

use super::{EditorFrame, EditorRuntime, RenderError, RenderResources};
use crate::tab_session::{TabSession, TabSessionMut};

const PLUGIN_CONTENT_TOP_PADDING_LOGICAL: f32 = 16.0;
const PLUGIN_CONTENT_BOTTOM_PADDING_LOGICAL: f32 = 24.0;
const ACTIVE_SEARCH_OVERLAY_OPACITY: f32 = 0.28;
const INACTIVE_SEARCH_OVERLAY_OPACITY: f32 = 0.14;

fn search_overlay_colors(theme: &ui::Theme) -> ([f32; 4], [f32; 4]) {
    let mut active = theme.palette.highlight;
    active[3] = active[3].min(ACTIVE_SEARCH_OVERLAY_OPACITY);
    let mut inactive = theme.palette.inactive_highlight;
    inactive[3] = inactive[3].min(INACTIVE_SEARCH_OVERLAY_OPACITY);
    (active, inactive)
}

#[derive(Clone, Copy, Default)]
pub enum PreeditAdvanceMeasure {
    #[default]
    NonNegative,
    AtLeastOnePixel,
}

impl PreeditAdvanceMeasure {
    fn measure(self, shaper: &mut shaping::Shaper, preedit_text: &str, font_size: f32) -> f32 {
        if preedit_text.is_empty() {
            return 0.0;
        }
        let minimum_advance = match self {
            Self::NonNegative => 0.0,
            Self::AtLeastOnePixel => 1.0,
        };
        let previous_font_size = shaper.font_size();
        shaper.set_font_size(font_size);
        let advance = shaper
            .shape(preedit_text)
            .map(|shaped| {
                shaped.clusters.iter().map(|cluster| cluster.advance.max(minimum_advance)).sum()
            })
            .unwrap_or(0.0);
        shaper.set_font_size(previous_font_size);
        advance
    }
}

fn plain_text_preedit_origin(
    preedit_text: &str,
    cursor_visual_line: Option<usize>,
    cursor_x_px: f32,
    editor_top_px: f32,
    line_height_px: f32,
    sub_line_offset_px: f32,
) -> Option<(f32, f32)> {
    if preedit_text.is_empty() {
        return None;
    }

    let cursor_visual_line = cursor_visual_line?;
    let cursor_y_px =
        editor_top_px + cursor_visual_line as f32 * line_height_px + sub_line_offset_px;
    Some((cursor_x_px, cursor_y_px))
}

fn editor_viewport_dimensions(editor_height_px: f32, line_height_px: f32) -> (usize, f64) {
    let viewport_height = (editor_height_px / line_height_px).max(1.0);
    (viewport_height.floor() as usize, viewport_height as f64)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EditorSurfacePaint {
    #[default]
    Empty,
    Document {
        vertex_count: usize,
    },
}

#[derive(Clone, Copy)]
struct EditorPaintStyle<'a> {
    theme: &'a ui::Theme,
    settings: &'a ui::Settings,
    metrics: &'a ui::UiMetrics,
}

#[derive(Clone, Copy)]
struct TextSurfaceLayout {
    screen: ui::Screen,
    left_margin: f32,
    top: f32,
    gutter_width: f32,
}

struct ShapedTextSurface {
    vertices: Vec<GlyphVertex>,
    tree_dirty: bool,
    preedit_advance_px: f32,
}

/// 本帧内容绘制和命中测试共用的窗口物理坐标。
#[derive(Clone, Copy)]
pub(super) enum EditorContentGeometry {
    Source { text_left: f32 },
    Plugin(ui::Rect),
}

/// 插件已经同步源文本与交互状态，使用产品布局提供的内容几何。
#[derive(Clone, Copy)]
pub enum PreparedPluginContent<'a> {
    Bounds(ui::Rect),
    Canvas(&'a CanvasViewportSnapshot),
}

#[derive(Clone, Copy, Default)]
pub enum SearchHighlightStyle {
    Theme,
    #[default]
    TranslucentWhilePanelVisible,
}

#[derive(Clone, Copy, Default)]
pub enum TextHighlightLayer {
    BelowGlyphs,
    #[default]
    AboveGlyphs,
}

#[derive(Clone, Copy, Default)]
pub enum TextCaretReveal {
    SourceLine,
    #[default]
    VisualRow,
}

#[derive(Default)]
pub struct EditorPaintOptions<'a> {
    pub plugin_content: Option<PreparedPluginContent<'a>>,
    /// 文本起点的窗口物理坐标，包含侧栏、目录和行号栏。
    pub text_left_margin: Option<f32>,
    pub gutter_background: bool,
    pub search_highlights: SearchHighlightStyle,
    pub text_highlight_layer: TextHighlightLayer,
    /// 查找栏取得焦点时，编辑器保留压暗后的光标并停止绘制预编辑内容。
    pub search_caret_opacity: Option<f32>,
    pub preedit_advance: PreeditAdvanceMeasure,
    /// 光标滚出视口时，预编辑文字可使用产品指定的窗口物理坐标。
    pub preedit_fallback_origin: Option<(f32, f32)>,
    pub caret_reveal: TextCaretReveal,
    /// 启用源文本后台预排版，宽度与产品视口的换行宽度一致。
    pub reshape_ahead_width: Option<f32>,
}

impl EditorRuntime {
    pub fn paint_active_editor(
        &mut self,
        frame: &mut EditorFrame,
        resources: &mut RenderResources,
        editor_rect: ui::Rect,
    ) -> Result<EditorSurfacePaint, RenderError> {
        self.paint_active_editor_with(frame, resources, editor_rect, EditorPaintOptions::default())
    }

    pub fn paint_active_editor_with(
        &mut self,
        frame: &mut EditorFrame,
        resources: &mut RenderResources,
        editor_rect: ui::Rect,
        options: EditorPaintOptions<'_>,
    ) -> Result<EditorSurfacePaint, RenderError> {
        let Some(tab_id) = self.active_tab_id() else {
            return Ok(EditorSurfacePaint::Empty);
        };
        frame.paint_editor(editor_rect)?;
        if editor_rect.w == 0.0 || editor_rect.h == 0.0 {
            return Ok(EditorSurfacePaint::Document { vertex_count: 0 });
        }

        let settings = self.settings.clone();
        let theme = self.theme.clone();
        let dpi = self.render_session.scale_factor() as f32;
        let metrics = ui::settings::UiMetrics::from_settings(&settings, dpi);
        let style = EditorPaintStyle { theme: &theme, settings: &settings, metrics: &metrics };
        let screen = editor_screen(resources, editor_rect);
        self.plain_text_preedit_advance_px = 0.0;
        let Some(tab) = self.tab_session(tab_id) else {
            return Ok(EditorSurfacePaint::Empty);
        };
        let mut vertices = Vec::new();
        let content_geometry = if tab.handles_own_rendering() {
            let bounds = match options.plugin_content {
                Some(PreparedPluginContent::Bounds(bounds)) => bounds,
                Some(PreparedPluginContent::Canvas(snapshot)) => snapshot.viewport,
                None => plugin_bounds(editor_rect, dpi, tab.is_canvas()),
            };
            paint_plugin_editor(
                self,
                tab_id,
                resources,
                bounds,
                screen,
                &style,
                &options,
                &mut vertices,
            );
            EditorContentGeometry::Plugin(bounds)
        } else {
            let layout = text_surface_layout(&tab, editor_rect, screen, style, &options);
            paint_source_surface(
                self,
                tab_id,
                resources,
                editor_rect,
                layout,
                &style,
                &options,
                &mut vertices,
            );
            EditorContentGeometry::Source { text_left: layout.left_margin }
        };
        let vertex_count = vertices.len();
        frame.paint_editor_vertices(editor_rect, vertices)?;
        self.painted_editor_content = Some((tab_id, content_geometry));
        Ok(EditorSurfacePaint::Document { vertex_count })
    }
    fn active_editor_content_geometry(&self) -> Option<EditorContentGeometry> {
        let (painted_tab, geometry) = self.painted_editor_content?;
        (Some(painted_tab) == self.active_tab_id()).then_some(geometry)
    }

    pub(super) fn plugin_input_bounds(&self, editor_rect: ui::Rect, is_canvas: bool) -> ui::Rect {
        match self.active_editor_content_geometry() {
            Some(EditorContentGeometry::Plugin(bounds)) => bounds,
            _ => plugin_bounds(editor_rect, self.scale_factor() as f32, is_canvas),
        }
    }

    pub(super) fn source_input_left(&self) -> Option<f32> {
        match self.active_editor_content_geometry() {
            Some(EditorContentGeometry::Source { text_left }) => Some(text_left),
            _ => None,
        }
    }
}

fn paint_plugin_editor(
    runtime: &mut EditorRuntime,
    tab_id: appkit_core::tab_set::types::TabId,
    resources: &mut RenderResources,
    bounds: ui::Rect,
    screen: ui::Screen,
    style: &EditorPaintStyle<'_>,
    options: &EditorPaintOptions<'_>,
    vertices: &mut Vec<GlyphVertex>,
) {
    let (Some(text), Some(_)) = (resources.text.as_mut(), resources.gpu.as_ref()) else {
        return;
    };
    let cursor_paint_enabled = runtime.active_cursor_paint_enabled();
    let preedit = options.plugin_content.is_none().then(|| runtime.preedit());
    let Some(mut tab) = runtime.tab_session_mut(tab_id) else {
        return;
    };
    if let Some(preedit) = preedit {
        let preedit_active = !preedit.0.is_empty();
        synchronize_plugin_document(&mut tab, preedit);
        if preedit_active {
            tab.invalidate_cursor_visibility();
        }
    }
    configure_plugin_paint(&mut tab, style, cursor_paint_enabled);
    let content = match options.plugin_content {
        Some(prepared) => render_prepared_plugin(&mut tab, prepared, style, &mut text.shaper),
        None => render_plugin_in_editor(&mut tab, bounds, style, &mut text.shaper),
    };
    let tab = TabSession::new(tab.id, tab.document, tab.runtime);
    paint_plugin_layers(
        &tab,
        content,
        resources,
        screen,
        style.theme,
        options.search_highlights,
        vertices,
    );
}

fn configure_plugin_paint(
    tab: &mut TabSessionMut<'_>,
    style: &EditorPaintStyle<'_>,
    cursor_paint_enabled: bool,
) {
    let EditorPaintStyle { settings, metrics, .. } = style;
    let dpi = metrics.dpi;
    tab.set_view_render_settings(ui::plugin::ViewRenderSettings {
        font_family: settings.font_family.clone(),
        font_size: metrics.font_size / dpi,
        line_height: metrics.line_height / dpi,
        toc_max_depth: settings.toc_max_depth,
        markdown_first_line_indent: settings.markdown_first_line_indent,
    });
    tab.set_view_text_spacing_mode(settings.text_spacing_mode);
    let cursor_visible = cursor_paint_enabled
        && super::cursor_blink_phase(tab.cursor_blink_instant(), Instant::now()).visible;
    tab.set_cursor_visible(cursor_visible);
}

fn render_prepared_plugin(
    tab: &mut TabSessionMut<'_>,
    prepared: PreparedPluginContent<'_>,
    style: &EditorPaintStyle<'_>,
    shaper: &mut shaping::Shaper,
) -> ui::DrawList {
    let dpi = style.metrics.dpi;
    match prepared {
        PreparedPluginContent::Bounds(bounds) => {
            tab.render_plugin(bounds, style.theme, shaper, dpi)
        }
        PreparedPluginContent::Canvas(snapshot) => {
            tab.render_canvas_plugin(snapshot, style.theme, shaper, dpi)
        }
    }
}

fn render_plugin_in_editor(
    tab: &mut TabSessionMut<'_>,
    bounds: ui::Rect,
    style: &EditorPaintStyle<'_>,
    shaper: &mut shaping::Shaper,
) -> ui::DrawList {
    let dpi = style.metrics.dpi;
    if !tab.is_canvas() {
        return render_plugin_with_visible_cursor(tab, bounds, style.theme, shaper, dpi);
    }
    tab.prepare_canvas_plugin(style.theme, shaper, dpi)
        .and_then(|metrics| tab.prepare_canvas_viewport(metrics, bounds, dpi))
        .map_or_else(ui::DrawList::new, |snapshot| {
            tab.render_canvas_plugin(&snapshot, style.theme, shaper, dpi)
        })
}

fn paint_plugin_layers(
    tab: &TabSession<'_>,
    content: ui::DrawList,
    resources: &mut RenderResources,
    screen: ui::Screen,
    theme: &ui::Theme,
    search_highlights: SearchHighlightStyle,
    vertices: &mut Vec<GlyphVertex>,
) {
    let gpu = resources.gpu.as_ref();
    if let (Some(text), Some(gpu)) = (resources.text.as_mut(), gpu) {
        crate::paint_backend::drain_into(content, screen, Some(text), Some(gpu), vertices);
    }
    if tab.handles_own_rendering() && tab.plugin_has_selection() {
        let selection = tab.selection_highlights(theme.editor.selection);
        if !selection.cmds.is_empty() {
            crate::paint_backend::drain_into(
                selection,
                screen,
                resources.text.as_mut(),
                gpu,
                vertices,
            );
        }
    }
    if let Some(search) = plugin_search_layer(tab, theme, search_highlights)
        && !search.cmds.is_empty()
    {
        crate::paint_backend::drain_into(search, screen, resources.text.as_mut(), gpu, vertices);
    }
}

fn plugin_search_layer(
    tab: &TabSession<'_>,
    theme: &ui::Theme,
    highlights: SearchHighlightStyle,
) -> Option<ui::DrawList> {
    let search = tab.search_state();
    if !search.is_active() {
        return None;
    }
    let (active_color, inactive_color) = match highlights {
        SearchHighlightStyle::Theme => (theme.palette.highlight, theme.palette.inactive_highlight),
        SearchHighlightStyle::TranslucentWhilePanelVisible if search.panel_visible => {
            search_overlay_colors(theme)
        }
        SearchHighlightStyle::TranslucentWhilePanelVisible => return None,
    };
    Some(tab.search_highlights(
        search.query.clone(),
        search.options.match_case,
        search.options.use_regex,
        search.active_match_idx,
        active_color,
        inactive_color,
    ))
}

fn render_plugin_with_visible_cursor(
    tab: &mut crate::tab_session::TabSessionMut<'_>,
    bounds: ui::Rect,
    theme: &ui::Theme,
    shaper: &mut shaping::Shaper,
    dpi: f32,
) -> ui::DrawList {
    let mut draw_list = tab.render_plugin(bounds, theme, shaper, dpi);
    let cursor_offset = tab.document.cursor().offset;
    if cursor_offset == tab.last_cursor_offset() {
        return draw_list;
    }
    tab.set_last_cursor_offset(cursor_offset);

    let cursor_byte = cursor_offset.to_usize();
    let cursor_rect = TabSession::new(tab.id, &*tab.document, &*tab.runtime)
        .query_cursor_screen_rect(cursor_byte);
    let scroll_delta = cursor_rect.and_then(|(_, cursor_y, _, cursor_height)| {
        plugin_cursor_visibility_scroll_delta(cursor_y, cursor_height, bounds.h)
    });
    let Some(scroll_delta) = scroll_delta else {
        return draw_list;
    };
    if !tab.scroll_plugin(scroll_delta, bounds.h) {
        return draw_list;
    }

    draw_list = tab.render_plugin(bounds, theme, shaper, dpi);
    draw_list
}

pub(super) fn plugin_cursor_visibility_scroll_delta(
    cursor_y: f32,
    cursor_height: f32,
    viewport_height: f32,
) -> Option<f32> {
    if !cursor_y.is_finite()
        || !cursor_height.is_finite()
        || !viewport_height.is_finite()
        || cursor_height < 0.0
        || viewport_height <= 0.0
    {
        return None;
    }
    if cursor_y < 0.0 {
        return Some(cursor_y);
    }

    let overflow_below = cursor_y + cursor_height - viewport_height;
    (overflow_below > 0.0).then_some(overflow_below)
}

fn synchronize_plugin_document(
    tab: &mut crate::tab_session::TabSessionMut<'_>,
    preedit: (String, Option<(usize, usize)>),
) {
    tab.sync_document(ui::plugin::SyncReason::Normal);
    if let Some((start, end)) = tab.document.selection_range().filter(|(start, end)| start < end) {
        tab.set_plugin_selection_anchor_byte(Some(start));
        tab.set_plugin_selection_cursor_byte(Some(end));
    } else {
        tab.set_plugin_selection_anchor_byte(None);
        tab.set_plugin_selection_cursor_byte(None);
    }
    let byte_offset = tab.document.cursor_offset().to_usize();
    tab.set_cursor_byte(byte_offset);
    let affinity = tab.document.tb().cursor_affinity();
    tab.set_visual_caret(ui::plugin::VisualCaretPosition { byte_offset, affinity });
    tab.set_preedit(preedit.0, preedit.1);
}

fn text_surface_layout(
    tab: &TabSession<'_>,
    editor_rect: ui::Rect,
    screen: ui::Screen,
    style: EditorPaintStyle<'_>,
    options: &EditorPaintOptions<'_>,
) -> TextSurfaceLayout {
    let gutter_width = style.settings.gutter_width(tab.document.line_count()) * style.metrics.dpi;
    let left_margin = options
        .text_left_margin
        .unwrap_or_else(|| editor_rect.x + style.metrics.content_left_margin.max(gutter_width));
    TextSurfaceLayout { screen, left_margin, top: editor_rect.y, gutter_width }
}

fn paint_source_surface(
    runtime: &mut EditorRuntime,
    tab_id: appkit_core::tab_set::types::TabId,
    resources: &mut RenderResources,
    editor_rect: ui::Rect,
    layout: TextSurfaceLayout,
    style: &EditorPaintStyle<'_>,
    options: &EditorPaintOptions<'_>,
    vertices: &mut Vec<GlyphVertex>,
) {
    if options.gutter_background && layout.gutter_width > 0.0 {
        let mut gutter = ui::DrawList::new();
        gutter.fill(
            ui::Rect::new(
                layout.left_margin - layout.gutter_width,
                layout.top,
                layout.gutter_width,
                editor_rect.h,
            ),
            style.theme.editor.gutter_bg,
        );
        crate::paint_backend::drain_into(gutter, layout.screen, None, None, vertices);
    }
    if options.reshape_ahead_width.is_some() && runtime.apply_active_reshape_results() {
        runtime.request_redraw();
    }

    paint_text_editor(runtime, tab_id, resources, editor_rect, layout, style, options, vertices);
    if matches!(options.caret_reveal, TextCaretReveal::SourceLine)
        && runtime.reveal_active_caret_after_shape()
    {
        runtime.request_redraw();
    }
    if let Some(width) = options.reshape_ahead_width {
        runtime.submit_active_reshape_ahead(width);
    }
}

fn paint_text_editor(
    runtime: &mut EditorRuntime,
    tab_id: appkit_core::tab_set::types::TabId,
    resources: &mut RenderResources,
    editor_rect: ui::Rect,
    layout: TextSurfaceLayout,
    style: &EditorPaintStyle<'_>,
    options: &EditorPaintOptions<'_>,
    vertices: &mut Vec<GlyphVertex>,
) {
    let (preedit_text, _) = runtime.preedit();
    let preedit_text = if options.search_caret_opacity.is_some() { "" } else { &preedit_text };
    let line_height = style.metrics.line_height;
    let (visible_rows, viewport_height) = editor_viewport_dimensions(editor_rect.h, line_height);
    let Some(mut tab) = runtime.tab_session_mut(tab_id) else {
        return;
    };
    tab.resize_and_refresh_presentation(visible_rows, viewport_height, line_height);
    let Some(shaped) = shape_text_surface(
        &mut tab,
        resources,
        layout,
        style,
        preedit_text,
        options.preedit_advance,
    ) else {
        return;
    };
    if shaped.tree_dirty {
        tab.derive_scroll_top(line_height);
    }
    let tab_view = TabSession::new(tab.id, tab.document, tab.runtime);
    paint_text_layers(&tab_view, layout, style, options, shaped.vertices, vertices);
    paint_text_caret(
        &tab_view,
        resources,
        layout,
        style,
        options,
        preedit_text,
        shaped.preedit_advance_px,
        vertices,
    );
    let cursor_scroll_changed = matches!(options.caret_reveal, TextCaretReveal::VisualRow)
        && tab.ensure_cursor_visual_row_visible(line_height);
    runtime.plain_text_preedit_advance_px = shaped.preedit_advance_px;
    if cursor_scroll_changed || (shaped.tree_dirty && options.reshape_ahead_width.is_some()) {
        runtime.request_redraw();
    }
}

fn paint_text_layers(
    tab: &TabSession<'_>,
    layout: TextSurfaceLayout,
    style: &EditorPaintStyle<'_>,
    options: &EditorPaintOptions<'_>,
    mut glyphs: Vec<GlyphVertex>,
    vertices: &mut Vec<GlyphVertex>,
) {
    if matches!(options.text_highlight_layer, TextHighlightLayer::AboveGlyphs) {
        vertices.append(&mut glyphs);
    }
    vertices.extend(text_selection_vertices(tab, layout, style));
    vertices.extend(text_search_vertices(tab, layout, style, options.search_highlights));
    vertices.append(&mut glyphs);
}

fn paint_text_caret(
    tab: &TabSession<'_>,
    resources: &mut RenderResources,
    layout: TextSurfaceLayout,
    style: &EditorPaintStyle<'_>,
    options: &EditorPaintOptions<'_>,
    preedit_text: &str,
    preedit_advance_px: f32,
    vertices: &mut Vec<GlyphVertex>,
) {
    let EditorPaintStyle { metrics, theme, .. } = style;
    let screen = layout.screen;
    let cursor_line = tab.cursor_visual_line();
    let cursor_x = tab.cursor_pixel_x();
    let sub_line_offset = tab.sub_line_pixel_offset(metrics.line_height);
    let preedit_origin = plain_text_preedit_origin(
        preedit_text,
        cursor_line,
        cursor_x,
        layout.top,
        metrics.line_height,
        sub_line_offset,
    )
    .or_else(|| options.preedit_fallback_origin.filter(|_| !preedit_text.is_empty()));
    if let Some((preedit_x, preedit_y)) = preedit_origin
        && let (Some(text), Some(gpu)) = (resources.text.as_mut(), resources.gpu.as_ref())
    {
        vertices.extend(crate::render_pipeline::preedit_text_vertices(
            metrics,
            preedit_text,
            preedit_x,
            preedit_y,
            text,
            gpu,
            screen.w,
            screen.h,
            theme.editor.foreground,
        ));
    }
    vertices.extend(ui::decorations::cursor_vertices(
        theme,
        cursor_line,
        layout.top,
        cursor_x + preedit_advance_px,
        tab.cursor_blink_instant(),
        metrics,
        screen.w,
        screen.h,
        sub_line_offset,
        options.search_caret_opacity,
    ));
}

fn shape_text_surface(
    tab: &mut TabSessionMut<'_>,
    resources: &mut RenderResources,
    layout: TextSurfaceLayout,
    style: &EditorPaintStyle<'_>,
    preedit_text: &str,
    preedit_measure: PreeditAdvanceMeasure,
) -> Option<ShapedTextSurface> {
    let (Some(text), Some(gpu)) = (resources.text.as_mut(), resources.gpu.as_ref()) else {
        return None;
    };
    let EditorPaintStyle { theme, settings, metrics } = *style;
    text.shaper.set_font_family(Some(&settings.font_family));
    text.shaper.set_font_size(metrics.font_size);
    let preedit_advance_px =
        preedit_measure.measure(&mut text.shaper, preedit_text, metrics.font_size);
    let context = ui::gutter::RenderContext {
        theme,
        screen_w: layout.screen.w,
        screen_h: layout.screen.h,
        left_margin: layout.left_margin,
        tab_bar_height: layout.top,
        is_active_tab: true,
        gutter_width: layout.gutter_width,
        preedit_advance_px,
        preedit_cursor_col: tab.document.cursor_column(),
    };
    let shaped = tab.with_presentation(|document, presentation| {
        crate::render_pipeline::shape_visible_lines(
            crate::render_pipeline::SourceLayoutOptions {
                metrics,
                text_spacing_mode: settings.text_spacing_mode,
                render_context: &context,
                word_wrap: settings.word_wrap,
            },
            document,
            presentation,
            text,
            gpu,
            &mut resources.frame_cache,
        )
    });
    Some(ShapedTextSurface {
        vertices: shaped.vertices,
        tree_dirty: shaped.tree_dirty,
        preedit_advance_px,
    })
}

fn text_selection_vertices(
    tab: &TabSession<'_>,
    layout: TextSurfaceLayout,
    style: &EditorPaintStyle<'_>,
) -> Vec<GlyphVertex> {
    let TextSurfaceLayout { screen, left_margin, top, .. } = layout;
    let EditorPaintStyle { theme, metrics, .. } = *style;
    let line_offsets = visible_line_byte_offsets(tab);
    ui::decorations::selection_vertices(
        tab.document.selection_range(),
        tab.advance_cache(),
        metrics,
        screen.w,
        screen.h,
        left_margin,
        theme,
        top,
        tab.sub_line_pixel_offset(metrics.line_height),
        &line_offsets,
    )
}

fn text_search_vertices(
    tab: &TabSession<'_>,
    layout: TextSurfaceLayout,
    style: &EditorPaintStyle<'_>,
    highlights: SearchHighlightStyle,
) -> Vec<GlyphVertex> {
    let TextSurfaceLayout { screen, left_margin, top, .. } = layout;
    let EditorPaintStyle { theme, metrics, .. } = *style;
    let search = tab.search_state();
    if matches!(highlights, SearchHighlightStyle::Theme) {
        if !search.is_active() {
            return Vec::new();
        }
        return ui::decorations::search_match_vertices(
            &search.matches.iter().map(|matched| (matched.start, matched.end)).collect::<Vec<_>>(),
            search.active_match_idx,
            true,
            tab.advance_cache(),
            metrics,
            screen.w,
            screen.h,
            left_margin,
            theme,
            top,
            tab.sub_line_pixel_offset(metrics.line_height),
            &visible_line_byte_offsets(tab),
        );
    }
    if !search.panel_visible
        || search.matches.is_empty()
        || search.is_stale(tab.document.tb.gap_buffer().generation())
    {
        return Vec::new();
    }
    let line_offsets = visible_line_byte_offsets(tab);
    let (active_color, inactive_color) = search_overlay_colors(theme);
    search
        .matches
        .iter()
        .enumerate()
        .flat_map(|(index, matched)| {
            let color =
                if index == search.active_match_idx { active_color } else { inactive_color };
            ui::render_geom::compute_selection_highlight_quads(
                tab.advance_cache(),
                (matched.start, matched.end),
                &line_offsets,
                screen.w,
                screen.h,
                metrics.line_height,
                tab.sub_line_pixel_offset(metrics.line_height),
                left_margin,
                color,
                top,
            )
        })
        .collect()
}

fn visible_line_byte_offsets(tab: &TabSession<'_>) -> Vec<usize> {
    let max_doc_line = tab.advance_cache().iter().map(|entry| entry.doc_line).max().unwrap_or(0);
    let mut line_offsets = vec![0usize; max_doc_line + 1];
    for entry in tab.advance_cache() {
        line_offsets[entry.doc_line] = tab.document.line_byte_offset(entry.doc_line).unwrap_or(0);
    }
    line_offsets
}

fn editor_screen(resources: &RenderResources, editor_rect: ui::Rect) -> ui::Screen {
    resources.gpu.as_ref().map_or_else(
        || ui::Screen::new(editor_rect.right().max(1.0), editor_rect.bottom().max(1.0)),
        |gpu| ui::Screen::new(gpu.ctx.config.width as f32, gpu.ctx.config.height as f32),
    )
}

pub(super) fn plugin_bounds(editor_rect: ui::Rect, dpi: f32, is_canvas: bool) -> ui::Rect {
    if is_canvas {
        return editor_rect;
    }
    let horizontal_padding = ui::layout::reading_content_inset(editor_rect.w, dpi);
    let available_padding = (editor_rect.h - 1.0).max(0.0);
    let top_padding = (PLUGIN_CONTENT_TOP_PADDING_LOGICAL * dpi).min(available_padding * 0.5);
    let bottom_padding =
        (PLUGIN_CONTENT_BOTTOM_PADDING_LOGICAL * dpi).min(available_padding - top_padding);
    ui::Rect::new(
        editor_rect.x + horizontal_padding,
        editor_rect.y + top_padding,
        (editor_rect.w - horizontal_padding * 2.0).max(0.0),
        (editor_rect.h - top_padding - bottom_padding).max(0.0),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        EditorPaintStyle, SearchHighlightStyle, TextSurfaceLayout, editor_viewport_dimensions,
        plain_text_preedit_origin, plugin_cursor_visibility_scroll_delta,
    };

    #[test]
    fn plain_text_search_highlights_all_matches_and_closing_removes_them() {
        let mut buffer =
            textora_core::buffer::TextBuffer::new(false).expect("fixture creates buffer");
        buffer.write_raw("中文 中文".as_bytes());
        let document = appkit_core::document::DocumentModel::new(buffer);
        let mut runtime = crate::tab_runtime::TabRuntime::new(Box::new(
            crate::editor_plugin::EditorPlugin::new(),
        ));
        runtime.presentation.display.advance_cache = vec![ui::render_geom::AdvanceCacheEntry {
            doc_line: 0,
            vl_byte_start: 0,
            vl_grapheme_start: 0,
            clusters: vec![(3, 50.0, 0), (6, 60.0, 1), (7, 65.0, 2), (10, 75.0, 3), (13, 85.0, 4)],
        }];
        runtime.presentation.search_state.query = "中文".to_owned();
        runtime.presentation.search_state.panel_visible = true;
        runtime
            .presentation
            .search_state
            .update_matches(vec![0..6, 7..13], document.tb.gap_buffer().generation());
        let mut ids = appkit_core::tab_set::types::TabIdAllocator::new();
        let tab_id = ids.allocate();
        let theme = ui::Theme::resolve_builtin(ui::ThemeMode::Dark, winit::window::Theme::Dark);
        let settings = ui::Settings::new();
        let metrics = ui::UiMetrics::from_settings(&settings, 1.0);
        let style = EditorPaintStyle { theme: &theme, settings: &settings, metrics: &metrics };
        let layout = TextSurfaceLayout {
            screen: ui::Screen::new(300.0, 200.0),
            left_margin: 40.0,
            top: 50.0,
            gutter_width: 0.0,
        };
        let translucent_style = SearchHighlightStyle::TranslucentWhilePanelVisible;
        let tab = crate::tab_session::TabSession::new(tab_id, &document, &runtime);
        let vertices = super::text_search_vertices(&tab, layout, &style, translucent_style);
        assert_eq!(vertices.len(), 12, "both source matches must produce highlight quads");
        assert_eq!(vertices[0].color[..3], theme.palette.highlight[..3]);
        assert_eq!(vertices[6].color[..3], theme.palette.inactive_highlight[..3]);
        assert!(vertices[0].color[3] < 1.0, "search overlay must leave matched glyphs readable");
        runtime.presentation.search_state.panel_visible = false;
        let tab = crate::tab_session::TabSession::new(tab_id, &document, &runtime);
        assert!(super::text_search_vertices(&tab, layout, &style, translucent_style).is_empty());
        let full_color =
            super::text_search_vertices(&tab, layout, &style, SearchHighlightStyle::Theme);
        assert_eq!(full_color.len(), 12, "an active query retains its theme-colored highlights");
        assert_eq!(full_color[0].color, theme.palette.highlight);
        assert_eq!(full_color[6].color, theme.palette.inactive_highlight);
    }

    #[test]
    fn reading_bounds_center_wide_documents_and_preserve_canvas_extent() {
        let editor = ui::Rect::new(200.0, 120.0, 1200.0, 600.0);
        let content = super::plugin_bounds(editor, 1.0, false);
        assert_eq!(content.w, 760.0);
        assert_eq!(content.x - editor.x, editor.right() - content.right());
        assert_eq!(super::plugin_bounds(editor, 1.0, true), editor);
        let scaled = super::plugin_bounds(ui::Rect::new(400.0, 240.0, 2400.0, 1200.0), 2.0, false);
        assert_eq!(scaled.x, content.x * 2.0);
        assert_eq!(scaled.w, content.w * 2.0);
    }

    #[test]
    fn reading_bounds_stay_inside_a_tiny_editor() {
        let editor = ui::Rect::new(20.0, 30.0, 12.0, 10.0);
        let content = super::plugin_bounds(editor, 2.0, false);
        assert!(content.x >= editor.x && content.y >= editor.y);
        assert!(content.right() <= editor.right() && content.bottom() <= editor.bottom());
        assert!(content.w > 0.0 && content.h > 0.0);
    }

    #[test]
    fn partial_bottom_row_is_not_counted_as_fully_visible() {
        const LINE_HEIGHT_PX: f32 = 20.0;
        const EDITOR_HEIGHT_PX: f32 = LINE_HEIGHT_PX * 10.5;

        let (visible_rows, viewport_height) =
            editor_viewport_dimensions(EDITOR_HEIGHT_PX, LINE_HEIGHT_PX);

        assert_eq!(visible_rows, 10);
        assert_eq!(viewport_height, 10.5);
    }

    #[test]
    fn plugin_cursor_below_the_short_viewport_requests_minimal_scroll() {
        const VIEWPORT_HEIGHT_PX: f32 = 90.0;
        const CURSOR_TOP_PX: f32 = 82.0;
        const CURSOR_HEIGHT_PX: f32 = 30.0;

        assert_eq!(
            plugin_cursor_visibility_scroll_delta(
                CURSOR_TOP_PX,
                CURSOR_HEIGHT_PX,
                VIEWPORT_HEIGHT_PX,
            ),
            Some(22.0),
        );
    }

    #[test]
    fn visible_plugin_cursor_does_not_request_scroll() {
        assert_eq!(plugin_cursor_visibility_scroll_delta(24.0, 30.0, 90.0), None);
    }

    #[test]
    fn plugin_cursor_above_the_viewport_requests_minimal_scroll() {
        assert_eq!(plugin_cursor_visibility_scroll_delta(-12.0, 30.0, 90.0), Some(-12.0));
    }

    #[test]
    fn non_empty_plain_text_preedit_starts_at_the_painted_document_caret() {
        const EDITOR_TOP_PX: f32 = 120.0;
        const CURSOR_X_PX: f32 = 248.0;
        const LINE_HEIGHT_PX: f32 = 24.0;
        const SUB_LINE_OFFSET_PX: f32 = -6.0;

        let origin = plain_text_preedit_origin(
            "拼音",
            Some(3),
            CURSOR_X_PX,
            EDITOR_TOP_PX,
            LINE_HEIGHT_PX,
            SUB_LINE_OFFSET_PX,
        )
        .expect("active preedit on a visible caret must produce a paint origin");

        assert_eq!(origin, (CURSOR_X_PX, 186.0));
        assert_eq!(
            plain_text_preedit_origin(
                "",
                Some(3),
                CURSOR_X_PX,
                EDITOR_TOP_PX,
                LINE_HEIGHT_PX,
                SUB_LINE_OFFSET_PX,
            ),
            None
        );
    }
}
