//! Top-level view mode (Sidebar vs Tabs). Persisted in settings.toml.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum ViewMode {
    #[default]
    Sidebar,
    Tabs,
}
