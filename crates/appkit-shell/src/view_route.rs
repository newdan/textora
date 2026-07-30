use std::collections::HashSet;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewPathMatcher {
    FileNameSuffix(&'static str),
    Extension(&'static str),
}

impl ViewPathMatcher {
    fn matches(self, path: &Path) -> bool {
        match self {
            Self::FileNameSuffix(suffix) => path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(suffix)),
            Self::Extension(extension) => {
                path.extension().and_then(|value| value.to_str()) == Some(extension)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewRouteRule {
    pub matcher: ViewPathMatcher,
    pub default_plugin: &'static str,
    pub toggle_target: Option<&'static str>,
    pub priority: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewRouteError {
    DuplicatePriority(u16),
    UnknownPluginId(&'static str),
}

#[derive(Debug)]
pub struct ViewRouteTable {
    rules: Vec<ViewRouteRule>,
}

impl ViewRouteTable {
    pub fn new(
        mut rules: Vec<ViewRouteRule>,
        registered_plugin_ids: &HashSet<&'static str>,
    ) -> Result<Self, ViewRouteError> {
        let mut priorities = HashSet::with_capacity(rules.len());
        for rule in &rules {
            if !priorities.insert(rule.priority) {
                return Err(ViewRouteError::DuplicatePriority(rule.priority));
            }
            Self::validate_plugin_id(rule.default_plugin, registered_plugin_ids)?;
            if let Some(toggle_target) = rule.toggle_target {
                Self::validate_plugin_id(toggle_target, registered_plugin_ids)?;
            }
        }

        rules.sort_unstable_by_key(|rule| std::cmp::Reverse(rule.priority));
        Ok(Self { rules })
    }

    pub fn resolve(&self, path: &Path) -> Option<&ViewRouteRule> {
        self.rules.iter().find(|rule| rule.matcher.matches(path))
    }

    fn validate_plugin_id(
        plugin_id: &'static str,
        registered_plugin_ids: &HashSet<&'static str>,
    ) -> Result<(), ViewRouteError> {
        if registered_plugin_ids.contains(plugin_id) {
            return Ok(());
        }
        Err(ViewRouteError::UnknownPluginId(plugin_id))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::Path;

    use ui::plugin::{PLUGIN_EDITOR, PLUGIN_MARKDOWN_EDITOR, PLUGIN_MINDMAP, PLUGIN_NOVEL_VIEW};

    use super::{ViewPathMatcher, ViewRouteError, ViewRouteRule, ViewRouteTable};

    fn registered_plugin_ids() -> HashSet<&'static str> {
        [PLUGIN_EDITOR, PLUGIN_MARKDOWN_EDITOR, PLUGIN_MINDMAP, PLUGIN_NOVEL_VIEW]
            .into_iter()
            .collect()
    }

    fn route_table() -> ViewRouteTable {
        ViewRouteTable::new(
            vec![
                ViewRouteRule {
                    matcher: ViewPathMatcher::FileNameSuffix(".mmap.md"),
                    default_plugin: PLUGIN_MINDMAP,
                    toggle_target: Some(PLUGIN_MARKDOWN_EDITOR),
                    priority: 100,
                },
                ViewRouteRule {
                    matcher: ViewPathMatcher::Extension("md"),
                    default_plugin: PLUGIN_MARKDOWN_EDITOR,
                    toggle_target: Some(PLUGIN_EDITOR),
                    priority: 20,
                },
                ViewRouteRule {
                    matcher: ViewPathMatcher::Extension("txt"),
                    default_plugin: PLUGIN_EDITOR,
                    toggle_target: Some(PLUGIN_NOVEL_VIEW),
                    priority: 10,
                },
            ],
            &registered_plugin_ids(),
        )
        .expect("test route table should be valid")
    }

    #[test]
    fn higher_priority_mmap_suffix_beats_markdown_extension() {
        let routes = route_table();
        let matched = routes.resolve(Path::new("brain.mmap.md")).expect("mmap route should match");

        assert_eq!(matched.default_plugin, PLUGIN_MINDMAP);
        assert_eq!(matched.toggle_target, Some(PLUGIN_MARKDOWN_EDITOR));
    }

    #[test]
    fn text_extension_maps_to_editor_and_novel_view() {
        let routes = route_table();
        let matched = routes.resolve(Path::new("draft.txt")).expect("text route should match");

        assert_eq!(matched.default_plugin, PLUGIN_EDITOR);
        assert_eq!(matched.toggle_target, Some(PLUGIN_NOVEL_VIEW));
    }

    #[test]
    fn duplicate_priorities_are_rejected() {
        let error = ViewRouteTable::new(
            vec![
                ViewRouteRule {
                    matcher: ViewPathMatcher::Extension("md"),
                    default_plugin: PLUGIN_MARKDOWN_EDITOR,
                    toggle_target: Some(PLUGIN_EDITOR),
                    priority: 10,
                },
                ViewRouteRule {
                    matcher: ViewPathMatcher::Extension("txt"),
                    default_plugin: PLUGIN_EDITOR,
                    toggle_target: Some(PLUGIN_NOVEL_VIEW),
                    priority: 10,
                },
            ],
            &registered_plugin_ids(),
        )
        .expect_err("duplicate priorities must be rejected");

        assert_eq!(error, ViewRouteError::DuplicatePriority(10));
    }

    #[test]
    fn every_referenced_plugin_id_must_be_registered() {
        let error = ViewRouteTable::new(
            vec![ViewRouteRule {
                matcher: ViewPathMatcher::Extension("md"),
                default_plugin: "missing-plugin",
                toggle_target: Some(PLUGIN_EDITOR),
                priority: 10,
            }],
            &registered_plugin_ids(),
        )
        .expect_err("unknown plugin IDs must be rejected");

        assert_eq!(error, ViewRouteError::UnknownPluginId("missing-plugin"));
    }
}
