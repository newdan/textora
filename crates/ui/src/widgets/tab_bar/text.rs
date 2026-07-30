//! tab_bar/text.rs — 文本宽度估算与截断。
//! 实际实现在 core::text_util，此处保留 re-export 以维持向后兼容。

pub(crate) use crate::core::text_util::compute_text_width;
pub(crate) use crate::core::text_util::truncate_title_by_width;
