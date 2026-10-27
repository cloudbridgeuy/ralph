//! Loads `config.toml`. Locations, precedence, and keys are in `docs/configuration.md`.

use crate::input_theme::InputTheme;
use crate::invoke::{
    resolve_permission_mode, ASK_DEFAULT_PERMISSION_MODE, CHAT_DEFAULT_PERMISSION_MODE,
};
use crate::markdown::{RoleTheme, ThemeRole};
use crate::paths;
use crate::status_theme::StatusTheme;
use crate::warn::warn_once;
use ralph_core::chat::WORKER_QUIET_SECS;
use ralph_core::inbox_alert::UnknownFocus;
use ralph_core::worker::TicketKeys;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use toml::Table;

mod changes;
mod check;
pub use check::{check, hard_errors};
mod chat_settings;
mod claude;
mod codex;
mod command;
mod cursor;
mod empty_submit;
mod layers;
mod opencode;
mod permissions;
#[cfg(test)]
mod recent_history_tests;
#[cfg(test)]
mod reset_limits_tests;
mod wire;
mod workers;
pub use changes::ChangesSection;
pub use chat_settings::PromptFiles;
pub use claude::ClaudeSection;
pub use codex::{configured_codex, CodexSection};
pub use command::{run_check, run_show};
pub use cursor::CursorSection;
pub use layers::{leaf_sources, Source};
pub use opencode::OpencodeSection;
pub use permissions::configured_permission_mode;
pub use wire::WireSection;
pub use workers::{PrProvider, WorkersSection};

#[derive(Debug, PartialEq, Eq)]
struct Notice {
    key: String,
    message: String,
}

fn security_notices(store: &Path) -> Vec<Notice> {
    let Ok(table) = read_layer(store) else {
        return Vec::new();
    };
    layers::security_keys(&table)
        .into_iter()
        .map(|key| Notice {
            key: format!("security:{key}"),
            message: format!(
                "store config {} sets {key}; check it before you trust this store",
                store.display()
            ),
        })
        .collect()
}

static STORE_LAYER: OnceLock<PathBuf> = OnceLock::new();

pub fn set_store_layer(store: &Path) {
    let _ = STORE_LAYER.set(store.to_path_buf());
}

fn parse_error(path: &Path, source: toml::de::Error) -> ConfigError {
    ConfigError::ParseConfig {
        path: path.display().to_string(),
        source,
    }
}

fn read_layer(path: &Path) -> Result<Table, ConfigError> {
    if !path.exists() {
        return Ok(Table::new());
    }
    let content = fs::read_to_string(path).map_err(|e| ConfigError::ReadConfig {
        path: path.display().to_string(),
        source: e,
    })?;
    let table = content
        .parse::<Table>()
        .map_err(|source| parse_error(path, source))?;
    Ok(match path.parent() {
        Some(dir) => layers::anchor_chat_paths(table, dir),
        None => table,
    })
}

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
    pub prompt_file: Option<PathBuf>,
    #[serde(default)]
    pub append_prompt_files: Vec<PathBuf>,
    #[serde(default = "default_true")]
    pub project_claude_md: bool,
    #[serde(default)]
    pub user_settings: bool,
    #[serde(default)]
    pub user_claude_md: bool,
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
            prompt_file: None,
            append_prompt_files: Vec::new(),
            project_claude_md: true,
            user_settings: false,
            user_claude_md: false,
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

    pub fn deprecations(&self) -> Vec<String> {
        match &self.agent {
            Some(AgentTheme::Legacy(code)) => vec![format!(
                "[theme] agent = \"{code}\" is deprecated; move it to [theme.agent] code = \"{code}\""
            )],
            _ => Vec::new(),
        }
    }

    pub fn warnings(&self) -> Vec<String> {
        self.assistant
            .palette(ThemeRole::Assistant)
            .1
            .into_iter()
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

    pub fn load() -> Result<Self, ConfigError> {
        let store_file = STORE_LAYER.get().map(|dir| dir.join("config.toml"));
        let (result, notices) = Self::load_from(&config_path(), store_file.as_deref());
        for notice in &notices {
            warn_once(&notice.key, &notice.message);
        }
        result
    }

    fn load_from(global: &Path, store: Option<&Path>) -> (Result<Self, ConfigError>, Vec<Notice>) {
        match (Self::load_layers(global, store), store) {
            (Err(store_error), Some(store_path)) => match Self::load_layers(global, None) {
                Ok(config) => {
                    let notice = Notice {
                        key: format!("store-fallback:{}", store_path.display()),
                        message: format!(
                            "ignoring store config {}: {store_error}; using the global config only",
                            store_path.display()
                        ),
                    };
                    (Ok(config), vec![notice])
                }
                Err(global_error) => (Err(global_error), Vec::new()),
            },
            (Ok(config), Some(store_path)) => (Ok(config), security_notices(store_path)),
            (result, _) => (result, Vec::new()),
        }
    }

    pub fn load_layers(global: &Path, store: Option<&Path>) -> Result<Self, ConfigError> {
        let layers = [Some(global), store]
            .into_iter()
            .flatten()
            .map(|path| read_layer(path).map(|table| (path, table)))
            .collect::<Result<Vec<_>, _>>()?;
        let merged = layers
            .iter()
            .cloned()
            .fold(Table::new(), |merged, (_, layer)| {
                layers::merge_tables(merged, layer)
            });
        merged
            .try_into::<Self>()
            .map_err(|source| {
                let culprit = layers.iter().find_map(|(path, layer)| {
                    layer.clone().try_into::<Self>().err().map(|e| (*path, e))
                });
                match culprit {
                    Some((path, source)) => parse_error(path, source),
                    None => parse_error(global, source),
                }
            })?
            .validated()
    }

    pub fn parse(content: &str, path: &str) -> Result<Self, ConfigError> {
        toml::from_str::<Self>(content)
            .map_err(|source| ConfigError::ParseConfig {
                path: path.to_string(),
                source,
            })?
            .validated()
    }

    pub fn parse_reporting_unknown(
        content: &str,
        path: &str,
    ) -> Result<(Self, Vec<String>), ConfigError> {
        let parse_error = |source| ConfigError::ParseConfig {
            path: path.to_string(),
            source,
        };
        let mut unknown = Vec::new();
        let deserializer = toml::de::Deserializer::parse(content).map_err(parse_error)?;
        let config: Self =
            serde_ignored::deserialize(deserializer, |key| unknown.push(key.to_string()))
                .map_err(parse_error)?;
        Ok((config.validated()?, unknown))
    }

    fn validated(self) -> Result<Self, ConfigError> {
        ralph_core::wire::price::validate_price_overrides(&self.wire.prices)
            .map_err(ConfigError::InvalidWirePrice)?;
        self.workers
            .validate()
            .map_err(ConfigError::InvalidWorkers)?;
        Ok(self)
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
