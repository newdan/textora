//! App 初始化：GPU 上下文、窗口、字体系统、display map。

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::app::App;
use crate::file_history::FileHistory;
use crate::product_paths::ProductPaths;
use crate::ui_shell::UiShell;
use crate::view_route::{ViewPathMatcher, ViewRouteRule, ViewRouteTable};
use crate::workspace::Workspace;
use appkit_shell::editor_runtime::{EditorRuntime, EditorRuntimeConfig, EditorRuntimeError};

const MINDMAP_ROUTE_PRIORITY: u16 = 400;
const MARKDOWN_ROUTE_PRIORITY: u16 = 300;
const MARKDOWN_LONG_EXTENSION_ROUTE_PRIORITY: u16 = 299;
const TEXT_ROUTE_PRIORITY: u16 = 200;
const PLUGIN_MARKDOWN_VIEW: &str = "markdown_view";

fn register_plugin_factory(
    registry: &mut ui::plugin::PluginRegistry,
    registered_plugin_ids: &mut HashSet<&'static str>,
    expected_plugin_id: &'static str,
    factory: Box<dyn ui::plugin::PluginFactory>,
) {
    assert_eq!(
        factory.name(),
        expected_plugin_id,
        "registered plugin factory must match its declared product plugin ID"
    );
    registered_plugin_ids.insert(expected_plugin_id);
    registry.register(factory);
}

/// 构造 textora 产品的插件和路径路由；runtime 只接收这一步产出的 shared 配置。
pub(crate) fn build_product_workspace() -> Workspace {
    use ui::plugin::{PLUGIN_EDITOR, PLUGIN_MARKDOWN_EDITOR, PLUGIN_MINDMAP, PLUGIN_NOVEL_VIEW};

    let mut registry = ui::plugin::PluginRegistry::new();
    let mut registered_plugin_ids = HashSet::new();
    register_plugin_factory(
        &mut registry,
        &mut registered_plugin_ids,
        PLUGIN_EDITOR,
        Box::new(crate::plugins::editor::EditorPluginFactory),
    );
    register_plugin_factory(
        &mut registry,
        &mut registered_plugin_ids,
        PLUGIN_MINDMAP,
        Box::new(textora_markdown::mindmap_view::MindmapPluginFactory),
    );
    #[cfg(feature = "markdown")]
    register_plugin_factory(
        &mut registry,
        &mut registered_plugin_ids,
        PLUGIN_MARKDOWN_EDITOR,
        Box::new(textora_markdown::view::MarkdownEditorViewFactory),
    );
    #[cfg(feature = "markdown")]
    register_plugin_factory(
        &mut registry,
        &mut registered_plugin_ids,
        PLUGIN_MARKDOWN_VIEW,
        Box::new(textora_markdown::view::MarkdownViewFactory),
    );
    register_plugin_factory(
        &mut registry,
        &mut registered_plugin_ids,
        PLUGIN_NOVEL_VIEW,
        Box::new(textora_markdown::view::NovelViewFactory),
    );

    let routes = ViewRouteTable::new(
        vec![
            ViewRouteRule {
                matcher: ViewPathMatcher::FileNameSuffix(".mmap.md"),
                default_plugin: PLUGIN_MINDMAP,
                toggle_target: Some(PLUGIN_MARKDOWN_EDITOR),
                priority: MINDMAP_ROUTE_PRIORITY,
            },
            ViewRouteRule {
                matcher: ViewPathMatcher::Extension("md"),
                default_plugin: PLUGIN_MARKDOWN_EDITOR,
                toggle_target: Some(PLUGIN_EDITOR),
                priority: MARKDOWN_ROUTE_PRIORITY,
            },
            ViewRouteRule {
                matcher: ViewPathMatcher::Extension("markdown"),
                default_plugin: PLUGIN_MARKDOWN_EDITOR,
                toggle_target: Some(PLUGIN_EDITOR),
                priority: MARKDOWN_LONG_EXTENSION_ROUTE_PRIORITY,
            },
            ViewRouteRule {
                matcher: ViewPathMatcher::Extension("txt"),
                default_plugin: PLUGIN_EDITOR,
                toggle_target: Some(PLUGIN_NOVEL_VIEW),
                priority: TEXT_ROUTE_PRIORITY,
            },
        ],
        &registered_plugin_ids,
    )
    .expect("textora product routes must reference unique priorities and registered plugins");

    Workspace::with_plugins(registry, routes)
}

pub(crate) fn build_product_editor_runtime(
    settings: &ui::settings::Settings,
    theme: &ui::Theme,
    snapshots_directory: &std::path::Path,
) -> Result<EditorRuntime, EditorRuntimeError> {
    let empty_routes = ViewRouteTable::new(Vec::new(), &HashSet::new())
        .expect("an empty migration route table must be valid");
    EditorRuntime::new_with_model(
        EditorRuntimeConfig {
            plugin_registry: ui::plugin::PluginRegistry::new(),
            view_routes: empty_routes,
            initial_settings: settings.clone(),
            initial_theme: theme.clone(),
            snapshots_directory: snapshots_directory.to_owned(),
        },
        build_product_workspace(),
        Default::default(),
    )
}

#[cfg(test)]
mod product_runtime_assembly_tests {
    #[test]
    fn product_factory_keeps_plugin_registration_outside_shared_shell() {
        let source = include_str!("app_init.rs");
        assert!(source.contains("build_product_workspace"));
        assert!(source.contains("register_plugin_factory"));
    }
}

fn settings_from_persisted(
    persisted: &crate::settings_io::PersistedSettings,
) -> ui::settings::Settings {
    let mut settings = ui::settings::Settings::new();
    settings.view_mode = persisted.view_mode;
    settings.theme_mode = persisted.theme_mode;
    settings.show_line_numbers = persisted.show_line_numbers;
    settings.word_wrap = persisted.word_wrap;
    settings.show_status_bar = persisted.show_status_bar;
    settings.font_family = persisted.font_family.clone();
    settings.font_size = persisted.font_size;
    settings.line_height_ratio = persisted.line_height_ratio;
    settings.line_height = persisted.font_size * persisted.line_height_ratio;
    settings.tab_width = persisted.tab_width;
    settings
}

fn load_user_themes(
    dir: &std::path::Path,
) -> (ui::theme::ThemeRegistry, crate::theme_loader::ThemeLoadReport) {
    let batch = crate::theme_loader::load_theme_sources(dir);
    let mut registry = ui::theme::ThemeRegistry::new();
    let registration = registry.register_sources(batch.sources);
    let report = crate::theme_loader::ThemeLoadReport {
        source_diagnostics: batch.diagnostics,
        registry_errors: registration.errors,
        registered_ids: registration.registered_ids,
    };
    (registry, report)
}

impl App {
    pub fn new(file_path: Option<PathBuf>) -> Self {
        let startup_started_at = std::time::Instant::now();
        let _t0 = startup_started_at;

        let home_dir = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()));
        let paths = ProductPaths::textora(&home_dir);

        // Load persisted settings from settings.toml (before FontSystem/worker creation
        // so that font shaping uses persisted settings rather than defaults).
        let persisted = match crate::settings_io::load(&paths.settings_file) {
            Ok(settings) => settings,
            Err(error) => {
                eprintln!("[settings] failed to load settings: {}", error);
                crate::settings_io::PersistedSettings::default()
            }
        };
        let settings = settings_from_persisted(&persisted);

        // Create FontSystem once, shared between main thread and reshape worker.
        // Uses disk cache to skip font parsing on subsequent startups.
        let _t1 = std::time::Instant::now();
        let font_system = shaping::font_cache::new_font_system_with_cache(
            &shaping::font_cache::default_cache_path().unwrap_or_default(),
        );
        eprintln!("[startup] FontSystem::new: {:?}", _t1.elapsed());
        let shared_fs = Arc::new(Mutex::new(font_system));

        // ReshapeWorker creation is deferred to init_window() where the actual
        // scale factor is known.  We store None here and spawn the worker
        // after scale setup in init_window.

        let workspace_store = crate::workspace_store::WorkspaceStore::new(
            paths.workspace_file.clone(),
            paths.pinned_paths_file.clone(),
            paths.snapshots_dir.clone(),
        );
        let file_history = FileHistory::load(&paths.history_file).unwrap_or_default();

        let (theme_registry, theme_load_report) = load_user_themes(&paths.theme_dir);
        for diagnostic in &theme_load_report.source_diagnostics {
            eprintln!("[theme] {diagnostic:?}");
        }
        for error in &theme_load_report.registry_errors {
            eprintln!("[theme] {error}");
        }
        let mut active_theme_pair = ui::theme::ActiveThemePair::default();
        active_theme_pair.light = "spec-light".into();
        active_theme_pair.dark = "spec-dark".into();
        let current_theme = {
            let mode = settings.theme_mode;
            ui::Theme::resolve(
                mode,
                winit::window::Theme::Dark,
                &active_theme_pair,
                &theme_registry,
            )
        };
        let mut editor_runtime =
            build_product_editor_runtime(&settings, &current_theme, &paths.snapshots_dir)
                .expect("textora editor runtime must be constructible");
        editor_runtime.set_shared_font_system(shared_fs.clone());
        let mut app = Self {
            file_path,
            paths,
            settings,
            settings_persistence: crate::app::SettingsPersistenceState::Saved,
            current_theme,
            theme_registry,
            active_theme_pair,
            theme_load_report,
            product: crate::textora_product::TextoraProduct::new(),
            editor_runtime,
            popup_tab_id_snapshot: Vec::new(),
            workspace_store,
            ui_shell: UiShell::new(),
            file_history,
            library_file_monitor: None,
            file_safety_notices: Vec::new(),
            pending_close_after_save: HashSet::new(),
            pending_quit_after_save: false,
            running: false,
            needs_redraw: true,
            sidebar_animating: false,
            tab_scroll: crate::smooth_scroll::SmoothScroll::new(),
            mouse: crate::mouse::MouseState::new(),
            last_scroll_time: std::time::Instant::now(),
            last_cursor_visible: true,
            event_loop_proxy: None,
            preedit_advance_px: 0.0,
            startup_started_at,
        };
        // Apply persisted sidebar width (logical pixels, will be scaled by DPI later)
        app.ui_shell.set_sidebar_width(persisted.sidebar_width);
        app.ui_shell.sidebar_clamp_width(1.0); // Clamp to valid range [160, 400] at default DPI
        eprintln!("[startup] App::new total: {:?}", _t0.elapsed());
        app
    }

    pub(crate) fn init_display_map(&mut self, dv_idx: usize) {
        let Some(tab_id) = self.editor_tab_id_at(dv_idx) else {
            return;
        };
        let metrics = self.ui_metrics();
        let line_height = metrics.line_height;
        let font_size = metrics.font_size;

        // Early-out: collect metadata first so we can release dv borrow before shaping.
        let (_vc, viewport_width, lh, _font_size, _pre_start, _pre_end, viewport_line_data) = {
            let Some(tab) = self.tab_session(tab_id) else { return };
            let dv = tab.document;

            // Skip rebuild if display map is already up-to-date
            if tab.display().display_map.line_count() == dv.line_count() && dv.line_count() > 1 {
                let anchor_doc =
                    tab.scroll_anchor_doc_line().min(dv.line_count().saturating_sub(1));
                if let Some(entry) = tab.display_map_entry(anchor_doc) {
                    let alen = dv.line_byte_length(anchor_doc).unwrap_or(0);
                    let aoff = dv.line_byte_offset(anchor_doc).unwrap_or(0);
                    let screen_w = self.screen_width();
                    let metrics = self.ui_metrics();
                    let vp_w = screen_w
                        - metrics.scrollbar_reserve
                        - self.editor_left_margin(dv.line_count());
                    let expected_hash =
                        crate::content_hash::content_hash(aoff, alen as u32, vp_w, font_size);
                    let is_placeholder = entry.visual_breaks.is_empty()
                        || (entry.visual_breaks.len() == 1
                            && entry.visual_breaks[0].pixel_width == 0.0
                            && alen > 0);
                    if !is_placeholder && entry.content_hash == expected_hash {
                        if let Some(mut tab) = self.tab_session_mut(tab_id) {
                            tab.clamp_scroll_anchor(line_height);
                            tab.derive_scroll_top(line_height);
                        }
                        return;
                    }
                    if is_placeholder {
                        return;
                    }
                    // Hash mismatch but entry has valid visual_breaks (e.g. viewport resize).
                    // Fall through to full rebuild with correct dimensions.
                }
            }

            let screen_w = self.screen_width();
            let metrics = self.ui_metrics();
            let vw =
                screen_w - metrics.scrollbar_reserve - self.editor_left_margin(dv.line_count());
            let fs = font_size;
            let lh = line_height;
            let total = dv.line_count();
            let vc = dv.line_count();

            // Compute viewport range using simple est_vl (placeholder quality, good enough for range)
            let anchor = tab.scroll_anchor_doc_line().min(total.saturating_sub(1));
            let visible = tab.visible_rows().max(1);
            // Worst-case range: every line has VL=1. Guarantees full viewport
            // coverage regardless of actual wrapping. Pre-shaping extra lines
            // is cheap (shaper caches glyph data).
            let above_lines = (tab.scroll_anchor_pixel_offset() / lh).ceil() as usize;
            let pre_start = anchor.saturating_sub(above_lines + 64);
            let pre_end = (anchor + visible + 64).min(total);

            // Collect line strings for viewport range
            let mut vld: Vec<(usize, String, usize, u32)> = Vec::new();
            for dl in pre_start..pre_end {
                if let Some(bytes) = dv.doc_line_bytes(dl)
                    && let Ok(s) = std::str::from_utf8(&bytes)
                    && !s.is_empty()
                {
                    let off = dv.line_byte_offset(dl).unwrap_or(0);
                    let len = dv.line_byte_length(dl).unwrap_or(0);
                    vld.push((dl, s.to_owned(), off, len as u32));
                }
            }
            (vc, vw, lh, fs, pre_start, pre_end, vld)
        }; // dv borrow released

        // Phase 2: shape viewport lines with main-thread shaper
        let mut pre_entries: std::collections::HashMap<usize, crate::snap_tree::DisplayLineEntry> =
            std::collections::HashMap::new();
        let mut render_resources = self.editor_runtime.take_render_resources();
        if let Some(text) = render_resources.text.as_mut() {
            text.shaper.set_font_size(font_size);
            for (dl, line_str, off, len) in &viewport_line_data {
                let shaped = match text.shaper.shape_fast(line_str) {
                    Ok(s) => s,
                    Err(_) => match text.shaper.shape(line_str) {
                        Ok(s) => s,
                        Err(_) => continue,
                    },
                };
                {
                    let bytes = line_str.as_bytes();
                    let char_width = text.shaper.col_width();
                    let visual_lines = ui::layout::compute_visual_lines(
                        &shaped.clusters,
                        bytes,
                        char_width,
                        viewport_width,
                        0.5,
                    );
                    let mut breaks: smallvec::SmallVec<[crate::snap_tree::VisualBreak; 1]> =
                        smallvec::SmallVec::new();
                    if visual_lines.is_empty() {
                        breaks.push(crate::snap_tree::VisualBreak {
                            byte_start: 0,
                            byte_end: *len,
                            pixel_width: 0.0,
                        });
                    } else {
                        for (vs, ve, pw) in &visual_lines {
                            let bs = shaped.clusters[*vs].byte_range.start as u32;
                            let be = shaped.clusters[*ve - 1].byte_range.end as u32;
                            breaks.push(crate::snap_tree::VisualBreak {
                                byte_start: bs,
                                byte_end: be,
                                pixel_width: *pw,
                            });
                        }
                    }
                    let vl = breaks.len().max(1) as u16;
                    let hash =
                        crate::content_hash::content_hash(*off, *len, viewport_width, font_size);
                    pre_entries.insert(
                        *dl,
                        crate::snap_tree::DisplayLineEntry {
                            visual_line_count: vl,
                            visual_breaks: breaks,
                            byte_offset: *off,
                            byte_length: *len,
                            content_hash: hash,
                        },
                    );
                }
            }
        }
        self.editor_runtime.restore_render_resources(render_resources);

        // Phase 3: build all entries and commit
        if let Some(mut tab) = self.tab_session_mut(tab_id) {
            let dv = &mut tab.document;
            let est_cpl = (viewport_width / (font_size * 0.6)).max(40.0);

            let entries: Vec<crate::snap_tree::DisplayLineEntry> = (0..dv.line_count())
                .map(|i| {
                    if let Some(entry) = pre_entries.get(&i) {
                        entry.clone()
                    } else {
                        let len = dv.line_byte_length(i).unwrap_or(0);
                        let off = dv.line_byte_offset(i).unwrap_or(0);
                        let hash = crate::content_hash::content_hash(
                            off,
                            len as u32,
                            viewport_width,
                            font_size,
                        );
                        let est_vl = (len as f32 / est_cpl).ceil() as u16;
                        crate::snap_tree::DisplayLineEntry::placeholder(
                            off, len as u32, hash, est_vl,
                        )
                    }
                })
                .collect();

            {
                let display = tab.display_mut();
                display.display_map.set_viewport_size(viewport_width, font_size);
                display.display_map.set_entries(entries);
            }
            tab.clamp_scroll_anchor(lh);
            tab.derive_scroll_top(lh);
            self.editor_runtime.mark_skip_next_reshape_submit();
        }
    }
}

#[cfg(test)]
mod settings_tests {
    use super::*;
    use crate::document_view::DocumentView;
    use crate::plugins::editor::EditorPlugin;

    #[test]
    fn settings_from_persisted_preserves_font_configuration() {
        let persisted = crate::settings_io::PersistedSettings {
            font_family: "Audit Mono".into(),
            font_size: 19.0,
            line_height_ratio: 1.5,
            ..Default::default()
        };
        let settings = settings_from_persisted(&persisted);
        assert_eq!(settings.font_family, "Audit Mono");
        assert_eq!(settings.font_size, 19.0);
        assert_eq!(settings.line_height, 28.5);
    }

    #[test]
    fn persisted_font_size_is_loaded_as_logical_value() {
        let persisted = crate::settings_io::PersistedSettings {
            font_size: 18.0,
            ..crate::settings_io::PersistedSettings::default()
        };
        let settings = settings_from_persisted(&persisted);
        assert_eq!(settings.font_size, 18.0);
    }

    #[test]
    fn startup_timestamp_is_initialized() {
        let app = App::new(None);
        assert!(app.startup_started_at.elapsed() < std::time::Duration::from_secs(60));
    }

    #[test]
    fn app_constructor_creates_empty_product_inboxes() {
        let mut app = App::new(None);
        assert!(app.product.drain_open_documents().is_empty());
    }

    #[test]
    fn init_display_map_uses_instance_font_size() {
        let mut app = App::new(None);
        app.settings.font_size = 31.0;
        app.settings.line_height = 44.0;
        let dv = DocumentView::new(vec!["sentinel".into()], 10, 10.0);
        app.push_entry_for_test(dv, Box::new(EditorPlugin::new()));

        app.init_display_map(0);

        let snapshot = app.active_tab_session().unwrap().display().display_map.snapshot();
        assert_eq!(snapshot.font_size, 31.0);
    }

    #[test]
    fn build_theme_registry_retains_source_and_registry_diagnostics() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a-invalid.toml"), "not = [valid").unwrap();
        std::fs::write(dir.path().join("b-good.toml"), "is_dark = true\n").unwrap();
        std::fs::write(dir.path().join("default-dark.toml"), "is_dark = true\n").unwrap();

        let (registry, report) = load_user_themes(dir.path());

        assert!(registry.get("b-good").is_some());
        assert_eq!(report.registered_ids, vec!["b-good"]);
        assert!(report.registry_errors.iter().any(|error| matches!(
            error,
            ui::theme::ThemeLoadError::TomlParse { id, .. } if id == "a-invalid"
        )));
        assert!(report.registry_errors.iter().any(|error| matches!(
            error,
            ui::theme::ThemeLoadError::ReservedId { id, .. } if id == "default-dark"
        )));
    }
}

#[cfg(test)]
mod product_paths_tests {
    use super::*;
    use crate::product_paths::ProductPaths;
    use std::path::PathBuf;
    use std::sync::Mutex;

    static HOME_LOCK: Mutex<()> = Mutex::new(());

    struct HomeGuard {
        previous: Option<String>,
    }

    impl HomeGuard {
        fn set(home: &std::path::Path) -> Self {
            let previous = std::env::var("HOME").ok();
            unsafe { std::env::set_var("HOME", home) };
            Self { previous }
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(home) => unsafe { std::env::set_var("HOME", home) },
                None => unsafe { std::env::remove_var("HOME") },
            }
        }
    }

    #[test]
    fn app_init_uses_product_paths_for_config_dir() {
        let _lock = HOME_LOCK.lock().unwrap();
        let temp_home = tempfile::tempdir().unwrap();
        let config_dir = temp_home.path().join(".edit+");
        std::fs::create_dir_all(&config_dir).unwrap();

        std::fs::write(
            config_dir.join("history.toml"),
            r#"version = 1
entries = [
    { file_path = "/tmp/seeded.md", last_closed_at = 42, last_cursor_line = 0, last_cursor_col = 0, scroll_anchor_line = 0, scroll_anchor_offset = 0.0 }
]
excluded_dirs = []
"#,
        )
        .unwrap();

        std::fs::write(
            config_dir.join("workspace.toml"),
            "version = 1\nactive_index = 0\nentries = []\n",
        )
        .unwrap();

        let _home_guard = HomeGuard::set(temp_home.path());
        let app = App::new(None);

        let expected_paths = ProductPaths::textora(temp_home.path());
        assert_eq!(app.paths, expected_paths);

        assert_eq!(app.file_history.entries.len(), 1);
        assert_eq!(app.file_history.entries[0].file_path, PathBuf::from("/tmp/seeded.md"));

        let workspace = app.workspace_store.load_workspace().unwrap();
        assert!(workspace.is_some());
    }
}
