//! Centralized UI constants to eliminate magic numbers.

// === 尺寸 ===
pub const BAR_HEIGHT: f32 = 28.0; // 统一 header/tab bar/search bar 高度
pub const ROW_HEIGHT: f32 = 32.0; // 列表行高（sidebar 等）
pub const SIDEBAR_MIN_WIDTH: f32 = 160.0;
pub const SIDEBAR_MAX_WIDTH: f32 = 400.0;
pub const SIDEBAR_DEFAULT_WIDTH: f32 = 220.0;
pub const SCROLLBAR_THUMB_MIN_HEIGHT: f32 = 25.0;

pub const TITLE_BAR_HEIGHT: f32 = 36.0;

// === 间距 ===
pub const H_PADDING: f32 = 12.0;
pub const MEDIUM_GAP: f32 = 10.0;
pub const SMALL_GAP: f32 = 8.0;
pub const TINY_GAP: f32 = 4.0;
pub const MICRO_GAP: f32 = 2.0;

// === 字体 ===
pub const BODY_FONT_SIZE: f32 = 14.0;
pub const TITLE_FONT_SIZE: f32 = 13.0;
pub const CAPTION_FONT_SIZE: f32 = 10.0;
pub const LN_FONT_SCALE: f32 = 0.8; // 行号字号缩放比
pub const BASELINE_RATIO: f32 = 0.8; // 基线偏移比

// === 其他 ===
pub const BUTTON_SIZE: f32 = 16.0;
pub const CLOSE_BTN_SIZE: f32 = 12.0;
pub const UNDERLINE_ALPHA: f32 = 0.75;
pub const TRAFFIC_LIGHT_TOTAL_W: f32 = 96.0;
