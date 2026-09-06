use crate::settings::ThemeMode;
use crate::view_mode::ViewMode;

const MIN_FONT_SIZE: f32 = 6.0;
const MAX_FONT_SIZE: f32 = 72.0;
const MIN_LINE_HEIGHT_RATIO: f32 = 1.0;
const MAX_LINE_HEIGHT_RATIO: f32 = 3.0;
const MIN_TAB_WIDTH: usize = 1;
const MAX_TAB_WIDTH: usize = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SettingsCategory {
    #[default]
    Appearance,
    Editor,
    Interface,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SettingsPersistenceView {
    #[default]
    Saved,
    SaveFailed {
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SettingsViewInput {
    pub theme_mode: ThemeMode,
    pub font_family: String,
    pub font_size: f32,
    pub line_height_ratio: f32,
    pub word_wrap: bool,
    pub markdown_first_line_indent: bool,
    pub show_line_numbers: bool,
    pub tab_width: usize,
    pub view_mode: ViewMode,
    pub show_status_bar: bool,
    pub persistence: SettingsPersistenceView,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SettingsViewAction {
    SetThemeMode(ThemeMode),
    SetFontFamily(String),
    SetFontSize(f32),
    SetLineHeightRatio(f32),
    SetWordWrap(bool),
    SetMarkdownFirstLineIndent(bool),
    SetShowLineNumbers(bool),
    SetTabWidth(usize),
    SetViewMode(ViewMode),
    SetShowStatusBar(bool),
    RetryPersistence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationError {
    InvalidNumber,
    OutOfRange,
    Empty,
}

impl ValidationError {
    pub fn message(self) -> &'static str {
        match self {
            Self::InvalidNumber => "请输入有效数字",
            Self::OutOfRange => "数值超出允许范围",
            Self::Empty => "内容不能为空",
        }
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str((*self).message())
    }
}

impl std::error::Error for ValidationError {}

pub fn parse_font_size(value: &str) -> Result<f32, ValidationError> {
    parse_bounded_float(value, MIN_FONT_SIZE, MAX_FONT_SIZE)
}

pub fn parse_line_height_ratio(value: &str) -> Result<f32, ValidationError> {
    parse_bounded_float(value, MIN_LINE_HEIGHT_RATIO, MAX_LINE_HEIGHT_RATIO)
}

pub fn parse_tab_width(value: &str) -> Result<usize, ValidationError> {
    let trimmed_value = value.trim();
    let parsed_value =
        trimmed_value.parse::<usize>().map_err(|_| ValidationError::InvalidNumber)?;
    if !(MIN_TAB_WIDTH..=MAX_TAB_WIDTH).contains(&parsed_value) {
        return Err(ValidationError::OutOfRange);
    }
    Ok(parsed_value)
}

pub fn parse_font_family(value: &str) -> Result<String, ValidationError> {
    let trimmed_value = value.trim();
    if trimmed_value.is_empty() {
        return Err(ValidationError::Empty);
    }
    Ok(trimmed_value.to_owned())
}

fn parse_bounded_float(
    value: &str,
    minimum_value: f32,
    maximum_value: f32,
) -> Result<f32, ValidationError> {
    let parsed_value = value.trim().parse::<f32>().map_err(|_| ValidationError::InvalidNumber)?;
    if !parsed_value.is_finite() {
        return Err(ValidationError::InvalidNumber);
    }
    if !(minimum_value..=maximum_value).contains(&parsed_value) {
        return Err(ValidationError::OutOfRange);
    }
    Ok(parsed_value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_settings_accept_only_documented_ranges() {
        assert_eq!(parse_font_size("6"), Ok(6.0));
        assert_eq!(parse_font_size("72"), Ok(72.0));
        assert_eq!(parse_font_size("5.9"), Err(ValidationError::OutOfRange));
        assert_eq!(parse_line_height_ratio("1.618"), Ok(1.618));
        assert_eq!(parse_tab_width("16"), Ok(16));
        assert_eq!(parse_tab_width("0"), Err(ValidationError::OutOfRange));
    }

    #[test]
    fn validators_trim_values_and_reject_non_finite_numbers() {
        assert_eq!(parse_font_size(" 18 "), Ok(18.0));
        assert_eq!(parse_font_size("NaN"), Err(ValidationError::InvalidNumber));
        assert_eq!(parse_line_height_ratio("inf"), Err(ValidationError::InvalidNumber));
        assert_eq!(parse_font_family("  Iosevka  "), Ok("Iosevka".to_owned()));
        assert_eq!(parse_font_family("   "), Err(ValidationError::Empty));
    }
}
