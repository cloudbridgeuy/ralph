use ralph_core::worker::CursorPermissions;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CursorSection {
    #[serde(default)]
    pub permissions: Option<toml::Value>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub fast: Option<bool>,
}

impl CursorSection {
    pub fn permissions_setting(&self) -> (CursorPermissions, Option<String>) {
        match self.permissions.as_ref().map(toml::Value::as_str) {
            None | Some(Some("ask")) => (CursorPermissions::Ask, None),
            Some(Some("yolo")) => (CursorPermissions::Yolo, None),
            Some(_) => (
                CursorPermissions::Ask,
                Some("[cursor] permissions must be \"ask\" or \"yolo\"; using \"ask\"".to_string()),
            ),
        }
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
        let config: AppConfig = toml::from_str(
            "[cursor]\nmodel = \"gpt-5\"\nreasoning_effort = \"low\"\nfast = false\n",
        )
        .unwrap();
        assert_eq!(config.cursor.model.as_deref(), Some("gpt-5"));
        assert_eq!(config.cursor.reasoning_effort.as_deref(), Some("low"));
        assert_eq!(config.cursor.fast, Some(false));
        assert_eq!(AppConfig::new().cursor.model, None);
        assert_eq!(AppConfig::new().cursor.reasoning_effort, None);
        assert_eq!(AppConfig::new().cursor.fast, None);
    }

    #[test]
    fn cursor_permissions_parse_and_default_to_ask() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");
        for (value, mode) in [
            ("yolo", CursorPermissions::Yolo),
            ("ask", CursorPermissions::Ask),
        ] {
            fs::write(&path, format!("[cursor]\npermissions = \"{value}\"\n")).unwrap();
            let config = AppConfig::load_from(&path).unwrap();
            assert_eq!(config.cursor.permissions_setting(), (mode, None));
        }
        assert_eq!(
            AppConfig::new().cursor.permissions_setting(),
            (CursorPermissions::Ask, None)
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
            let config = AppConfig::load_from(&path).unwrap();
            assert_eq!(config.chat.wrap_width, Some(80));
            let (mode, warning) = config.cursor.permissions_setting();
            assert_eq!(mode, CursorPermissions::Ask);
            assert!(warning.is_some());
        }
    }
}
