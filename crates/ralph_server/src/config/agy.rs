use super::permissions::permission_setting;
use ralph_core::worker::PermissionMode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgySection {
    #[serde(default)]
    pub permissions: Option<toml::Value>,
    #[serde(default)]
    pub model: Option<String>,
}

impl AgySection {
    pub fn permissions_setting(&self) -> (PermissionMode, Option<String>) {
        permission_setting("agy", self.permissions.as_ref())
    }

    pub fn selection_errors(&self) -> Vec<String> {
        [("model", self.model.as_deref())]
            .into_iter()
            .filter_map(|(name, value)| {
                value
                    .filter(|value| value.is_empty() || value.chars().any(char::is_whitespace))
                    .map(|_| format!("[agy] {name} must be non-empty with no whitespace"))
            })
            .collect()
    }
}
