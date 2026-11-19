use super::permissions::permission_setting;
use ralph_core::worker::PermissionMode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgySection {
    #[serde(default)]
    pub permissions: Option<toml::Value>,
}

impl AgySection {
    pub fn permissions_setting(&self) -> (PermissionMode, Option<String>) {
        permission_setting("agy", self.permissions.as_ref())
    }
}
