//! Navigator trait — 纯数据导航接口。
//!
//! 条目集合 + 激活项切换。不涉及渲染、滚动、命中测试。
//! Workspace 是默认实现。

use std::any::Any;
use std::collections::HashSet;
use std::path::PathBuf;

/// 条目的 UI 投影，不引用 DocumentView。
#[derive(Debug, Clone)]
pub struct NavEntry {
    pub title: String,
    pub file_path: Option<PathBuf>,
    pub is_dirty: bool,
    pub pinned: bool,
}

/// 导航操作效果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavEffect {
    None,
    ActiveChanged,
    ItemsChanged,
}

impl NavEffect {
    pub fn merge(self, other: Self) -> Self {
        match (self, other) {
            (NavEffect::ActiveChanged, _) | (_, NavEffect::ActiveChanged) => {
                NavEffect::ActiveChanged
            }
            (NavEffect::ItemsChanged, _) | (_, NavEffect::ItemsChanged) => NavEffect::ItemsChanged,
            _ => NavEffect::None,
        }
    }
}

pub trait Navigator: Any {
    fn id(&self) -> &str;
    fn name(&self) -> &str;

    fn items(&self) -> Vec<NavEntry>;
    fn len(&self) -> usize {
        self.items().len()
    }
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn active_index(&self) -> usize;

    fn toggle_pin(&mut self, index: usize) -> NavEffect;
    fn is_pinned(&self, index: usize) -> bool;
    fn pinned_indices(&self) -> HashSet<usize>;

    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

#[cfg(test)]
mod tests {
    #[test]
    fn navigator_trait_does_not_expose_operations_that_discard_workspace_effects() {
        let source = include_str!("navigator.rs");
        let trait_definition = source
            .split("pub trait Navigator")
            .nth(1)
            .expect("navigator source must define the Navigator trait")
            .split("#[cfg(test)]")
            .next()
            .expect("Navigator trait must precede its tests");
        let method_name = ["switch", "_to"].concat();

        assert!(
            !trait_definition.contains(&format!("fn {method_name}")),
            "Navigator must not expose methods that reduce typed workspace lifecycle effects"
        );
    }
}
