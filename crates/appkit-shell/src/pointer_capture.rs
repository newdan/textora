use std::ops::Range;

/// 当前编辑器指针捕获类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseCapture {
    None,
    TextSelection,
    CanvasDrag,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CanvasDragSession {
    pub source_range: Range<usize>,
    pub pressed_at: (f32, f32),
    pub source_generation: u32,
    pub started: bool,
}
