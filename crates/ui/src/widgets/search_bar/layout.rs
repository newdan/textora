use crate::core::Rect;

const DEFAULT_INPUT_LEFT_LOGICAL: f32 = 32.0;
const MINIMUM_INPUT_LEFT_LOGICAL: f32 = 4.0;
const DEFAULT_BUTTON_SIZE_LOGICAL: f32 = 20.0;
const COMPACT_BUTTON_SIZE_LOGICAL: f32 = 18.0;
const DEFAULT_BUTTON_GAP_LOGICAL: f32 = 4.0;
const COMPACT_BUTTON_GAP_LOGICAL: f32 = 2.0;
const DEFAULT_RIGHT_PADDING_LOGICAL: f32 = 8.0;
const COMPACT_RIGHT_PADDING_LOGICAL: f32 = 4.0;
const FIND_INPUT_MINIMUM_LOGICAL: f32 = 28.0;
const REPLACE_INPUTS_MINIMUM_LOGICAL: f32 = 44.0;
const SEPARATOR_WIDTH_LOGICAL: f32 = 12.0;
const FULL_REPLACE_ACTION_WIDTH_LOGICAL: f32 = 32.0;
const RESULT_CHARACTER_WIDTH_LOGICAL: f32 = 8.0;
const NO_RESULTS_WIDTH_LOGICAL: f32 = 70.0;
const COMPACT_LAYOUT_THRESHOLD_LOGICAL: f32 = 240.0;
const FULL_REPLACE_LABEL_THRESHOLD_LOGICAL: f32 = 480.0;

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub(super) struct SearchBarLayout {
    pub pill_rect: Rect,
    pub search_icon_rect: Rect,
    pub find_input_rect: Rect,
    pub replace_input_rect: Rect,
    pub separator_rect: Rect,
    pub close_btn_rect: Rect,
    pub toggle_replace_btn_rect: Rect,
    pub regex_btn_rect: Rect,
    pub prev_btn_rect: Rect,
    pub next_btn_rect: Rect,
    pub replace_btn_rect: Rect,
    pub replace_all_btn_rect: Rect,
    pub auxiliary_text_rect: Rect,
    pub full_replace_labels: bool,
}

pub(super) struct SearchBarLayoutInput {
    pub width: f32,
    pub height: f32,
    pub dpi: f32,
    pub replace_mode: bool,
    pub has_query: bool,
    pub match_count: usize,
    pub current_match: usize,
}

pub(super) fn calculate_search_bar_layout(input: SearchBarLayoutInput) -> SearchBarLayout {
    let width = input.width.max(0.0);
    let height = input.height.max(0.0);
    let dpi = input.dpi.max(f32::EPSILON);
    let pill_rect = Rect::new(0.0, 0.0, width, height);
    if width <= 0.0 || height <= 0.0 {
        return SearchBarLayout { pill_rect, ..SearchBarLayout::default() };
    }

    let compact = width < COMPACT_LAYOUT_THRESHOLD_LOGICAL * dpi;
    let button_gap =
        if compact { COMPACT_BUTTON_GAP_LOGICAL * dpi } else { DEFAULT_BUTTON_GAP_LOGICAL * dpi };
    let right_padding = if compact {
        COMPACT_RIGHT_PADDING_LOGICAL * dpi
    } else {
        DEFAULT_RIGHT_PADDING_LOGICAL * dpi
    };
    let action_count = if input.replace_mode { 4.0 } else { 2.0 };
    let minimum_input_width = if input.replace_mode {
        REPLACE_INPUTS_MINIMUM_LOGICAL * dpi
    } else {
        FIND_INPUT_MINIMUM_LOGICAL * dpi
    };
    let nominal_button_size =
        if compact { COMPACT_BUTTON_SIZE_LOGICAL * dpi } else { DEFAULT_BUTTON_SIZE_LOGICAL * dpi };
    let button_budget = (width
        - MINIMUM_INPUT_LEFT_LOGICAL * dpi
        - minimum_input_width
        - right_padding
        - action_count * button_gap)
        .max(0.0);
    let button_size = nominal_button_size.min(button_budget / action_count);
    let full_replace_labels =
        input.replace_mode && width >= FULL_REPLACE_LABEL_THRESHOLD_LOGICAL * dpi;
    let replace_action_width =
        if full_replace_labels { FULL_REPLACE_ACTION_WIDTH_LOGICAL * dpi } else { button_size };

    let essential_width = right_padding
        + (button_size + button_gap) * 2.0
        + if input.replace_mode { (replace_action_width + button_gap) * 2.0 } else { 0.0 };
    let input_left = (DEFAULT_INPUT_LEFT_LOGICAL * dpi)
        .min((width - essential_width - minimum_input_width).max(MINIMUM_INPUT_LEFT_LOGICAL * dpi));
    let mut optional_space = (width - input_left - essential_width - minimum_input_width).max(0.0);

    let regex_width = button_size + button_gap;
    let show_regex = optional_space >= regex_width && button_size > 0.0;
    if show_regex {
        optional_space -= regex_width;
    }

    let navigation_width = (button_size + button_gap) * 2.0;
    let show_navigation = input.has_query
        && input.match_count > 0
        && optional_space >= navigation_width
        && button_size > 0.0;
    if show_navigation {
        optional_space -= navigation_width;
    }

    let auxiliary_width = if input.match_count > 0 {
        let current = input.current_match.saturating_add(1).min(input.match_count);
        format!("{current}/{}", input.match_count).len() as f32
            * RESULT_CHARACTER_WIDTH_LOGICAL
            * dpi
    } else {
        NO_RESULTS_WIDTH_LOGICAL * dpi
    };
    let show_auxiliary = input.has_query && optional_space >= auxiliary_width + button_gap;

    let input_height = (18.0 * dpi).min(height);
    let input_y = (height - input_height) * 0.5;
    let button_y = (height - button_size) * 0.5;
    let mut right_edge = width - right_padding;
    let close_btn_rect =
        take_trailing_rect(&mut right_edge, button_size, button_size, button_y, button_gap);
    let toggle_replace_btn_rect =
        take_trailing_rect(&mut right_edge, button_size, button_size, button_y, button_gap);
    let regex_btn_rect = if show_regex {
        take_trailing_rect(&mut right_edge, button_size, button_size, button_y, button_gap)
    } else {
        Rect::ZERO
    };
    let next_btn_rect = if show_navigation {
        take_trailing_rect(&mut right_edge, button_size, button_size, button_y, button_gap)
    } else {
        Rect::ZERO
    };
    let prev_btn_rect = if show_navigation {
        take_trailing_rect(&mut right_edge, button_size, button_size, button_y, button_gap)
    } else {
        Rect::ZERO
    };
    let auxiliary_text_rect = if show_auxiliary {
        take_trailing_rect(&mut right_edge, auxiliary_width, input_height, input_y, button_gap)
    } else {
        Rect::ZERO
    };
    let replace_all_btn_rect = if input.replace_mode {
        take_trailing_rect(&mut right_edge, replace_action_width, button_size, button_y, button_gap)
    } else {
        Rect::ZERO
    };
    let replace_btn_rect = if input.replace_mode {
        take_trailing_rect(&mut right_edge, replace_action_width, button_size, button_y, button_gap)
    } else {
        Rect::ZERO
    };

    let input_right = right_edge.max(input_left);
    let (find_input_rect, replace_input_rect, separator_rect) = if input.replace_mode {
        split_replace_inputs(input_left, input_right, input_y, input_height, dpi)
    } else {
        (
            Rect::new(
                (input_left - 4.0 * dpi).max(0.0),
                input_y,
                (input_right - input_left + 4.0 * dpi).max(0.0),
                input_height,
            ),
            Rect::ZERO,
            Rect::ZERO,
        )
    };
    let icon_size = 14.0 * dpi;
    let search_icon_rect = if input_left >= 24.0 * dpi {
        Rect::new(5.0 * dpi, (height - icon_size) * 0.5, icon_size, icon_size)
    } else {
        Rect::ZERO
    };

    SearchBarLayout {
        pill_rect,
        search_icon_rect,
        find_input_rect,
        replace_input_rect,
        separator_rect,
        close_btn_rect,
        toggle_replace_btn_rect,
        regex_btn_rect,
        prev_btn_rect,
        next_btn_rect,
        replace_btn_rect,
        replace_all_btn_rect,
        auxiliary_text_rect,
        full_replace_labels,
    }
}

fn take_trailing_rect(right_edge: &mut f32, width: f32, height: f32, y: f32, gap: f32) -> Rect {
    if width <= 0.0 || height <= 0.0 {
        return Rect::ZERO;
    }
    let x = (*right_edge - width).max(0.0);
    let rect = Rect::new(x, y, (*right_edge - x).max(0.0), height);
    *right_edge = (x - gap).max(0.0);
    rect
}

fn split_replace_inputs(
    input_left: f32,
    input_right: f32,
    input_y: f32,
    input_height: f32,
    dpi: f32,
) -> (Rect, Rect, Rect) {
    let separator_width = (SEPARATOR_WIDTH_LOGICAL * dpi).min((input_right - input_left).max(0.0));
    let field_width = ((input_right - input_left - separator_width) * 0.5).max(0.0);
    let find_left = (input_left - 4.0 * dpi).max(0.0);
    let find_right = input_left + field_width;
    let replace_left = find_right + separator_width;
    let replace_box_left = (replace_left - 4.0 * dpi).max(find_right);
    (
        Rect::new(find_left, input_y, (find_right - find_left).max(0.0), input_height),
        Rect::new(
            replace_box_left,
            input_y,
            (input_right - replace_box_left).max(0.0),
            input_height,
        ),
        Rect::new(find_right, input_y, separator_width, input_height),
    )
}
