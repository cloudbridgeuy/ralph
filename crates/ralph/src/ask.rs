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
use crate::iteration::{
    write_iteration_log, Chunk, IterationError, IterationLog, LogMetadata, LogToolCall,
};
use crate::session::{self, SessionError};
use crate::spinner::SpinnerSessionInfo;
use crate::stream_processor::VerboseToolsConfig;
use crate::subprocess::{
    invoke_subprocess_with_spinner_config, SpinnerSubprocessConfig, StreamingSubprocessResult,
    SubprocessError,
};
use chrono::{DateTime, Utc};
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

    /// Iteration log write failed.
    #[error("Failed to write iteration log: {0}")]
    Iteration(#[from] IterationError),
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

/// Build an iteration log from subprocess result.
///
/// This is a pure function that extracts metadata, tool calls, chunks,
/// and output blocks from the subprocess result to create an IterationLog.
///
/// # Arguments
///
/// * `result` - The subprocess result containing stream processing data
/// * `started_at` - When the iteration started
/// * `completed_at` - When the iteration completed
///
/// # Returns
///
/// An `IterationLog` populated with sequence 1, timestamps, exit code,
/// and extracted metadata, tool calls, chunks, and output blocks.
fn build_iteration_log(
    result: &StreamingSubprocessResult,
    started_at: DateTime<Utc>,
    completed_at: DateTime<Utc>,
) -> IterationLog {
    // Build metadata from stream processing result
    let metadata = LogMetadata::from_extracted(
        result.stream_result.metadata.clone(),
        result.stream_result.costs.clone(),
    );

    // Build tool calls from stream processing result
    let tool_calls = LogToolCall::from_interactions(&result.stream_result.tool_interactions);

    // Convert parsed chunks to iteration log chunks
    let chunks = Chunk::from_parsed_chunks(&result.stream_result.chunks);

    IterationLog {
        sequence: 1, // Ask command always creates iteration 1
        started_at,
        completed_at,
        exit_code: result.exit_code,
        pending_before: 0, // Ask command has no stories
        pending_after: 0,  // Ask command has no stories
        metadata: metadata.into_option(),
        tool_calls,
        chunks,
        output_blocks: result.stream_result.output_blocks.clone(),
    }
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

    // Capture start time for iteration log
    let started_at = Utc::now();

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

    // Capture completion time
    let completed_at = Utc::now();

    // Build iteration log from subprocess result
    let iteration_log = build_iteration_log(&result, started_at, completed_at);

    // Write iteration log to session directory
    write_iteration_log(&session_dir, &iteration_log)?;

    // Check exit code (after writing log to preserve partial data)
    if result.exit_code != 0 {
        return Err(AskError::NonZeroExit {
            exit_code: result.exit_code,
        });
    }

    Ok(AskResult { slug })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_processor::StreamProcessorResult;

    /// Create a test StreamingSubprocessResult with the given stream result.
    fn make_test_result(stream_result: StreamProcessorResult) -> StreamingSubprocessResult {
        StreamingSubprocessResult {
            exit_code: 0,
            stderr: String::new(),
            stream_result,
        }
    }

    /// Create a test StreamingSubprocessResult with a custom exit code.
    fn make_test_result_with_exit(
        exit_code: i32,
        stream_result: StreamProcessorResult,
    ) -> StreamingSubprocessResult {
        StreamingSubprocessResult {
            exit_code,
            stderr: String::new(),
            stream_result,
        }
    }

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

    #[test]
    fn test_ask_error_session_display() {
        let session_error = SessionError::DuplicateSlug {
            slug: "test-session".to_string(),
        };
        let error = AskError::Session(session_error);
        let msg = format!("{}", error);
        assert!(msg.contains("Session initialization failed"));
        assert!(msg.contains("test-session"));
    }

    #[test]
    fn test_ask_error_iteration_display() {
        let io_error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied");
        let iteration_error = IterationError::WriteLog {
            path: "/test/path".to_string(),
            source: io_error,
        };
        let error = AskError::Iteration(iteration_error);
        let msg = format!("{}", error);
        assert!(msg.contains("iteration log"));
    }

    #[test]
    fn test_build_iteration_log_basic() {
        let result = make_test_result(StreamProcessorResult::default());
        let started = Utc::now();
        let completed = Utc::now();

        let log = build_iteration_log(&result, started, completed);

        assert_eq!(log.sequence, 1);
        assert_eq!(log.exit_code, 0);
        assert_eq!(log.pending_before, 0);
        assert_eq!(log.pending_after, 0);
        assert!(log.metadata.is_none());
        assert!(log.tool_calls.is_empty());
        assert!(log.chunks.is_empty());
        assert!(log.output_blocks.is_empty());
    }

    #[test]
    fn test_build_iteration_log_with_metadata() {
        use ralph_core::stream::{IterationCosts, IterationMetadata, Usage};

        let metadata = IterationMetadata {
            session_id: Some("test-session-id".to_string()),
            model: Some("claude-opus-4-5".to_string()),
            tools: vec![],
        };

        let costs = IterationCosts {
            cost_usd: Some(0.05),
            duration_ms: Some(5000),
            usage: Some(Usage {
                input_tokens: 100,
                output_tokens: 200,
                cache_read_input_tokens: Some(50),
                cache_creation_input_tokens: None,
            }),
        };

        let result = make_test_result(StreamProcessorResult {
            metadata,
            costs,
            ..Default::default()
        });

        let started = Utc::now();
        let completed = Utc::now();

        let log = build_iteration_log(&result, started, completed);

        assert!(log.metadata.is_some());
        let log_metadata = log.metadata.unwrap();
        assert_eq!(
            log_metadata.claude_session_id,
            Some("test-session-id".to_string())
        );
        assert_eq!(log_metadata.model, Some("claude-opus-4-5".to_string()));
        assert_eq!(log_metadata.cost_usd, Some(0.05));
        assert_eq!(log_metadata.duration_ms, Some(5000));
        assert!(log_metadata.usage.is_some());
    }

    #[test]
    fn test_build_iteration_log_with_tool_interactions() {
        use ralph_core::stream::ToolInteraction;
        use serde_json::json;

        let tool_interactions = vec![
            ToolInteraction {
                id: "toolu_01".to_string(),
                name: "Read".to_string(),
                input: json!({"file_path": "/src/main.rs"}),
                result: Some("fn main() {}".to_string()),
                is_error: false,
            },
            ToolInteraction {
                id: "toolu_02".to_string(),
                name: "Glob".to_string(),
                input: json!({"pattern": "*.rs"}),
                result: Some("src/main.rs\nsrc/lib.rs".to_string()),
                is_error: false,
            },
        ];

        let result = make_test_result(StreamProcessorResult {
            tool_interactions,
            ..Default::default()
        });

        let started = Utc::now();
        let completed = Utc::now();

        let log = build_iteration_log(&result, started, completed);

        assert_eq!(log.tool_calls.len(), 2);
        assert_eq!(log.tool_calls[0].name, "Read");
        assert_eq!(log.tool_calls[1].name, "Glob");
    }

    #[test]
    fn test_build_iteration_log_with_chunks() {
        use ralph_core::chunk::ParsedChunk;

        let chunks = vec![
            ParsedChunk::prose("Hello, world!"),
            ParsedChunk::code("fn main() {}", Some("rust".to_string())),
        ];

        let result = make_test_result(StreamProcessorResult {
            chunks,
            ..Default::default()
        });

        let started = Utc::now();
        let completed = Utc::now();

        let log = build_iteration_log(&result, started, completed);

        assert_eq!(log.chunks.len(), 2);
        assert_eq!(log.chunks[0].chunk_type, "prose");
        assert_eq!(log.chunks[1].chunk_type, "code");
        assert_eq!(log.chunks[1].language, Some("rust".to_string()));
    }

    #[test]
    fn test_build_iteration_log_preserves_non_zero_exit() {
        let result = make_test_result_with_exit(1, StreamProcessorResult::default());
        let started = Utc::now();
        let completed = Utc::now();

        let log = build_iteration_log(&result, started, completed);

        assert_eq!(log.exit_code, 1);
    }

    #[test]
    fn test_build_iteration_log_preserves_timestamps() {
        use chrono::TimeZone;

        let result = make_test_result(StreamProcessorResult::default());

        // Use distinct timestamps to verify they're not swapped
        let started = Utc.with_ymd_and_hms(2025, 1, 1, 10, 0, 0).unwrap();
        let completed = Utc.with_ymd_and_hms(2025, 1, 1, 10, 5, 0).unwrap();

        let log = build_iteration_log(&result, started, completed);

        assert_eq!(log.started_at, started);
        assert_eq!(log.completed_at, completed);
    }
}
