use super::permissions::permission_setting;
use ralph_core::worker::PermissionMode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CursorSection {
    #[serde(default)]
    pub permissions: Option<toml::Value>,
    #[serde(default)]
    pub model: Option<String>,
}

impl CursorSection {
    pub fn permissions_setting(&self) -> (PermissionMode, Option<String>) {
        permission_setting("cursor", self.permissions.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn cursor_model_parses_and_defaults_to_unset() {
        let config: AppConfig = toml::from_str("[cursor]\nmodel = \"gpt-5\"\n").unwrap();
        assert_eq!(config.cursor.model.as_deref(), Some("gpt-5"));
        assert_eq!(AppConfig::new().cursor.model, None);
    }

    #[test]
    fn cursor_permissions_parse_and_default_to_ask() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");
        for (value, mode) in [("yolo", PermissionMode::Yolo), ("ask", PermissionMode::Ask)] {
            fs::write(&path, format!("[cursor]\npermissions = \"{value}\"\n")).unwrap();
            let config = AppConfig::load_layers(&path, None).unwrap();
            assert_eq!(config.cursor.permissions_setting(), (mode, None));
        }
        assert_eq!(
            AppConfig::new().cursor.permissions_setting(),
            (PermissionMode::Ask, None)
        );
    }

    #[test]
    fn cursor_permissions_wrong_value_warns_and_keeps_other_keys() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");
        for value in ["\"always\"", "true", "[1]"] {
            fs::write(
                &path,
                format!("[cursor]\npermissions = {value}\n[chat]\nwrap_width = 80\n"),
            )
            .unwrap();
            let config = AppConfig::load_layers(&path, None).unwrap();
            assert_eq!(config.chat.wrap_width, Some(80));
            let (mode, warning) = config.cursor.permissions_setting();
            assert_eq!(mode, PermissionMode::Ask);
            assert!(warning.is_some());
        }
    }
}
