use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanvasDragEligibility {
    Enabled,
    Disabled,
}

/// 当前编辑器指针捕获类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseCapture {
    None,
    TextSelection,
    CanvasDrag,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CanvasDragSession {
    pub source_range: Range<usize>,
    pub pressed_at: (f32, f32),
    pub source_generation: u32,
    pub eligibility: CanvasDragEligibility,
    pub started: bool,
}
