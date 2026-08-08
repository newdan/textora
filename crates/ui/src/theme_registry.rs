use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::theme::ThemeDefinition;
use crate::theme_file::ThemeFile;

#[derive(Debug, Clone)]
pub struct ThemeSource {
    pub id: String,
    pub path: PathBuf,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeLoadError {
    ReservedId { id: String, path: PathBuf },
    DuplicateId { id: String, first_path: Option<PathBuf>, duplicate_path: PathBuf },
    TomlParse { id: String, path: PathBuf, message: String },
    UnknownExtends { id: String, path: PathBuf, base_id: String },
    CyclicExtends { ids: Vec<String> },
    BaseThemeFailed { id: String, path: PathBuf, base_id: String },
    Resolve { id: String, path: PathBuf, message: String },
}

impl std::fmt::Display for ThemeLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReservedId { id, path } => {
                write!(f, "{}: reserved theme id {id}", path.display())
            }
            Self::DuplicateId { id, first_path, duplicate_path } => match first_path {
                Some(first) => write!(
                    f,
                    "{}: duplicate theme id {id}; first declared at {}",
                    duplicate_path.display(),
                    first.display(),
                ),
                None => {
                    write!(f, "{}: theme id {id} is already registered", duplicate_path.display())
                }
            },
            Self::TomlParse { id, path, message } => {
                write!(f, "{}: failed to parse theme {id}: {message}", path.display())
            }
            Self::UnknownExtends { id, path, base_id } => {
                write!(f, "{}: theme {id} extends unknown theme {base_id}", path.display())
            }
            Self::CyclicExtends { ids } => {
                write!(f, "cyclic theme inheritance: {}", ids.join(" -> "))
            }
            Self::BaseThemeFailed { id, path, base_id } => {
                write!(f, "{}: theme {id} depends on failed theme {base_id}", path.display())
            }
            Self::Resolve { id, path, message } => {
                write!(f, "{}: failed to resolve theme {id}: {message}", path.display())
            }
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ThemeRegistrationReport {
    pub registered_ids: Vec<String>,
    pub errors: Vec<ThemeLoadError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterError {
    ReservedId(String),
    DuplicateId(String),
}

#[derive(Debug, Clone)]
pub struct ThemeRegistry {
    themes: BTreeMap<String, ThemeDefinition>,
    default_dark: ThemeDefinition,
    default_light: ThemeDefinition,
}

struct ParsedTheme {
    path: PathBuf,
    file: ThemeFile,
    base_id: String,
}

#[derive(Clone)]
enum ResolveState {
    Visiting,
    Resolved(Box<ThemeDefinition>),
    Failed,
}

/// Built-in theme ids — hyphenated to match TOML file naming conventions.
pub const BUILTIN_DARK_ID: &str = "default-dark";
pub const BUILTIN_LIGHT_ID: &str = "default-light";

impl ThemeRegistry {
    /// Create registry with built-in defaults.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            themes: BTreeMap::new(),
            default_dark: ThemeDefinition::default_dark(),
            default_light: ThemeDefinition::default_light(),
        }
    }

    /// Register a user theme. Returns Err if id clashes with built-in reserved keys.
    pub fn register(&mut self, id: String, def: ThemeDefinition) -> Result<(), RegisterError> {
        if matches!(id.as_str(), BUILTIN_DARK_ID | BUILTIN_LIGHT_ID) {
            return Err(RegisterError::ReservedId(id));
        }
        if self.themes.contains_key(&id) {
            return Err(RegisterError::DuplicateId(id));
        }
        self.themes.insert(id, def);
        Ok(())
    }

    /// Look up a definition by id. Pure immutable query — no side effects.
    pub fn get(&self, id: &str) -> Option<&ThemeDefinition> {
        match id {
            BUILTIN_DARK_ID => Some(&self.default_dark),
            BUILTIN_LIGHT_ID => Some(&self.default_light),
            _ => self.themes.get(id),
        }
    }

    /// Look up a definition by id, falling back to the built-in default for the
    /// given appearance. Never returns None — guarantees a valid theme.
    pub fn get_or_default(&self, id: &str, prefer_dark: bool) -> &ThemeDefinition {
        self.get(id).unwrap_or(if prefer_dark { &self.default_dark } else { &self.default_light })
    }

    /// Get the built-in default dark definition.
    pub fn default_dark(&self) -> &ThemeDefinition {
        &self.default_dark
    }

    /// Get the built-in default light definition.
    pub fn default_light(&self) -> &ThemeDefinition {
        &self.default_light
    }

    /// List all available theme ids (built-in + user-registered).
    pub fn list_ids(&self) -> Vec<String> {
        let mut ids = vec![BUILTIN_DARK_ID.to_owned(), BUILTIN_LIGHT_ID.to_owned()];
        ids.extend(self.themes.keys().cloned());
        ids.sort();
        ids
    }

    /// Number of user-registered themes (excludes built-in defaults).
    pub fn len(&self) -> usize {
        self.themes.len()
    }

    /// Whether there are no user-registered themes.
    pub fn is_empty(&self) -> bool {
        self.themes.is_empty()
    }

    /// Eagerly parse, resolve, and register all theme sources.
    /// All sources are fully resolved before the function returns.
    /// Individual failures are collected in the report; successful themes are registered.
    pub fn register_sources(
        &mut self,
        sources: impl IntoIterator<Item = ThemeSource>,
    ) -> ThemeRegistrationReport {
        let mut sources: Vec<_> = sources.into_iter().collect();
        sources.sort_by(|a, b| (&a.path, &a.id).cmp(&(&b.path, &b.id)));

        let mut report = ThemeRegistrationReport::default();
        let mut accepted = BTreeMap::<String, ThemeSource>::new();
        for source in sources {
            if matches!(source.id.as_str(), BUILTIN_DARK_ID | BUILTIN_LIGHT_ID) {
                report.errors.push(ThemeLoadError::ReservedId { id: source.id, path: source.path });
                continue;
            }
            if let Some(first) = accepted.get(&source.id) {
                report.errors.push(ThemeLoadError::DuplicateId {
                    id: source.id,
                    first_path: Some(first.path.clone()),
                    duplicate_path: source.path,
                });
                continue;
            }
            if self.themes.contains_key(&source.id) {
                report.errors.push(ThemeLoadError::DuplicateId {
                    id: source.id,
                    first_path: None,
                    duplicate_path: source.path,
                });
                continue;
            }
            accepted.insert(source.id.clone(), source);
        }

        let mut parsed = BTreeMap::new();
        let mut failed_ids = BTreeSet::new();
        for (id, source) in accepted {
            match toml::from_str::<ThemeFile>(&source.content) {
                Ok(file) => {
                    let base_id = file.extends.clone().unwrap_or_else(|| {
                        if file.is_dark.unwrap_or(true) {
                            BUILTIN_DARK_ID
                        } else {
                            BUILTIN_LIGHT_ID
                        }
                        .to_owned()
                    });
                    parsed.insert(id, ParsedTheme { path: source.path, file, base_id });
                }
                Err(error) => {
                    failed_ids.insert(id.clone());
                    report.errors.push(ThemeLoadError::TomlParse {
                        id,
                        path: source.path,
                        message: error.to_string(),
                    });
                }
            }
        }

        let ids: Vec<_> = parsed.keys().cloned().collect();
        let mut states = BTreeMap::new();
        let mut cycle_members = BTreeSet::new();
        for id in ids {
            let mut stack = Vec::new();
            if let Some(definition) = resolve_theme(
                &id,
                &parsed,
                &self.themes,
                &self.default_dark,
                &self.default_light,
                &failed_ids,
                &mut states,
                &mut stack,
                &mut cycle_members,
                &mut report.errors,
            ) {
                self.themes.insert(id.clone(), definition);
                report.registered_ids.push(id);
            }
        }

        report.registered_ids.sort();
        sort_errors(&mut report.errors, &parsed);
        report
    }

    /// Remove all user themes and prepare for re-registration.
    pub fn clear_user_themes(&mut self) {
        self.themes.clear();
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "recursive theme resolution carries explicit graph state for cycle diagnostics"
)]
fn resolve_theme(
    id: &str,
    parsed: &BTreeMap<String, ParsedTheme>,
    existing: &BTreeMap<String, ThemeDefinition>,
    default_dark: &ThemeDefinition,
    default_light: &ThemeDefinition,
    failed_ids: &BTreeSet<String>,
    states: &mut BTreeMap<String, ResolveState>,
    stack: &mut Vec<String>,
    cycle_members: &mut BTreeSet<String>,
    errors: &mut Vec<ThemeLoadError>,
) -> Option<ThemeDefinition> {
    if id == BUILTIN_DARK_ID {
        return Some(default_dark.clone());
    }
    if id == BUILTIN_LIGHT_ID {
        return Some(default_light.clone());
    }
    if let Some(definition) = existing.get(id) {
        return Some(definition.clone());
    }
    match states.get(id) {
        Some(ResolveState::Resolved(definition)) => return Some((**definition).clone()),
        Some(ResolveState::Failed) => return None,
        Some(ResolveState::Visiting) => {
            let start = stack.iter().position(|entry| entry == id).unwrap();
            let mut raw = stack[start..].to_vec();
            raw.push(id.to_owned());
            let cycle = canonical_cycle(raw);
            for member in &cycle[..cycle.len() - 1] {
                cycle_members.insert(member.clone());
                states.insert(member.clone(), ResolveState::Failed);
            }
            if !errors.iter().any(|error| {
                matches!(
                    error,
                    ThemeLoadError::CyclicExtends { ids } if ids == &cycle
                )
            }) {
                errors.push(ThemeLoadError::CyclicExtends { ids: cycle });
            }
            return None;
        }
        None => {}
    }

    let theme = parsed.get(id)?;
    states.insert(id.to_owned(), ResolveState::Visiting);
    stack.push(id.to_owned());

    let base = if failed_ids.contains(&theme.base_id) {
        None
    } else if theme.base_id == BUILTIN_DARK_ID {
        Some(default_dark.clone())
    } else if theme.base_id == BUILTIN_LIGHT_ID {
        Some(default_light.clone())
    } else if let Some(definition) = existing.get(&theme.base_id) {
        Some(definition.clone())
    } else if parsed.contains_key(&theme.base_id) {
        resolve_theme(
            &theme.base_id,
            parsed,
            existing,
            default_dark,
            default_light,
            failed_ids,
            states,
            stack,
            cycle_members,
            errors,
        )
    } else {
        errors.push(ThemeLoadError::UnknownExtends {
            id: id.to_owned(),
            path: theme.path.clone(),
            base_id: theme.base_id.clone(),
        });
        None
    };

    stack.pop();
    let Some(base) = base else {
        if !cycle_members.contains(id)
            && !errors.iter().any(|error| {
                matches!(
                    error,
                    ThemeLoadError::UnknownExtends { id: failed, .. } if failed == id
                )
            })
        {
            errors.push(ThemeLoadError::BaseThemeFailed {
                id: id.to_owned(),
                path: theme.path.clone(),
                base_id: theme.base_id.clone(),
            });
        }
        states.insert(id.to_owned(), ResolveState::Failed);
        return None;
    };

    match theme.file.resolve(&base) {
        Ok(mut definition) => {
            if theme.file.display_name.is_none() && definition.display_name == base.display_name {
                definition.display_name = id.to_owned();
            }
            states.insert(id.to_owned(), ResolveState::Resolved(Box::new(definition.clone())));
            Some(definition)
        }
        Err(error) => {
            errors.push(ThemeLoadError::Resolve {
                id: id.to_owned(),
                path: theme.path.clone(),
                message: error.to_string(),
            });
            states.insert(id.to_owned(), ResolveState::Failed);
            None
        }
    }
}

fn canonical_cycle(mut ids: Vec<String>) -> Vec<String> {
    ids.pop();
    let start = ids.iter().enumerate().min_by_key(|(_, id)| *id).map(|(i, _)| i).unwrap();
    ids.rotate_left(start);
    ids.push(ids[0].clone());
    ids
}

fn sort_errors(errors: &mut [ThemeLoadError], parsed: &BTreeMap<String, ParsedTheme>) {
    errors.sort_by_key(|error| {
        let (path, kind, id) = match error {
            ThemeLoadError::ReservedId { id, path } => (path.clone(), 0, id.clone()),
            ThemeLoadError::DuplicateId { id, duplicate_path, .. } => {
                (duplicate_path.clone(), 1, id.clone())
            }
            ThemeLoadError::TomlParse { id, path, .. } => (path.clone(), 2, id.clone()),
            ThemeLoadError::UnknownExtends { id, path, .. } => (path.clone(), 3, id.clone()),
            ThemeLoadError::CyclicExtends { ids } => {
                (parsed.get(&ids[0]).unwrap().path.clone(), 4, ids[0].clone())
            }
            ThemeLoadError::BaseThemeFailed { id, path, .. } => (path.clone(), 5, id.clone()),
            ThemeLoadError::Resolve { id, path, .. } => (path.clone(), 6, id.clone()),
        };
        (path, kind, id, error.to_string())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(id: &str, path: &str, content: &str) -> ThemeSource {
        ThemeSource { id: id.into(), path: path.into(), content: content.into() }
    }

    #[test]
    fn register_sources_resolves_every_source_before_returning() {
        let mut registry = ThemeRegistry::new();
        let report = registry.register_sources([
            source("good", "z-good.toml", "is_dark = false\n"),
            source("bad", "a-bad.toml", "[palette]\naccent = \"not-hex\"\n"),
        ]);

        assert_eq!(report.registered_ids, vec!["good"]);
        assert!(matches!(
            report.errors.as_slice(),
            [ThemeLoadError::Resolve { id, .. }] if id == "bad"
        ));
        assert!(registry.get("good").is_some());
        assert!(registry.get("bad").is_none());
    }

    #[test]
    fn get_is_an_immutable_side_effect_free_query() {
        fn query<'a>(registry: &'a ThemeRegistry, id: &str) -> Option<&'a ThemeDefinition> {
            registry.get(id)
        }

        let registry = ThemeRegistry::new();
        assert!(query(&registry, BUILTIN_DARK_ID).is_some());
        assert!(query(&registry, "missing").is_none());
    }

    #[test]
    fn unrelated_valid_theme_survives_unknown_base() {
        let mut registry = ThemeRegistry::new();
        let report = registry.register_sources([
            source("broken", "a.toml", "extends = \"missing\"\n"),
            source("valid", "b.toml", "is_dark = true\n"),
        ]);

        assert_eq!(report.registered_ids, vec!["valid"]);
        assert!(matches!(
            report.errors.as_slice(),
            [ThemeLoadError::UnknownExtends { id, base_id, .. }]
                if id == "broken" && base_id == "missing"
        ));
    }

    #[test]
    fn inheritance_is_independent_of_source_order() {
        let mut registry = ThemeRegistry::new();
        let report = registry.register_sources([
            source("derived", "a.toml", "extends = \"base\"\n[editor]\ncursor = \"#00FF00\"\n"),
            source("base", "z.toml", "is_dark = true\n[palette]\naccent = \"#FF0000\"\n"),
        ]);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(report.registered_ids, vec!["base", "derived"]);
        assert_eq!(registry.get("derived").unwrap().editor.cursor, [0.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn cycle_is_canonical_and_dependent_reports_base_failure() {
        let mut registry = ThemeRegistry::new();
        let report = registry.register_sources([
            source("c", "c.toml", "extends = \"a\"\n"),
            source("b", "b.toml", "extends = \"a\"\n"),
            source("a", "a.toml", "extends = \"b\"\n"),
        ]);
        assert!(report.errors.iter().any(|error| matches!(
            error,
            ThemeLoadError::CyclicExtends { ids } if ids == &["a", "b", "a"]
        )));
        assert!(report.errors.iter().any(|error| matches!(
            error,
            ThemeLoadError::BaseThemeFailed { id, base_id, .. }
                if id == "c" && base_id == "a"
        )));
        assert!(registry.is_empty());
    }

    #[test]
    fn duplicate_first_source_wins_even_when_it_is_invalid() {
        let mut registry = ThemeRegistry::new();
        let report = registry.register_sources([
            source("same", "a.toml", "not = [valid"),
            source("same", "b.toml", "is_dark = true\n"),
        ]);
        assert!(report.errors.iter().any(|e| matches!(e, ThemeLoadError::DuplicateId { .. })));
        assert!(report.errors.iter().any(|e| matches!(e, ThemeLoadError::TomlParse { .. })));
        assert!(registry.get("same").is_none());
    }

    #[test]
    fn reserved_and_existing_ids_are_rejected_without_overwrite() {
        let mut registry = ThemeRegistry::new();
        registry.register("existing".into(), ThemeDefinition::default_dark()).unwrap();
        let report = registry.register_sources([
            source(BUILTIN_DARK_ID, "a.toml", "is_dark = true\n"),
            source("existing", "b.toml", "is_dark = false\n"),
        ]);
        assert!(matches!(
            &report.errors[0],
            ThemeLoadError::ReservedId { id, .. } if id == BUILTIN_DARK_ID
        ));
        assert!(matches!(
            &report.errors[1],
            ThemeLoadError::DuplicateId { id, first_path: None, .. } if id == "existing"
        ));
        assert!(registry.get("existing").unwrap().is_dark);
    }

    #[test]
    fn clear_allows_same_user_id_to_register_again() {
        let mut registry = ThemeRegistry::new();
        assert_eq!(
            registry.register_sources([source("user", "a.toml", "")]).registered_ids,
            vec!["user"]
        );
        assert_eq!(registry.len(), 1);
        registry.clear_user_themes();
        assert!(registry.is_empty());
        assert_eq!(
            registry.register_sources([source("user", "b.toml", "")]).registered_ids,
            vec!["user"]
        );
    }

    #[test]
    fn empty_batch_and_unknown_fallback_are_stable() {
        let mut registry = ThemeRegistry::new();
        assert_eq!(
            registry.register_sources(Vec::<ThemeSource>::new()),
            ThemeRegistrationReport::default()
        );
        assert!(registry.get_or_default("missing", true).is_dark);
        assert!(!registry.get_or_default("missing", false).is_dark);
        assert_eq!(
            registry.list_ids(),
            vec![BUILTIN_DARK_ID.to_owned(), BUILTIN_LIGHT_ID.to_owned()]
        );
    }

    #[test]
    fn repeated_batches_produce_identically_ordered_errors() {
        let make_report = || {
            let mut registry = ThemeRegistry::new();
            registry.register_sources([
                source("z", "z.toml", "extends = \"missing\"\n"),
                source("a", "a.toml", "not = [valid"),
            ])
        };
        assert_eq!(make_report().errors, make_report().errors);
    }

    #[test]
    fn register_invalid_hex() {
        let mut registry = ThemeRegistry::new();
        let report = registry.register_sources([source(
            "invalid-hex",
            "invalid-hex.toml",
            "is_dark = true\n[palette]\naccent = \"not-hex\"\n",
        )]);
        assert!(!report.errors.is_empty());
        assert!(registry.get("invalid-hex").is_none());
    }

    #[test]
    fn user_theme_extends_another_user_theme() {
        let mut registry = ThemeRegistry::new();
        let report = registry.register_sources([
            source("a-base", "a-base.toml", "is_dark = true\n[palette]\naccent = \"#FF6B6B\"\n"),
            source(
                "z-derived",
                "z-derived.toml",
                "extends = \"a-base\"\nis_dark = true\n[editor]\ncursor = \"#00FF00\"\n",
            ),
        ]);
        assert!(report.errors.is_empty(), "errors: {:?}", report.errors);
        assert_eq!(registry.len(), 2);

        let derived = registry.get("z-derived").unwrap().clone();
        assert!((derived.palette.accent[0] - 1.0).abs() < 0.01);
        assert!((derived.palette.accent[1] - 0.42).abs() < 0.01);
        assert!((derived.palette.accent[2] - 0.42).abs() < 0.01);
        assert_eq!(derived.editor.cursor, [0.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn user_theme_extends_chain_order_independent() {
        let mut registry = ThemeRegistry::new();
        let report = registry.register_sources([
            source("a-derived", "a-derived.toml", "extends = \"z-base\"\nis_dark = true\n"),
            source("z-base", "z-base.toml", "is_dark = true\n[palette]\naccent = \"#FF0000\"\n"),
        ]);
        assert!(report.errors.is_empty(), "errors: {:?}", report.errors);
        let derived = registry.get("a-derived").unwrap().clone();
        assert_eq!(derived.palette.accent, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn empty_file_loads_with_defaults() {
        let mut registry = ThemeRegistry::new();
        let report = registry.register_sources([source("empty", "empty.toml", "")]);
        assert!(report.errors.is_empty(), "errors: {:?}", report.errors);
        assert_eq!(registry.len(), 1);
        let def = registry.get("empty").unwrap().clone();
        assert!(def.is_dark);
    }
}
