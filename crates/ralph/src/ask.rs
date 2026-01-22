//! Ask command implementation.
//!
//! Provides a single-shot LLM prompt command that invokes claude subprocess
//! and displays the response with syntax highlighting, with optional session
//! persistence for replay.
//!
//! # Architecture
//!
//! This module follows the Functional Core - Imperative Shell pattern:
//! - Pure functions: command building, config validation
//! - Imperative shell: subprocess invocation, output display, session I/O
//!
//! # Example
//!
//! ```no_run
//! use ralph::ask::{ask, AskConfig, AskError};
//! use std::path::PathBuf;
//!
//! let config = AskConfig {
//!     prompt: "What is 2+2?".to_string(),
//!     project_path: PathBuf::from("/path/to/project"),
//!     ..Default::default()
//! };
//!
//! ask(config)?;
//! # Ok::<(), AskError>(())
//! ```

use crate::highlight::ThemeConfig;
use crate::session::{self, SessionError};
use crate::spinner::SpinnerSessionInfo;
use crate::stream_processor::VerboseToolsConfig;
use crate::subprocess::{
    invoke_subprocess_with_spinner_config, SpinnerSubprocessConfig, SubprocessError,
};
use std::path::PathBuf;

/// Default timeout for ask command (10 minutes).
pub const DEFAULT_TIMEOUT_SECS: u64 = 600;

/// Configuration for the ask command.
///
/// Groups all parameters needed to execute a single-shot LLM prompt.
#[derive(Debug, Clone)]
pub struct AskConfig {
    /// The prompt to send to the LLM.
    pub prompt: String,
    /// Maximum duration in seconds before timing out.
    pub timeout_secs: u64,
    /// Configuration for syntax highlighting theme.
    pub theme_config: ThemeConfig,
    /// Configuration for verbose tool output.
    pub verbose_tools: VerboseToolsConfig,
    /// Absolute path to the project directory.
    pub project_path: PathBuf,
    /// Optional user-provided session slug.
    pub slug: Option<String>,
}

impl Default for AskConfig {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            theme_config: ThemeConfig::new(),
            verbose_tools: VerboseToolsConfig::new(),
            project_path: PathBuf::new(),
            slug: None,
        }
    }
}

/// Error type for ask command operations.
#[derive(Debug, thiserror::Error)]
pub enum AskError {
    /// No prompt was provided.
    #[error("No prompt provided. Use: ralph ask 'your prompt here'")]
    NoPrompt,

    /// Subprocess invocation failed.
    #[error("Subprocess failed: {0}")]
    Subprocess(#[from] SubprocessError),

    /// LLM returned non-zero exit code.
    #[error("LLM subprocess exited with code {exit_code}")]
    NonZeroExit { exit_code: i32 },

    /// Session initialization failed.
    #[error("Session initialization failed: {0}")]
    Session(#[from] SessionError),
}

/// Build the claude command string for the ask command.
///
/// Constructs the command with appropriate flags:
/// - `--verbose` for detailed output
/// - `--permission-mode acceptEdits` for non-interactive mode
/// - `--output-format stream-json` for stream processing
/// - `-p` for the prompt
///
/// # Arguments
///
/// * `prompt` - The prompt to send to the LLM
///
/// # Returns
///
/// The full command string to execute.
fn build_command(prompt: &str) -> String {
    let escaped = prompt.replace('\'', "'\"'\"'");
    format!(
        "claude --verbose --permission-mode acceptEdits --output-format stream-json -p '{}'",
        escaped
    )
}

/// Result of a successful ask command execution.
#[derive(Debug)]
pub struct AskResult {
    /// The session slug that was created.
    pub slug: String,
}

/// Execute a single-shot LLM prompt and display the response.
///
/// This is the main entry point for the ask command. It:
/// 1. Validates the configuration
/// 2. Creates a session for persistence
/// 3. Builds the claude command
/// 4. Invokes the subprocess with spinner display
/// 5. Returns the result with session information
///
/// # Arguments
///
/// * `config` - Configuration for the ask command
///
/// # Returns
///
/// `Ok(AskResult)` on success with session information, or an `AskError` on failure.
///
/// # Example
///
/// ```no_run
/// use ralph::ask::{ask, AskConfig};
/// use std::path::PathBuf;
///
/// let config = AskConfig {
///     prompt: "What is 2+2?".to_string(),
///     project_path: PathBuf::from("/path/to/project"),
///     ..Default::default()
/// };
///
/// let result = ask(config)?;
/// println!("Session created: {}", result.slug);
/// # Ok::<(), AskError>(())
/// ```
pub fn ask(config: AskConfig) -> Result<AskResult, AskError> {
    // Validate prompt is not empty
    if config.prompt.is_empty() {
        return Err(AskError::NoPrompt);
    }

    // Initialize session for persistence
    let (slug, session_dir) = session::initialize_session(
        config.slug.as_deref(),
        &config.project_path,
        Some(config.prompt.clone()),
    )?;

    // Build the command
    let command = build_command(&config.prompt);

    // Build subprocess config with session info for spinner display
    let subprocess_config = SpinnerSubprocessConfig {
        command,
        timeout_secs: config.timeout_secs,
        theme_config: config.theme_config,
        session_elapsed_ms: 0,
        verbose_tools: config.verbose_tools,
        session_info: SpinnerSessionInfo {
            slug: Some(slug.clone()),
            current_iteration: Some(1),
            max_iterations: Some(1),
        },
    };

    // Invoke subprocess with spinner
    let result = invoke_subprocess_with_spinner_config(&subprocess_config)?;

    // Check exit code
    if result.exit_code != 0 {
        return Err(AskError::NonZeroExit {
            exit_code: result.exit_code,
        });
    }

    // session_dir will be used in future stories for iteration logging
    let _ = session_dir;

    Ok(AskResult { slug })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_command_simple() {
        let cmd = build_command("hello");
        assert_eq!(
            cmd,
            "claude --verbose --permission-mode acceptEdits --output-format stream-json -p 'hello'"
        );
    }

    #[test]
    fn test_build_command_with_quotes() {
        let cmd = build_command("it's a test");
        assert_eq!(
            cmd,
            "claude --verbose --permission-mode acceptEdits --output-format stream-json -p 'it'\"'\"'s a test'"
        );
    }

    #[test]
    fn test_build_command_multiline() {
        let cmd = build_command("line1\nline2");
        assert_eq!(
            cmd,
            "claude --verbose --permission-mode acceptEdits --output-format stream-json -p 'line1\nline2'"
        );
    }

    #[test]
    fn test_build_command_shell_special_chars() {
        let cmd = build_command("test $VAR `cmd`");
        assert_eq!(
            cmd,
            "claude --verbose --permission-mode acceptEdits --output-format stream-json -p 'test $VAR `cmd`'"
        );
    }

    #[test]
    fn test_ask_config_default() {
        let config = AskConfig::default();
        assert!(config.prompt.is_empty());
        assert_eq!(config.timeout_secs, DEFAULT_TIMEOUT_SECS);
        assert!(config.project_path.as_os_str().is_empty());
        assert!(config.slug.is_none());
    }

    #[test]
    fn test_ask_config_with_session_fields() {
        let config = AskConfig {
            prompt: "test".to_string(),
            project_path: PathBuf::from("/test/project"),
            slug: Some("my-session".to_string()),
            ..Default::default()
        };

        assert_eq!(config.prompt, "test");
        assert_eq!(config.project_path, PathBuf::from("/test/project"));
        assert_eq!(config.slug, Some("my-session".to_string()));
    }

    #[test]
    fn test_ask_error_no_prompt() {
        let config = AskConfig::default();
        let result = ask(config);
        assert!(matches!(result, Err(AskError::NoPrompt)));
    }

    #[test]
    fn test_ask_result_fields() {
        // Test that AskResult can be constructed with the expected fields
        let result = AskResult {
            slug: "test-slug".to_string(),
        };
        assert_eq!(result.slug, "test-slug");
    }

    #[test]
    fn test_ask_error_display() {
        let error = AskError::NoPrompt;
        let msg = format!("{}", error);
        assert!(msg.contains("No prompt provided"));

        let error = AskError::NonZeroExit { exit_code: 1 };
        let msg = format!("{}", error);
        assert!(msg.contains("exit"));
        assert!(msg.contains("1"));
    }
}
