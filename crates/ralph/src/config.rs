//! Application configuration management (Imperative Shell).
//!
//! This module handles loading and managing application configuration
//! from the config file. The config file location follows platform conventions
//! and can be overridden via the `RALPH_CONFIG_DIR` environment variable.
//!
//! # Configuration File Location
//!
//! By default:
//! - **Linux**: `~/.config/ralph/config.toml`
//! - **macOS**: `~/Library/Application Support/ralph/config.toml`
//! - **Windows**: `%APPDATA%\ralph\config.toml`
//!
//! Override with `RALPH_CONFIG_DIR` environment variable.
//!
//! # Configuration Precedence
//!
//! Settings are resolved in this order (highest priority first):
//! 1. CLI flags (e.g., `--theme`)
//! 2. Environment variables (e.g., `RALPH_THEME`)
//! 3. Config file (`config.toml` in config directory)
//! 4. Default values
//!
//! # Example Config File
//!
//! ```toml
//! [theme]
//! name = "Monokai Extended"  # or path like "/path/to/theme.tmTheme"
//! no_background = false
//! ```

use crate::invoke::{
    resolve_permission_mode, ASK_DEFAULT_PERMISSION_MODE, CHAT_DEFAULT_PERMISSION_MODE,
};
use crate::paths;
use ralph_core::inbox_alert::UnknownFocus;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Error type for configuration operations.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Failed to read configuration file.
    #[error("Failed to read config file at {path}: {source}")]
    ReadConfig {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// Failed to parse configuration file.
    #[error("Failed to parse config file at {path}: {source}")]
    ParseConfig {
        path: String,
        #[source]
        source: toml::de::Error,
    },

    /// Failed to serialize configuration.
    #[error("Failed to serialize config: {0}")]
    SerializeConfig(#[from] toml::ser::Error),

    /// Failed to write configuration file.
    #[error("Failed to write config file at {path}: {source}")]
    WriteConfig {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Application configuration.
///
/// This struct represents the full configuration that can be stored
/// in the config file. All fields are optional to support partial
/// configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    /// Theme configuration section.
    #[serde(default)]
    pub theme: ThemeSection,

    #[serde(default)]
    pub ask: AskSection,

    #[serde(default)]
    pub chat: ChatSection,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AskSection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSection {
    #[serde(default = "default_mcp_servers")]
    pub mcp_servers: Vec<String>,
    #[serde(default)]
    pub skills: Vec<PathBuf>,
    #[serde(default)]
    pub frozen_window: Option<usize>,
    #[serde(default)]
    pub worker_base: Option<String>,
    #[serde(default)]
    pub worktree_dir: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default = "default_true")]
    pub inbox_alert_sound: bool,
    #[serde(default)]
    pub inbox_alert_notification: bool,
    #[serde(default)]
    pub inbox_alert_unknown_focus: UnknownFocus,
}

fn default_true() -> bool {
    true
}

fn default_mcp_servers() -> Vec<String> {
    vec!["mcptools".to_string()]
}

impl Default for ChatSection {
    fn default() -> Self {
        Self {
            mcp_servers: default_mcp_servers(),
            skills: Vec::new(),
            frozen_window: None,
            worker_base: None,
            worktree_dir: None,
            permission_mode: None,
            inbox_alert_sound: true,
            inbox_alert_notification: false,
            inbox_alert_unknown_focus: UnknownFocus::Alert,
        }
    }
}

/// Theme configuration section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThemeSection {
    /// Theme name or file path.
    ///
    /// Can be either:
    /// - A built-in theme name (e.g., "Monokai Extended", "base16-ocean.dark")
    /// - A path to a .tmTheme file (e.g., "/path/to/theme.tmTheme")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Whether to disable background colors.
    ///
    /// When true, the theme's background colors are not applied,
    /// allowing the terminal's default background to show through.
    #[serde(default)]
    pub no_background: bool,
}

impl AppConfig {
    pub fn chat_permission_mode(&self, flag: Option<&str>, env: Option<&str>) -> String {
        resolve_permission_mode(
            flag,
            env,
            self.chat.permission_mode.as_deref(),
            CHAT_DEFAULT_PERMISSION_MODE,
        )
    }

    pub fn ask_permission_mode(&self, flag: Option<&str>, env: Option<&str>) -> String {
        resolve_permission_mode(
            flag,
            env,
            self.ask.permission_mode.as_deref(),
            ASK_DEFAULT_PERMISSION_MODE,
        )
    }

    /// Create a new empty configuration with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load configuration from the default config file path.
    ///
    /// Returns the loaded configuration if the file exists, or
    /// the default configuration if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be read or parsed.
    pub fn load() -> Result<Self, ConfigError> {
        let path = config_path();
        Self::load_from(&path)
    }

    /// Load configuration from a specific path.
    ///
    /// Returns the loaded configuration if the file exists, or
    /// the default configuration if it doesn't exist.
    pub fn load_from(path: &PathBuf) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(path).map_err(|e| ConfigError::ReadConfig {
            path: path.display().to_string(),
            source: e,
        })?;

        toml::from_str(&content).map_err(|e| ConfigError::ParseConfig {
            path: path.display().to_string(),
            source: e,
        })
    }

    /// Save configuration to the default config file path.
    ///
    /// Creates the parent directory if it doesn't exist.
    pub fn save(&self) -> Result<(), ConfigError> {
        let path = config_path();
        self.save_to(&path)
    }

    /// Save configuration to a specific path.
    ///
    /// Creates the parent directory if it doesn't exist.
    pub fn save_to(&self, path: &PathBuf) -> Result<(), ConfigError> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| ConfigError::WriteConfig {
                path: path.display().to_string(),
                source: e,
            })?;
        }

        let content = toml::to_string_pretty(self)?;
        fs::write(path, content).map_err(|e| ConfigError::WriteConfig {
            path: path.display().to_string(),
            source: e,
        })
    }
}

/// Get the path to the configuration file.
///
/// Uses platform-specific paths by default, which can be overridden
/// via the `RALPH_CONFIG_DIR` environment variable.
///
/// See [`crate::paths::config_path`] for details.
pub fn config_path() -> PathBuf {
    paths::config_path()
}

/// Get the path to the ralph config directory.
///
/// Uses platform-specific paths by default, which can be overridden
/// via the `RALPH_CONFIG_DIR` environment variable.
///
/// See [`crate::paths::config_dir`] for details.
pub fn config_dir() -> PathBuf {
    paths::config_dir()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = AppConfig::new();
        assert!(config.theme.name.is_none());
        assert!(!config.theme.no_background);
    }

    #[test]
    fn chat_section_defaults() {
        assert_eq!(AppConfig::new().chat.mcp_servers, ["mcptools"]);
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");
        fs::write(&path, "[chat]\n").unwrap();
        let config = AppConfig::load_from(&path).unwrap();
        assert_eq!(config.chat.mcp_servers, ["mcptools"]);
    }

    #[test]
    fn chat_section_parses_frozen_window() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");
        fs::write(&path, "[chat]\nfrozen_window = 16\n").unwrap();
        let config = AppConfig::load_from(&path).unwrap();
        assert_eq!(config.chat.frozen_window, Some(16));
        assert_eq!(AppConfig::new().chat.frozen_window, None);
    }

    #[test]
    fn chat_section_parses_inbox_alert() {
        let defaults = AppConfig::new().chat;
        assert!(defaults.inbox_alert_sound);
        assert!(!defaults.inbox_alert_notification);
        assert_eq!(defaults.inbox_alert_unknown_focus, UnknownFocus::Alert);
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");
        fs::write(
            &path,
            "[chat]\ninbox_alert_sound = false\ninbox_alert_notification = true\ninbox_alert_unknown_focus = \"silent\"\n",
        )
        .unwrap();
        let config = AppConfig::load_from(&path).unwrap();
        assert!(!config.chat.inbox_alert_sound);
        assert!(config.chat.inbox_alert_notification);
        assert_eq!(config.chat.inbox_alert_unknown_focus, UnknownFocus::Silent);
        fs::write(&path, "[chat]\ninbox_alert_unknown_focus = \"loud\"\n").unwrap();
        assert!(AppConfig::load_from(&path).is_err());
    }

    #[test]
    fn chat_section_parses_worker_base() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");
        fs::write(&path, "[chat]\nworker_base = \"develop\"\n").unwrap();
        let config = AppConfig::load_from(&path).unwrap();
        assert_eq!(config.chat.worker_base.as_deref(), Some("develop"));
        assert_eq!(AppConfig::new().chat.worker_base, None);
    }

    #[test]
    fn chat_section_parses_worktree_dir() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "[chat]\nworktree_dir = \"../wt\"\n").unwrap();
        let config = AppConfig::load_from(&path).unwrap();
        assert_eq!(config.chat.worktree_dir.as_deref(), Some("../wt"));
        assert_eq!(AppConfig::new().chat.worktree_dir, None);
    }

    #[test]
    fn chat_section_parses_mcp_servers() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");
        fs::write(&path, "[chat]\nmcp_servers = [\"a\", \"b\"]\n").unwrap();
        let config = AppConfig::load_from(&path).unwrap();
        assert_eq!(config.chat.mcp_servers, ["a", "b"]);
    }

    #[test]
    fn chat_section_parses_skills() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");
        fs::write(&path, "[chat]\nskills = [\"~/a\", \"/b\"]\n").unwrap();
        let config = AppConfig::load_from(&path).unwrap();
        assert_eq!(
            config.chat.skills,
            [PathBuf::from("~/a"), PathBuf::from("/b")]
        );
        assert_eq!(config.chat.mcp_servers, ["mcptools"]);
    }

    #[test]
    fn test_load_nonexistent_returns_default() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("nonexistent.toml");

        let config = AppConfig::load_from(&path).unwrap();
        assert!(config.theme.name.is_none());
        assert!(!config.theme.no_background);
    }

    #[test]
    fn test_load_empty_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");

        fs::write(&path, "").unwrap();

        let config = AppConfig::load_from(&path).unwrap();
        assert!(config.theme.name.is_none());
    }

    #[test]
    fn test_load_theme_name() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");

        fs::write(
            &path,
            r#"
            [theme]
            name = "Monokai Extended"
            "#,
        )
        .unwrap();

        let config = AppConfig::load_from(&path).unwrap();
        assert_eq!(config.theme.name, Some("Monokai Extended".to_string()));
        assert!(!config.theme.no_background);
    }

    #[test]
    fn test_load_theme_file_path() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");

        fs::write(
            &path,
            r#"
            [theme]
            name = "/path/to/my-theme.tmTheme"
            "#,
        )
        .unwrap();

        let config = AppConfig::load_from(&path).unwrap();
        assert_eq!(
            config.theme.name,
            Some("/path/to/my-theme.tmTheme".to_string())
        );
    }

    #[test]
    fn test_load_no_background() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");

        fs::write(
            &path,
            r#"
            [theme]
            no_background = true
            "#,
        )
        .unwrap();

        let config = AppConfig::load_from(&path).unwrap();
        assert!(config.theme.no_background);
    }

    #[test]
    fn test_load_full_config() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");

        fs::write(
            &path,
            r#"
            [theme]
            name = "base16-eighties.dark"
            no_background = true
            "#,
        )
        .unwrap();

        let config = AppConfig::load_from(&path).unwrap();
        assert_eq!(config.theme.name, Some("base16-eighties.dark".to_string()));
        assert!(config.theme.no_background);
    }

    #[test]
    fn test_save_and_load() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("subdir").join("config.toml");

        let config = AppConfig {
            theme: ThemeSection {
                name: Some("Monokai Extended".to_string()),
                no_background: true,
            },
            ask: AskSection::default(),
            chat: ChatSection::default(),
        };

        config.save_to(&path).unwrap();

        let loaded = AppConfig::load_from(&path).unwrap();
        assert_eq!(loaded.theme.name, Some("Monokai Extended".to_string()));
        assert!(loaded.theme.no_background);
    }

    #[test]
    fn test_parse_error() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");

        fs::write(&path, "invalid toml {{{{").unwrap();

        let result = AppConfig::load_from(&path);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ConfigError::ParseConfig { .. }));
    }

    #[test]
    fn test_unknown_fields_ignored() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");

        fs::write(
            &path,
            r#"
            [theme]
            name = "Monokai"
            unknown_field = "ignored"

            [unknown_section]
            foo = "bar"
            "#,
        )
        .unwrap();

        // Should not error on unknown fields
        let config = AppConfig::load_from(&path).unwrap();
        assert_eq!(config.theme.name, Some("Monokai".to_string()));
    }

    #[test]
    fn test_serialize_omits_none() {
        let config = AppConfig {
            theme: ThemeSection {
                name: None,
                no_background: false,
            },
            ask: AskSection::default(),
            chat: ChatSection::default(),
        };

        let content = toml::to_string_pretty(&config).unwrap();
        // Should not contain "name" since it's None
        assert!(!content.contains("name"));
    }

    #[test]
    fn test_config_path_contains_ralph() {
        let path = config_path();
        assert!(path.to_string_lossy().contains("ralph"));
        assert!(path.to_string_lossy().ends_with("config.toml"));
    }

    #[test]
    fn test_default_permission_modes() {
        let config = AppConfig::new();
        assert!(config.chat.permission_mode.is_none());
        assert!(config.ask.permission_mode.is_none());
    }

    #[test]
    fn test_load_chat_and_ask_permission_modes_separately() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");

        fs::write(
            &path,
            r#"
            [chat]
            permission_mode = "plan"

            [ask]
            permission_mode = "acceptEdits"
            "#,
        )
        .unwrap();

        let config = AppConfig::load_from(&path).unwrap();
        assert_eq!(config.chat.permission_mode, Some("plan".to_string()));
        assert_eq!(config.ask.permission_mode, Some("acceptEdits".to_string()));
    }

    fn configured(chat: Option<&str>, ask: Option<&str>) -> AppConfig {
        let mut config = AppConfig::new();
        config.chat.permission_mode = chat.map(str::to_string);
        config.ask.permission_mode = ask.map(str::to_string);
        config
    }

    #[test]
    fn chat_permission_mode_precedence() {
        let config = configured(Some("plan"), Some("dontAsk"));
        assert_eq!(
            config.chat_permission_mode(Some("manual"), Some("acceptEdits")),
            "manual"
        );
        assert_eq!(
            config.chat_permission_mode(None, Some("acceptEdits")),
            "acceptEdits"
        );
        assert_eq!(config.chat_permission_mode(None, Some("")), "plan");
        assert_eq!(config.chat_permission_mode(None, None), "plan");
        assert_eq!(
            configured(None, Some("dontAsk")).chat_permission_mode(None, None),
            "auto"
        );
    }

    #[test]
    fn ask_permission_mode_precedence() {
        let config = configured(Some("plan"), Some("dontAsk"));
        assert_eq!(
            config.ask_permission_mode(Some("manual"), Some("acceptEdits")),
            "manual"
        );
        assert_eq!(
            config.ask_permission_mode(None, Some("acceptEdits")),
            "acceptEdits"
        );
        assert_eq!(config.ask_permission_mode(None, Some("")), "dontAsk");
        assert_eq!(config.ask_permission_mode(None, None), "dontAsk");
        assert_eq!(
            configured(Some("plan"), None).ask_permission_mode(None, None),
            "bypassPermissions"
        );
    }
}
