use super::permissions::permission_setting;
use ralph_core::worker::PermissionMode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CodexSection {
    #[serde(default)]
    pub permissions: Option<toml::Value>,
}

impl CodexSection {
    pub fn permissions_setting(&self) -> (PermissionMode, Option<String>) {
        permission_setting("codex", self.permissions.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    #[test]
    fn codex_permissions_parse_default_to_ask_and_warn_on_wrong_values() {
        let parse = |text: &str| {
            toml::from_str::<AppConfig>(text)
                .unwrap()
                .codex
                .permissions_setting()
        };
        assert_eq!(
            parse("[codex]\npermissions = \"yolo\"\n"),
            (PermissionMode::Yolo, None)
        );
        assert_eq!(parse(""), (PermissionMode::Ask, None));
        let (mode, warning) = parse("[codex]\npermissions = true\n");
        assert_eq!(mode, PermissionMode::Ask);
        assert!(warning.unwrap().starts_with("[codex]"));
    }
}
