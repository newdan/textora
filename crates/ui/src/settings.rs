//! User-configurable settings for the editor.

use crate::view_mode::ViewMode;

/// Theme mode: follow system, force dark, or force light.
/// ClaudeDark and ClaudeLight legacy values deserialize as Dark/Light via aliases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ThemeMode {
    #[default]
    System,
    #[serde(alias = "ClaudeDark")]
    Dark,
    #[serde(alias = "ClaudeLight")]
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiMetrics {
    pub dpi: f32,
    pub font_size: f32,
    pub line_height: f32,
    pub status_bar_height: f32,
    pub gutter_padding: f32,
    pub toc_width: f32,
    pub content_left_margin: f32,
    pub scrollbar_reserve: f32,
    pub show_line_numbers: bool,
    pub show_status_bar: bool,
}

impl UiMetrics {
    /// Normalize DPI value: non-positive, NaN, or infinite values fall back to 1.0.
    fn normalize_dpi(dpi: f32) -> f32 {
        if dpi.is_finite() && dpi > 0.0 { dpi } else { 1.0 }
    }

    /// Pure derivation: scale logical settings by the given DPI factor.
    /// All dimensional values in settings are treated as logical (pre-scale)
    /// and multiplied by dpi once.
    pub fn from_settings(settings: &Settings, dpi: f32) -> Self {
        let dpi = Self::normalize_dpi(dpi);
        Self {
            dpi,
            font_size: settings.font_size * dpi,
            line_height: settings.line_height * dpi,
            status_bar_height: settings.status_bar_height * dpi,
            gutter_padding: settings.gutter_padding * dpi,
            toc_width: settings.toc_width * dpi,
            content_left_margin: 32.0 * dpi,
            scrollbar_reserve: crate::widgets::scrollbar::SCROLLBAR_RESERVE_PX * dpi,
            show_line_numbers: settings.show_line_numbers,
            show_status_bar: settings.show_status_bar,
        }
    }
}

/// User-configurable settings.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Font family name (e.g., "Menlo", "Helvetica").
    pub font_family: String,
    /// UI Font family name (e.g., "-apple-system", "Segoe UI").
    pub ui_font_family: String,
    /// Font size in pixels.
    pub font_size: f32,
    /// Line height in pixels (typically font_size * 1.4).
    pub line_height: f32,
    /// Tab width in spaces.
    pub tab_width: usize,
    /// Whether word wrap is enabled.
    pub word_wrap: bool,
    /// Status bar height in pixels.
    pub status_bar_height: f32,
    /// Whether to show line numbers in the gutter.
    pub show_line_numbers: bool,
    /// Whether to show the status bar.
    pub show_status_bar: bool,
    /// Line number text color (RGBA).
    pub line_number_color: [f32; 4],
    /// Gap between gutter area and text content (pixels).
    pub gutter_padding: f32,
    /// Version counter — incremented on each change for cache invalidation.
    pub version: u64,
    /// Maximum bytes per line to shape for async worker (5000 = cap at 5KB).
    /// Limits shaping cost for extremely long lines; subset shaping handles the rest.
    pub max_line_bytes_for_shaping: usize,
    /// Top-level view mode: Sidebar or Tabs.
    pub view_mode: ViewMode,
    /// Theme mode override (System follows winit theme).
    pub theme_mode: ThemeMode,
    /// Line height multiplier relative to font_size.
    pub line_height_ratio: f32,
    /// Minimum width ratio for punctuation glyphs relative to font_size/em.
    /// 0.5 means punctuation takes at least half an em width; 0.0 disables.
    pub min_punctuation_width_ratio: f32,
    /// Maximum heading depth shown in the TOC panel (1-6, default 3).
    pub toc_max_depth: u8,
    /// TOC panel width in logical pixels (default 200).
    pub toc_width: f32,
    /// Whether to enable novel reading mode for .txt files.
    pub enable_novel_mode: bool,
}

/// Platform-dependent default monospace font family.
pub fn platform_default_font_family() -> &'static str {
    if cfg!(target_os = "macos") {
        "Menlo"
    } else if cfg!(target_os = "windows") {
        "Consolas"
    } else {
        "Droid Sans Mono"
    }
}

/// Platform-dependent default UI sans-serif font family.
pub fn platform_default_ui_font_family() -> &'static str {
    if cfg!(target_os = "macos") {
        "-apple-system"
    } else if cfg!(target_os = "windows") {
        "Segoe UI"
    } else {
        "system-ui"
    }
}

impl Settings {
    /// Create new settings with sensible defaults.
    pub fn new() -> Self {
        Self {
            font_family: platform_default_font_family().to_string(),
            ui_font_family: platform_default_ui_font_family().to_string(),
            font_size: 15.0,
            line_height: 15.0 * 1.618, // font_size * line_height_ratio
            tab_width: 4,
            word_wrap: true,
            status_bar_height: 20.0,
            max_line_bytes_for_shaping: 5000,
            show_line_numbers: true,
            show_status_bar: false,
            line_number_color: [0.4, 0.4, 0.45, 1.0],
            gutter_padding: 8.0,
            version: 1,
            view_mode: ViewMode::default(),
            theme_mode: ThemeMode::default(),
            line_height_ratio: 1.618,
            min_punctuation_width_ratio: 0.5,
            toc_max_depth: 3,
            toc_width: 200.0,
            enable_novel_mode: true,
        }
    }

    /// Update font size and recalculate line height.
    pub fn set_font_size(&mut self, size: f32) {
        self.font_size = size;
        self.line_height = size * self.line_height_ratio;
        self.version += 1;
    }

    /// Update line height directly.
    pub fn set_line_height(&mut self, height: f32) {
        self.line_height = height;
        self.version += 1;
    }

    /// Update line height ratio and recalculate line_height.
    pub fn set_line_height_ratio(&mut self, ratio: f32) {
        self.line_height_ratio = ratio;
        self.line_height = self.font_size * ratio;
        self.version += 1;
    }

    /// Update font family.
    pub fn set_font_family(&mut self, family: String) {
        self.font_family = family;
        self.version += 1;
    }

    /// Update UI font family.
    pub fn set_ui_font_family(&mut self, family: String) {
        self.ui_font_family = family;
        self.version += 1;
    }

    /// Toggle word wrap.
    pub fn set_word_wrap(&mut self, enabled: bool) {
        self.word_wrap = enabled;
        self.version += 1;
    }

    /// Update Tab indentation width.
    pub fn set_tab_width(&mut self, width: usize) {
        self.tab_width = width;
        self.version += 1;
    }

    /// Update line-number visibility.
    pub fn set_show_line_numbers(&mut self, enabled: bool) {
        self.show_line_numbers = enabled;
        self.version += 1;
    }

    /// Update status-bar visibility.
    pub fn set_show_status_bar(&mut self, enabled: bool) {
        self.show_status_bar = enabled;
        self.version += 1;
    }

    /// Update the top-level view mode.
    pub fn set_view_mode(&mut self, mode: ViewMode) {
        self.view_mode = mode;
        self.version += 1;
    }

    /// Update the theme mode.
    pub fn set_theme_mode(&mut self, mode: ThemeMode) {
        self.theme_mode = mode;
        self.version += 1;
    }

    /// Update minimum punctuation width ratio.
    pub fn set_min_punctuation_width_ratio(&mut self, ratio: f32) {
        self.min_punctuation_width_ratio = ratio;
        self.version += 1;
    }

    /// Calculate exact visible height in lines.
    pub fn visible_height_lines(&self, screen_height: f32, tab_bar_height: f32) -> f64 {
        let status_h = if self.show_status_bar { self.status_bar_height } else { 0.0 };
        ((screen_height - status_h - tab_bar_height) / self.line_height).max(1.0) as f64
    }

    /// Calculate visible rows for rendering loop.
    /// Uses floor to ensure the last visible row does not overflow into the status bar area.
    /// Each visual line occupies a full line_height, so the last partially-visible line
    /// must stay within screen_h - status_bar_height - tab_bar_height.
    pub fn visible_rows(&self, screen_height: f32, tab_bar_height: f32) -> usize {
        self.visible_height_lines(screen_height, tab_bar_height).floor() as usize
    }
    /// Calculate gutter width needed for line numbers.
    /// Returns 0.0 when show_line_numbers is false.
    pub fn gutter_width(&self, line_count: usize) -> f32 {
        if !self.show_line_numbers {
            return 0.0;
        }
        let raw_digits =
            if line_count == 0 { 1 } else { (line_count as f64).log10().floor() as usize + 1 };
        let digits = raw_digits.max(3);
        let digit_width = self.font_size * 0.48;
        digits as f32 * digit_width + self.gutter_padding
    }

    /// Test helper: create default Settings and return &'static reference.
    /// Only for use in tests; intentionally leaks memory.
    #[cfg(test)]
    pub(crate) fn test_default() -> &'static Settings {
        Box::leak(Box::new(Settings::new()))
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self::new()
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn ui_metrics_scale_logical_dimensions_exactly_once() {
        let mut settings = Settings::new();
        settings.font_size = 10.0;
        settings.line_height = 16.0;
        settings.status_bar_height = 20.0;
        settings.gutter_padding = 8.0;
        settings.toc_width = 200.0;

        let metrics = UiMetrics::from_settings(&settings, 2.0);

        assert_eq!(metrics.dpi, 2.0);
        assert_eq!(metrics.font_size, 20.0);
        assert_eq!(metrics.line_height, 32.0);
        assert_eq!(metrics.status_bar_height, 40.0);
        assert_eq!(metrics.gutter_padding, 16.0);
        assert_eq!(metrics.toc_width, 400.0);
        assert_eq!(metrics.content_left_margin, 64.0);
        assert_eq!(
            metrics.scrollbar_reserve,
            crate::widgets::scrollbar::SCROLLBAR_RESERVE_PX * 2.0
        );
    }

    #[test]
    fn ui_metrics_invalid_dpi_falls_back_to_one() {
        let settings = Settings::new();
        for dpi in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let metrics = UiMetrics::from_settings(&settings, dpi);
            assert_eq!(metrics.dpi, 1.0);
            assert_eq!(metrics.font_size, settings.font_size);
            assert_eq!(metrics.line_height, settings.line_height);
        }
    }

    #[test]
    fn ui_metrics_derivation_is_repeatable() {
        let settings = Settings::new();
        assert_eq!(
            UiMetrics::from_settings(&settings, 1.75),
            UiMetrics::from_settings(&settings, 1.75)
        );
    }

    #[test]
    fn ui_metrics() {
        let mut settings = Settings::new();
        settings.show_status_bar = false;
        settings.show_line_numbers = false;
        settings.font_size = 14.0;
        settings.line_height = 20.0;
        settings.status_bar_height = 24.0;
        settings.gutter_padding = 10.0;

        // Explicit destructuring: if word_wrap, theme_mode, or any other
        // behavior field creeps back into UiMetrics, this will fail to compile.
        let UiMetrics {
            dpi,
            font_size,
            line_height,
            status_bar_height,
            gutter_padding,
            toc_width,
            content_left_margin,
            scrollbar_reserve,
            show_line_numbers,
            show_status_bar,
        } = UiMetrics::from_settings(&settings, 2.0);

        assert_eq!(dpi, 2.0);
        assert!(!show_status_bar);
        assert!(!show_line_numbers);
        assert_eq!(font_size, 28.0);
        assert_eq!(line_height, 40.0);
        assert_eq!(status_bar_height, 48.0);
        assert_eq!(gutter_padding, 20.0);
        // Suppress unused warnings for fields not asserted above
        let _ = (toc_width, content_left_margin, scrollbar_reserve);
    }

    use super::*;

    #[test]
    fn default_settings() {
        let s = Settings::default();
        assert_eq!(s.font_family, platform_default_font_family());
        assert_eq!(s.font_size, 15.0);
        assert_eq!(s.line_height, 24.27);
        assert_eq!(s.status_bar_height, 20.0);
        assert_eq!(s.tab_width, 4);
        assert!(s.word_wrap);
        assert_eq!(s.status_bar_height, 20.0);
        assert!(s.show_line_numbers);
        assert_eq!(s.line_number_color, [0.4, 0.4, 0.45, 1.0]);
        assert_eq!(s.gutter_padding, 8.0);
        assert_eq!(s.version, 1);
    }

    #[test]
    fn set_font_size_updates_line_height() {
        let mut s = Settings::new();
        s.set_font_size(20.0);
        assert_eq!(s.font_size, 20.0);
        assert_eq!(s.line_height, 32.36); // 20.0 * 1.618
    }

    #[test]
    fn set_font_size_increments_version() {
        let mut s = Settings::new();
        assert_eq!(s.version, 1);
        s.set_font_size(18.0);
        assert_eq!(s.version, 2);
    }

    #[test]
    fn set_line_height_increments_version() {
        let mut s = Settings::new();
        s.set_line_height(30.0);
        assert_eq!(s.line_height, 30.0);
        assert_eq!(s.version, 2);
    }

    #[test]
    fn set_font_family_increments_version() {
        let mut s = Settings::new();
        s.set_font_family("Helvetica".to_string());
        assert_eq!(s.font_family, "Helvetica");
        assert_eq!(s.version, 2);
    }

    #[test]
    fn set_word_wrap_increments_version() {
        let mut s = Settings::new();
        s.set_word_wrap(false);
        assert!(!s.word_wrap);
        assert_eq!(s.version, 2);
    }

    #[test]
    fn visible_rows_calculation() {
        let mut s = Settings::new(); // line_height = 24.27, status_bar_height = 20.0, tab_bar_height = 32.0
        s.show_status_bar = true;
        assert_eq!(s.visible_rows(800.0, 32.0), 30); // (800 - 20 - 32) / 24.27 = 30.8.floor() = 30
        assert_eq!(s.visible_rows(100.0, 32.0), 1); // (100 - 20 - 32) / 24.27 = 1.9.floor() = 1
        assert_eq!(s.visible_rows(10.0, 32.0), 1); // min 1.0.floor() = 1
        assert_eq!(s.visible_rows(0.0, 32.0), 1); // min 1.0.floor() = 1
    }

    #[test]
    fn visible_rows_uses_line_height() {
        let mut s = Settings::new();
        s.show_status_bar = true;
        s.line_height = 30.0; // don't use set_ to avoid version bump
        assert_eq!(s.visible_rows(900.0, 32.0), 28); // (900-20-32)/30 = 28.26.floor() = 28
    }

    #[test]
    fn set_line_height_ratio_updates_line_height() {
        let mut s = Settings::new();
        s.set_line_height_ratio(2.0);
        assert_eq!(s.line_height_ratio, 2.0);
        assert_eq!(s.line_height, 15.0 * 2.0);
        assert_eq!(s.version, 2);
    }

    #[test]
    fn set_line_height_ratio_uses_current_font_size() {
        let mut s = Settings::new();
        s.set_font_size(20.0); // version 2
        s.set_line_height_ratio(1.5); // version 3
        assert_eq!(s.line_height, 20.0 * 1.5);
    }

    #[test]
    fn multiple_changes_increment_version() {
        let mut s = Settings::new();
        assert_eq!(s.version, 1);
        s.set_font_size(16.0);
        assert_eq!(s.version, 2);
        s.set_font_family("Arial".to_string());
        assert_eq!(s.version, 3);
        s.set_word_wrap(false);
        assert_eq!(s.version, 4);
    }

    #[test]
    fn dpi_derivation_does_not_mutate_settings_version() {
        let settings = Settings::new();
        let version = settings.version;
        let _ = UiMetrics::from_settings(&settings, 2.0);
        assert_eq!(settings.version, version);
    }
}

// ── A1: Verify field is directly accessible (getter is redundant) ──
// ── Gutter width tests ──────────────────────────────────────────────
#[cfg(test)]
mod gutter_tests {
    use super::*;

    #[test]
    fn default_show_line_numbers_true() {
        let s = Settings::new();
        assert!(s.show_line_numbers);
    }

    #[test]
    fn line_number_color_default() {
        let s = Settings::new();
        assert_eq!(s.line_number_color, [0.4, 0.4, 0.45, 1.0]);
    }

    #[test]
    fn gutter_padding_default() {
        let s = Settings::new();
        assert_eq!(s.gutter_padding, 8.0);
    }

    #[test]
    fn gutter_width_single_digit() {
        let s = Settings::new();
        let w = s.gutter_width(9);
        // With min 3 digits: 3 * 7.2 + 8.0 = 29.6
        assert!(w > 0.0);
        assert!((w - 29.6).abs() < 0.5);
    }

    #[test]
    fn gutter_width_two_digits() {
        let s = Settings::new();
        let w = s.gutter_width(99);
        // With min 3 digits: 3 * 7.2 + 8.0 = 29.6
        assert!((w - 29.6).abs() < 0.5);
    }

    #[test]
    fn gutter_width_three_digits() {
        let s = Settings::new();
        let w = s.gutter_width(999);
        // 3 digits * 7.2 + 8.0 = 29.6
        assert!((w - 29.6).abs() < 0.5);
    }

    #[test]
    fn gutter_width_four_digits() {
        let s = Settings::new();
        let w = s.gutter_width(1000);
        // 4 digits * 7.2 + 8.0 = 36.8
        assert!((w - 36.8).abs() < 0.5);
    }

    #[test]
    fn gutter_width_zero_when_disabled() {
        let mut s = Settings::new();
        s.show_line_numbers = false;
        assert_eq!(s.gutter_width(100), 0.0);
    }

    #[test]
    fn gutter_width_nonzero_for_empty_file() {
        let s = Settings::new();
        assert!((s.gutter_width(0) - 29.6).abs() < 0.5); // min 3 digits for empty doc
    }

    #[test]
    fn gutter_width_monotonic() {
        let s = Settings::new();
        assert!((s.gutter_width(10) - s.gutter_width(9)).abs() < 0.5); // both clamped to 3 digits
        assert!((s.gutter_width(100) - s.gutter_width(99)).abs() < 0.5); // both clamped to 3 digits
    }

    #[test]
    fn gutter_width_digit_boundary() {
        let s = Settings::new();
        // 9 (1 digit clamped to 3) vs 10 (2 digits clamped to 3) — same width
        assert!((s.gutter_width(10) - s.gutter_width(9)).abs() < 0.5);
        // 999 (3 digits) vs 1000 (4 digits) — should increase
        assert!(s.gutter_width(1000) > s.gutter_width(999));
    }
}

#[cfg(test)]
mod a1_tests {
    use super::*;

    #[test]
    fn status_bar_height_field_direct_access() {
        let s = Settings::new();
        // Field is pub, so direct access works — getter is redundant
        assert_eq!(s.status_bar_height, 20.0);
    }

    #[test]
    fn status_bar_height_field_modifiable() {
        let mut s = Settings::new();
        s.status_bar_height = 32.0;
        assert_eq!(s.status_bar_height, 32.0);
    }
}
