//! Tab bar — 拆分为 types / text / layout / render / hit / state + widget + tests。
//! mod.rs 仅做 子模块声明 + 选择性 re-export，不定义类型。

// ── 共享类型模块 ──
mod types;
pub use types::{TabBarCtx, TabInfo, tab_bar_height};

// ── 子模块声明 ──
pub(crate) mod hit;
pub(crate) mod layout;
pub(crate) mod state;
pub(crate) mod text;
pub(crate) mod widget;

#[cfg(test)]
#[path = "tests.rs"]
mod tab_bar_tests;

// ── 选择性 re-export ──
pub use crate::core::widget::MouseButton;
pub(crate) use hit::TabHit;
pub use layout::{NavButtonLayout, TabBarLayout, TabEntry, TabIndicator};
pub(crate) use state::TabBarInput;
pub use state::{TabBarAction, TabBarState};
pub(crate) use text::truncate_title_by_width;

// Widget re-export
pub use widget::{TabBarWidget, TabBarWidgetInput};
