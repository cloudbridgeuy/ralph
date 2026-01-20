//! Centralized path resolution for ralph configuration and data directories.
//!
//! This module provides cross-platform path resolution following platform conventions:
//! - **Linux**: `~/.config/ralph` (XDG Base Directory spec via `dirs` crate)
//! - **macOS**: `~/Library/Application Support/ralph`
//! - **Windows**: `%APPDATA%\ralph`
//!
//! # Environment Variable Overrides
//!
//! The default paths can be overridden using environment variables:
//! - `RALPH_CONFIG_DIR`: Override config directory (config.toml location)
//! - `RALPH_DATA_DIR`: Override data directory (sessions storage)
//!
//! # Path Resolution Precedence
//!
//! 1. Environment variable (if set and non-empty)
//! 2. Platform-specific default (via `dirs` crate)
//! 3. Fallback to current directory (if platform resolution fails)
//!
//! # Example
//!
//! ```no_run
//! use ralph::paths::{config_dir, data_dir, config_path, sessions_dir, session_dir};
//!
//! // Get configuration directory
//! let config = config_dir();  // e.g., ~/.config/ralph on Linux
//!
//! // Get data directory (can be different on some platforms)
//! let data = data_dir();  // e.g., ~/.local/share/ralph on Linux
//!
//! // Get specific paths
//! let config_file = config_path();  // ~/.config/ralph/config.toml
//! let sessions = sessions_dir();    // ~/.local/share/ralph/sessions
//! let session = session_dir("quiet-mountain");  // ~/.local/share/ralph/sessions/quiet-mountain
//! ```

use std::path::PathBuf;

/// Environment variable to override the configuration directory.
pub const RALPH_CONFIG_DIR_ENV: &str = "RALPH_CONFIG_DIR";

/// Environment variable to override the data directory (sessions storage).
pub const RALPH_DATA_DIR_ENV: &str = "RALPH_DATA_DIR";

/// Get the ralph configuration directory.
///
/// Resolution order:
/// 1. `RALPH_CONFIG_DIR` environment variable (if set and non-empty)
/// 2. Platform-specific config directory:
///    - Linux: `$XDG_CONFIG_HOME/ralph` or `~/.config/ralph`
///    - macOS: `~/Library/Application Support/ralph`
///    - Windows: `%APPDATA%\ralph`
/// 3. Fallback: `./ralph` (current directory)
///
/// # Example
///
/// ```no_run
/// use ralph::paths::config_dir;
///
/// let config = config_dir();
/// println!("Config directory: {}", config.display());
/// ```
pub fn config_dir() -> PathBuf {
    // Check environment variable override first
    if let Some(path) = env_override(RALPH_CONFIG_DIR_ENV) {
        return path;
    }

    // Use platform-specific config directory
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ralph")
}

/// Get the ralph data directory.
///
/// By default, data (sessions) is stored alongside configuration. However,
/// this can be overridden with `RALPH_DATA_DIR` for users who want to store
/// sessions in a different location (e.g., a larger disk or shared location).
///
/// Resolution order:
/// 1. `RALPH_DATA_DIR` environment variable (if set and non-empty)
/// 2. Platform-specific data directory:
///    - Linux: `$XDG_DATA_HOME/ralph` or `~/.local/share/ralph`
///    - macOS: `~/Library/Application Support/ralph`
///    - Windows: `%APPDATA%\ralph`
/// 3. Fallback: same as config directory
///
/// # Example
///
/// ```no_run
/// use ralph::paths::data_dir;
///
/// let data = data_dir();
/// println!("Data directory: {}", data.display());
/// ```
pub fn data_dir() -> PathBuf {
    // Check environment variable override first
    if let Some(path) = env_override(RALPH_DATA_DIR_ENV) {
        return path;
    }

    // Use platform-specific data directory
    // On macOS, data_dir returns ~/Library/Application Support (same as config)
    // On Linux, data_dir returns ~/.local/share (different from config)
    // On Windows, data_dir returns %APPDATA% (same as config)
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ralph")
}

/// Get the path to the configuration file.
///
/// Returns `{config_dir}/config.toml`.
///
/// # Example
///
/// ```no_run
/// use ralph::paths::config_path;
///
/// let path = config_path();
/// // ~/.config/ralph/config.toml on Linux
/// ```
pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Get the path to the global sessions directory.
///
/// Returns `{data_dir}/sessions`.
///
/// # Example
///
/// ```no_run
/// use ralph::paths::sessions_dir;
///
/// let path = sessions_dir();
/// // ~/.local/share/ralph/sessions on Linux
/// ```
pub fn sessions_dir() -> PathBuf {
    data_dir().join("sessions")
}

/// Get the path to the global sessions index file.
///
/// Returns `{data_dir}/sessions.toml`.
///
/// # Example
///
/// ```no_run
/// use ralph::paths::sessions_index_path;
///
/// let path = sessions_index_path();
/// // ~/.local/share/ralph/sessions.toml on Linux
/// ```
pub fn sessions_index_path() -> PathBuf {
    data_dir().join("sessions.toml")
}

/// Get the path to a specific session directory.
///
/// Returns `{sessions_dir}/{slug}`.
///
/// # Example
///
/// ```no_run
/// use ralph::paths::session_dir;
///
/// let path = session_dir("quiet-mountain");
/// // ~/.local/share/ralph/sessions/quiet-mountain on Linux
/// ```
pub fn session_dir(slug: &str) -> PathBuf {
    sessions_dir().join(slug)
}

/// Helper function to get an environment variable override path.
///
/// Returns `Some(PathBuf)` if the environment variable is set and non-empty,
/// otherwise returns `None`.
fn env_override(var_name: &str) -> Option<PathBuf> {
    std::env::var(var_name)
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Get debug information about resolved paths.
///
/// Returns a formatted string showing all resolved paths and their sources.
/// Useful for `--verbose` output or debugging configuration issues.
pub fn debug_paths() -> String {
    let mut info = String::new();

    // Config directory
    let config_env = std::env::var(RALPH_CONFIG_DIR_ENV).ok();
    let config = config_dir();
    info.push_str(&format!("Config directory: {}\n", config.display()));
    if let Some(env_val) = &config_env {
        info.push_str(&format!("  (from {}={})\n", RALPH_CONFIG_DIR_ENV, env_val));
    } else {
        info.push_str("  (platform default)\n");
    }

    // Data directory
    let data_env = std::env::var(RALPH_DATA_DIR_ENV).ok();
    let data = data_dir();
    info.push_str(&format!("Data directory: {}\n", data.display()));
    if let Some(env_val) = &data_env {
        info.push_str(&format!("  (from {}={})\n", RALPH_DATA_DIR_ENV, env_val));
    } else {
        info.push_str("  (platform default)\n");
    }

    // Specific paths
    info.push_str(&format!("Config file: {}\n", config_path().display()));
    info.push_str(&format!(
        "Sessions index: {}\n",
        sessions_index_path().display()
    ));
    info.push_str(&format!(
        "Sessions directory: {}\n",
        sessions_dir().display()
    ));

    info
}
