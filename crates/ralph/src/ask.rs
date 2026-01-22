//! Ask command implementation.
//!
//! Provides a single-shot LLM prompt command that invokes claude subprocess
//! and displays the response with syntax highlighting.
//!
//! # Architecture
//!
//! This module follows the Functional Core - Imperative Shell pattern:
//! - Pure functions: command building, config validation
//! - Imperative shell: subprocess invocation, output display
//!
//! # Example
//!
//! ```no_run
//! use ralph::ask::{ask, AskConfig, AskError};
//!
//! let config = AskConfig {
//!     prompt: "What is 2+2?".to_string(),
//!     ..Default::default()
//! };
//!
//! ask(config)?;
//! # Ok::<(), AskError>(())
//! ```

use crate::highlight::ThemeConfig;
use crate::spinner::SpinnerSessionInfo;
use crate::stream_processor::VerboseToolsConfig;
use crate::subprocess::{
    invoke_subprocess_with_spinner_config, SpinnerSubprocessConfig, SubprocessError,
};

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
}

impl Default for AskConfig {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            theme_config: ThemeConfig::new(),
            verbose_tools: VerboseToolsConfig::new(),
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

/// Execute a single-shot LLM prompt and display the response.
///
/// This is the main entry point for the ask command. It:
/// 1. Validates the configuration
/// 2. Builds the claude command
/// 3. Invokes the subprocess with spinner display
/// 4. Returns the result
///
/// # Arguments
///
/// * `config` - Configuration for the ask command
///
/// # Returns
///
/// `Ok(())` on success, or an `AskError` on failure.
///
/// # Example
///
/// ```no_run
/// use ralph::ask::{ask, AskConfig};
///
/// let config = AskConfig {
///     prompt: "What is 2+2?".to_string(),
///     ..Default::default()
/// };
///
/// ask(config)?;
/// # Ok::<(), AskError>(())
/// ```
pub fn ask(config: AskConfig) -> Result<(), AskError> {
    // Validate prompt is not empty
    if config.prompt.is_empty() {
        return Err(AskError::NoPrompt);
    }

    // Build the command
    let command = build_command(&config.prompt);

    // Build subprocess config
    let subprocess_config = SpinnerSubprocessConfig {
        command,
        timeout_secs: config.timeout_secs,
        theme_config: config.theme_config,
        session_elapsed_ms: 0,
        verbose_tools: config.verbose_tools,
        session_info: SpinnerSessionInfo::default(),
    };

    // Invoke subprocess with spinner
    let result = invoke_subprocess_with_spinner_config(&subprocess_config)?;

    // Check exit code
    if result.exit_code != 0 {
        return Err(AskError::NonZeroExit {
            exit_code: result.exit_code,
        });
    }

    Ok(())
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
    }

    #[test]
    fn test_ask_error_no_prompt() {
        let config = AskConfig::default();
        let result = ask(config);
        assert!(matches!(result, Err(AskError::NoPrompt)));
    }
}
