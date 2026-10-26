//! Loads `config.toml`. Locations, precedence, and keys are in `docs/configuration.md`.

use crate::input_theme::InputTheme;
use crate::invoke::{
    resolve_permission_mode, ASK_DEFAULT_PERMISSION_MODE, CHAT_DEFAULT_PERMISSION_MODE,
};
use crate::markdown::{RoleTheme, ThemeRole};
use crate::paths;
use crate::status_theme::StatusTheme;
use ralph_core::chat::WORKER_QUIET_SECS;
use ralph_core::inbox_alert::UnknownFocus;
use ralph_core::worker::TicketKeys;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

mod changes;
mod chat_settings;
mod claude;
mod codex;
mod cursor;
mod empty_submit;
mod opencode;
mod permissions;
#[cfg(test)]
mod recent_history_tests;
#[cfg(test)]
mod reset_limits_tests;
mod wire;
mod workers;
pub use changes::ChangesSection;
pub use claude::ClaudeSection;
pub use codex::{configured_codex, CodexSection};
pub use cursor::CursorSection;
pub use opencode::OpencodeSection;
pub use permissions::configured_permission_mode;
pub use wire::WireSection;
pub use workers::{PrProvider, WorkersSection};

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

    /// A `[wire.prices]` value is below zero.
    #[error("{0}")]
    InvalidWirePrice(String),

    #[error("{0}")]
    InvalidWorkers(String),
}

/// Application configuration.
///
/// This struct represents the full configuration that can be stored
/// in the config file. All fields are optional to support partial
/// configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub theme: ThemeSection,

    #[serde(default)]
    pub ask: AskSection,

    #[serde(default)]
    pub chat: ChatSection,

    #[serde(default)]
    pub mermaid: MermaidSection,

    #[serde(default)]
    pub cursor: CursorSection,

    #[serde(default)]
    pub codex: CodexSection,

    #[serde(default)]
    pub claude: ClaudeSection,

    #[serde(default)]
    pub opencode: OpencodeSection,

    #[serde(default)]
    pub changes: ChangesSection,

    #[serde(default)]
    pub wire: WireSection,

    #[serde(default)]
    pub tickets: TicketKeys,

    #[serde(default)]
    pub workers: WorkersSection,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MermaidSection {
    #[serde(default = "default_mermaid_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_mermaid_max_rss_mb")]
    pub max_rss_mb: u64,
    #[serde(default = "default_mermaid_max_input_bytes")]
    pub max_input_bytes: usize,
    #[serde(default = "default_mermaid_max_flowchart_edges")]
    pub max_flowchart_edges: usize,
    #[serde(default = "default_mermaid_max_class_relations")]
    pub max_class_relations: usize,
}

fn default_mermaid_timeout_ms() -> u64 {
    500
}

fn default_mermaid_max_rss_mb() -> u64 {
    256
}

fn default_mermaid_max_input_bytes() -> usize {
    1_048_576
}

fn default_mermaid_max_flowchart_edges() -> usize {
    50
}

fn default_mermaid_max_class_relations() -> usize {
    400
}

impl Default for MermaidSection {
    fn default() -> Self {
        Self {
            timeout_ms: default_mermaid_timeout_ms(),
            max_rss_mb: default_mermaid_max_rss_mb(),
            max_input_bytes: default_mermaid_max_input_bytes(),
            max_flowchart_edges: default_mermaid_max_flowchart_edges(),
            max_class_relations: default_mermaid_max_class_relations(),
        }
    }
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
    pub recent_history_tokens: Option<usize>,
    #[serde(default)]
    pub worker_check_secs: Option<u64>,
    #[serde(default)]
    pub worker_quiet_secs: Option<toml::Value>,
    #[serde(default)]
    pub reset_turns: Option<toml::Value>,
    #[serde(default)]
    pub reset_tokens: Option<toml::Value>,
    #[serde(default)]
    pub watch_timeout_secs: Option<u64>,
    #[serde(default)]
    pub wrap_width: Option<usize>,
    #[serde(default)]
    pub cell_output_limit: Option<usize>,
    #[serde(default)]
    pub cell_output_bytes: Option<usize>,
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
    #[serde(default)]
    pub status_powerline: bool,
    #[serde(default)]
    pub editor: Option<EditorSetting>,
    #[serde(default)]
    pub edit_mode: Option<String>,
    #[serde(default)]
    pub empty_submit: Option<String>,
    #[serde(default)]
    pub empty_prompt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EditorSetting {
    Argv(Vec<String>),
    Invalid(toml::Value),
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
            recent_history_tokens: None,
            worker_check_secs: None,
            worker_quiet_secs: None,
            reset_turns: None,
            reset_tokens: None,
            watch_timeout_secs: None,
            wrap_width: None,
            cell_output_limit: None,
            cell_output_bytes: None,
            worker_base: None,
            worktree_dir: None,
            permission_mode: None,
            inbox_alert_sound: true,
            inbox_alert_notification: false,
            inbox_alert_unknown_focus: UnknownFocus::Alert,
            status_powerline: false,
            editor: None,
            edit_mode: None,
            empty_submit: None,
            empty_prompt: None,
        }
    }
}

/// Theme configuration section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThemeSection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    #[serde(default)]
    pub no_background: bool,

    #[serde(default)]
    pub assistant: RoleTheme,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentTheme>,

    #[serde(default)]
    pub status: StatusTheme,

    #[serde(default)]
    pub input: InputTheme,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AgentTheme {
    Legacy(String),
    Role(RoleTheme),
}

impl ThemeSection {
    pub fn agent(&self) -> RoleTheme {
        match &self.agent {
            None => RoleTheme::default(),
            Some(AgentTheme::Legacy(code)) => RoleTheme {
                code: Some(code.clone()),
                ..RoleTheme::default()
            },
            Some(AgentTheme::Role(role)) => role.clone(),
        }
    }

    pub fn warnings(&self) -> Vec<String> {
        let legacy = match &self.agent {
            Some(AgentTheme::Legacy(code)) => Some(format!(
                "[theme] agent = \"{code}\" is deprecated; move it to [theme.agent] code = \"{code}\""
            )),
            _ => None,
        };
        legacy
            .into_iter()
            .chain(self.assistant.palette(ThemeRole::Assistant).1)
            .chain(self.agent().palette(ThemeRole::Agent).1)
            .chain(self.status.resolve().1)
            .chain(self.input.resolve().1)
            .collect()
    }
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

        let config: Self = toml::from_str(&content).map_err(|e| ConfigError::ParseConfig {
            path: path.display().to_string(),
            source: e,
        })?;
        ralph_core::wire::price::validate_price_overrides(&config.wire.prices)
            .map_err(ConfigError::InvalidWirePrice)?;
        config
            .workers
            .validate()
            .map_err(ConfigError::InvalidWorkers)?;
        Ok(config)
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
mod tests;
