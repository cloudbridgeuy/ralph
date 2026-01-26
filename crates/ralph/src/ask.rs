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
    extract_conversation_messages, extract_response_text, load_session_iterations,
    write_iteration_log, Chunk, ConversationMessage, IterationError, IterationLog, LogMetadata,
    LogToolCall,
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

/// Default permission mode for ask command (allows tool execution).
pub const DEFAULT_PERMISSION_MODE: &str = "bypassPermissions";

/// Environment variable for permission mode override.
const RALPH_PERMISSION_MODE_ENV: &str = "RALPH_PERMISSION_MODE";

/// Resolve permission mode from multiple sources with precedence.
///
/// Resolution order (highest priority first):
/// 1. CLI flag (passed as `cli_value`)
/// 2. Environment variable (`RALPH_PERMISSION_MODE`)
/// 3. Config file (`~/.config/ralph/config.toml` under `[ask]` section)
/// 4. Default value (`bypassPermissions`)
///
/// # Arguments
///
/// * `cli_value` - The value from the CLI flag, if provided (None means not specified)
///
/// # Returns
///
/// The resolved permission mode string.
pub fn resolve_permission_mode(cli_value: Option<&str>) -> String {
    // If CLI value is explicitly set, use it (highest priority)
    if let Some(mode) = cli_value {
        return mode.to_string();
    }

    // Check environment variable
    if let Ok(env_value) = std::env::var(RALPH_PERMISSION_MODE_ENV) {
        if !env_value.is_empty() {
            return env_value;
        }
    }

    // Check config file
    if let Ok(config) = crate::config::AppConfig::load() {
        if let Some(config_value) = config.ask.permission_mode {
            return config_value;
        }
    }

    // Use default
    DEFAULT_PERMISSION_MODE.to_string()
}

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
    /// Optional user-provided session slug (for new sessions).
    pub slug: Option<String>,
    /// Session continuation info (when continuing an existing session).
    pub continuation: Option<ContinuationInfo>,
    /// Clone info (when cloning from an existing session into a new one).
    pub clone: Option<CloneInfo>,
    /// Permission mode for tool execution (default, acceptEdits, plan, bypassPermissions).
    pub permission_mode: String,
}

/// Information about continuing an existing session.
#[derive(Debug, Clone)]
pub struct ContinuationInfo {
    /// The slug of the session to continue.
    pub slug: String,
    /// The sequence number for the new iteration.
    pub next_sequence: u32,
    /// Path to the existing session directory.
    pub session_dir: PathBuf,
}

/// Information about cloning from an existing session.
#[derive(Debug, Clone)]
pub struct CloneInfo {
    /// The slug of the source session to clone from.
    pub source_slug: String,
    /// Path to the source session directory.
    pub source_session_dir: PathBuf,
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
            continuation: None,
            clone: None,
            permission_mode: DEFAULT_PERMISSION_MODE.to_string(),
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
/// - `--permission-mode` with configurable mode (default: bypassPermissions)
/// - `--output-format stream-json` for stream processing
/// - `-p` for the prompt
///
/// # Arguments
///
/// * `prompt` - The prompt to send to the LLM
/// * `permission_mode` - The permission mode for tool execution
///
/// # Returns
///
/// The full command string to execute.
fn build_command(prompt: &str, permission_mode: &str) -> String {
    let escaped = prompt.replace('\'', "'\"'\"'");
    format!(
        "claude --verbose --permission-mode {} --output-format stream-json -p '{}'",
        permission_mode, escaped
    )
}

/// Format conversation history for prepending to a new prompt.
///
/// Takes a list of previous conversation messages and formats them into a
/// structured history block that can be prepended to a new prompt. This allows
/// Claude to understand the context of a continued conversation.
///
/// # Format
///
/// The output format is:
/// ```text
/// <conversation_history>
/// [User]: Previous prompt 1
///
/// [Assistant]: Previous response 1
///
/// [User]: Previous prompt 2
///
/// [Assistant]: Previous response 2
/// </conversation_history>
///
/// [User]: {new_prompt}
/// ```
///
/// # Arguments
///
/// * `messages` - Previous conversation messages in chronological order
/// * `new_prompt` - The new prompt to append after the history
///
/// # Returns
///
/// The full prompt with conversation history prepended, or just the new prompt
/// if there are no previous messages.
fn format_conversation_history(messages: &[ConversationMessage], new_prompt: &str) -> String {
    if messages.is_empty() {
        return new_prompt.to_string();
    }

    let mut parts = vec!["<conversation_history>".to_string()];

    for message in messages {
        parts.push(format!("[User]: {}", message.prompt));
        parts.push(String::new()); // Empty line between user and assistant
        parts.push(format!("[Assistant]: {}", message.response));
        parts.push(String::new()); // Empty line between turns
    }

    parts.push("</conversation_history>".to_string());
    parts.push(String::new());
    parts.push(format!("[User]: {}", new_prompt));

    parts.join("\n")
}

/// Build an iteration log from subprocess result.
///
/// This is a pure function that extracts metadata, tool calls, chunks,
/// and output blocks from the subprocess result to create an IterationLog.
///
/// # Arguments
///
/// * `result` - The subprocess result containing stream processing data
/// * `prompt` - The user's prompt text for this iteration
/// * `sequence` - The iteration sequence number
/// * `started_at` - When the iteration started
/// * `completed_at` - When the iteration completed
///
/// # Returns
///
/// An `IterationLog` populated with the given sequence, timestamps, exit code,
/// prompt, extracted response, metadata, tool calls, chunks, and output blocks.
fn build_iteration_log(
    result: &StreamingSubprocessResult,
    prompt: &str,
    sequence: u32,
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

    // Extract the assistant's response text from output blocks
    let response = extract_response_text(&result.stream_result.output_blocks);

    IterationLog {
        sequence,
        started_at,
        completed_at,
        exit_code: result.exit_code,
        pending_before: 0, // Ask command has no stories
        pending_after: 0,  // Ask command has no stories
        prompt: Some(prompt.to_string()),
        response,
        metadata: metadata.into_option(),
        tool_calls,
        chunks,
        output_blocks: result.stream_result.output_blocks.clone(),
    }
}

/// Result of an ask command execution.
///
/// Contains the session information and metrics for display and finalization.
/// This is returned even on failure (non-zero exit code) so that the caller
/// can finalize the session with the appropriate outcome.
#[derive(Debug)]
pub struct AskResult {
    /// The session slug that was created or continued.
    pub slug: String,
    /// Total number of iterations in the session after this execution.
    pub iteration_count: u32,
    /// Exit code from the subprocess (0 = success).
    pub exit_code: i32,
    /// Cost in USD for this request.
    pub cost_usd: Option<f64>,
    /// Duration in milliseconds.
    pub duration_ms: Option<u64>,
    /// Input tokens used.
    pub input_tokens: Option<u64>,
    /// Output tokens generated.
    pub output_tokens: Option<u64>,
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

    // Determine session info: clone, continue existing, or create new
    let (slug, session_dir, sequence, conversation_history) =
        if let Some(ref clone_info) = config.clone {
            // Clone mode: load conversation history from source, create NEW session
            let logs = load_session_iterations(&clone_info.source_session_dir)?;
            let messages = extract_conversation_messages(&logs);

            // Create a new session (without user-provided slug, auto-generate)
            let (new_slug, new_session_dir) = session::initialize_session_with_clone(
                config.slug.as_deref(),
                &config.project_path,
                Some(config.prompt.clone()),
                &clone_info.source_slug,
            )?;

            (new_slug, new_session_dir, 1, messages)
        } else if let Some(ref continuation) = config.continuation {
            // Continue existing session - load conversation history
            let logs = load_session_iterations(&continuation.session_dir)?;
            let messages = extract_conversation_messages(&logs);

            (
                continuation.slug.clone(),
                continuation.session_dir.clone(),
                continuation.next_sequence,
                messages,
            )
        } else {
            // Create new session
            let (slug, session_dir) = session::initialize_session(
                config.slug.as_deref(),
                &config.project_path,
                Some(config.prompt.clone()),
            )?;
            (slug, session_dir, 1, Vec::new())
        };

    // Build the full prompt with conversation history if continuing
    let full_prompt = format_conversation_history(&conversation_history, &config.prompt);

    // Build the command with configured permission mode
    let command = build_command(&full_prompt, &config.permission_mode);

    // Build subprocess config with session info for spinner display
    let subprocess_config = SpinnerSubprocessConfig {
        command,
        timeout_secs: config.timeout_secs,
        theme_config: config.theme_config,
        session_elapsed_ms: 0,
        verbose_tools: config.verbose_tools,
        session_info: SpinnerSessionInfo {
            slug: Some(slug.clone()),
            current_iteration: Some(sequence as usize),
            max_iterations: None, // Unknown for ask command
        },
    };

    // Invoke subprocess with spinner
    let result = invoke_subprocess_with_spinner_config(&subprocess_config)?;

    // Capture completion time
    let completed_at = Utc::now();

    // Build iteration log from subprocess result
    let iteration_log =
        build_iteration_log(&result, &config.prompt, sequence, started_at, completed_at);

    // Write iteration log to session directory
    write_iteration_log(&session_dir, &iteration_log)?;

    // Extract metrics from iteration log metadata for result
    let (cost_usd, duration_ms, input_tokens, output_tokens) =
        extract_metrics_from_log(&iteration_log);

    Ok(AskResult {
        slug,
        iteration_count: sequence,
        exit_code: result.exit_code,
        cost_usd,
        duration_ms,
        input_tokens,
        output_tokens,
    })
}

/// Extract cost and token metrics from an iteration log.
///
/// This is a pure function that extracts optional metrics from the metadata.
fn extract_metrics_from_log(
    log: &IterationLog,
) -> (Option<f64>, Option<u64>, Option<u64>, Option<u64>) {
    let metadata = log.metadata.as_ref();

    let cost_usd = metadata.and_then(|m| m.cost_usd);
    let duration_ms = metadata.and_then(|m| m.duration_ms);
    let input_tokens = metadata.and_then(|m| m.usage.as_ref().map(|u| u.input_tokens));
    let output_tokens = metadata.and_then(|m| m.usage.as_ref().map(|u| u.output_tokens));

    (cost_usd, duration_ms, input_tokens, output_tokens)
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
        let cmd = build_command("hello", "bypassPermissions");
        assert_eq!(
            cmd,
            "claude --verbose --permission-mode bypassPermissions --output-format stream-json -p 'hello'"
        );
    }

    #[test]
    fn test_build_command_with_quotes() {
        let cmd = build_command("it's a test", "bypassPermissions");
        assert_eq!(
            cmd,
            "claude --verbose --permission-mode bypassPermissions --output-format stream-json -p 'it'\"'\"'s a test'"
        );
    }

    #[test]
    fn test_build_command_multiline() {
        let cmd = build_command("line1\nline2", "bypassPermissions");
        assert_eq!(
            cmd,
            "claude --verbose --permission-mode bypassPermissions --output-format stream-json -p 'line1\nline2'"
        );
    }

    #[test]
    fn test_build_command_shell_special_chars() {
        let cmd = build_command("test $VAR `cmd`", "bypassPermissions");
        assert_eq!(
            cmd,
            "claude --verbose --permission-mode bypassPermissions --output-format stream-json -p 'test $VAR `cmd`'"
        );
    }

    #[test]
    fn test_build_command_accept_edits_mode() {
        let cmd = build_command("hello", "acceptEdits");
        assert_eq!(
            cmd,
            "claude --verbose --permission-mode acceptEdits --output-format stream-json -p 'hello'"
        );
    }

    #[test]
    fn test_build_command_default_mode() {
        let cmd = build_command("hello", "default");
        assert_eq!(
            cmd,
            "claude --verbose --permission-mode default --output-format stream-json -p 'hello'"
        );
    }

    #[test]
    fn test_build_command_plan_mode() {
        let cmd = build_command("hello", "plan");
        assert_eq!(
            cmd,
            "claude --verbose --permission-mode plan --output-format stream-json -p 'hello'"
        );
    }

    #[test]
    fn test_ask_config_default() {
        let config = AskConfig::default();
        assert!(config.prompt.is_empty());
        assert_eq!(config.timeout_secs, DEFAULT_TIMEOUT_SECS);
        assert!(config.project_path.as_os_str().is_empty());
        assert!(config.slug.is_none());
        assert_eq!(config.permission_mode, DEFAULT_PERMISSION_MODE);
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
            iteration_count: 1,
            exit_code: 0,
            cost_usd: Some(0.05),
            duration_ms: Some(5000),
            input_tokens: Some(100),
            output_tokens: Some(200),
        };
        assert_eq!(result.slug, "test-slug");
        assert_eq!(result.iteration_count, 1);
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.cost_usd, Some(0.05));
        assert_eq!(result.duration_ms, Some(5000));
        assert_eq!(result.input_tokens, Some(100));
        assert_eq!(result.output_tokens, Some(200));
    }

    #[test]
    fn test_ask_error_display() {
        let error = AskError::NoPrompt;
        let msg = format!("{}", error);
        assert!(msg.contains("No prompt provided"));
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

        let log = build_iteration_log(&result, "test prompt", 1, started, completed);

        assert_eq!(log.sequence, 1);
        assert_eq!(log.exit_code, 0);
        assert_eq!(log.pending_before, 0);
        assert_eq!(log.pending_after, 0);
        assert_eq!(log.prompt, Some("test prompt".to_string()));
        assert!(log.response.is_none()); // No output_blocks in default result
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

        let log = build_iteration_log(&result, "test prompt", 1, started, completed);

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

        let log = build_iteration_log(&result, "test prompt", 1, started, completed);

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

        let log = build_iteration_log(&result, "test prompt", 1, started, completed);

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

        let log = build_iteration_log(&result, "test prompt", 1, started, completed);

        assert_eq!(log.exit_code, 1);
    }

    #[test]
    fn test_build_iteration_log_preserves_timestamps() {
        use chrono::TimeZone;

        let result = make_test_result(StreamProcessorResult::default());

        // Use distinct timestamps to verify they're not swapped
        let started = Utc.with_ymd_and_hms(2025, 1, 1, 10, 0, 0).unwrap();
        let completed = Utc.with_ymd_and_hms(2025, 1, 1, 10, 5, 0).unwrap();

        let log = build_iteration_log(&result, "test prompt", 1, started, completed);

        assert_eq!(log.started_at, started);
        assert_eq!(log.completed_at, completed);
    }

    #[test]
    fn test_format_conversation_history_empty_messages() {
        let result = format_conversation_history(&[], "new prompt");
        assert_eq!(result, "new prompt");
    }

    #[test]
    fn test_format_conversation_history_single_message() {
        let messages = vec![ConversationMessage::new(
            "What is 2+2?".to_string(),
            "The answer is 4.".to_string(),
        )];

        let result = format_conversation_history(&messages, "What is 3+3?");

        assert!(result.starts_with("<conversation_history>"));
        assert!(result.contains("[User]: What is 2+2?"));
        assert!(result.contains("[Assistant]: The answer is 4."));
        assert!(result.contains("</conversation_history>"));
        assert!(result.ends_with("[User]: What is 3+3?"));
    }

    #[test]
    fn test_format_conversation_history_multiple_messages() {
        let messages = vec![
            ConversationMessage::new("First question".to_string(), "First answer".to_string()),
            ConversationMessage::new("Second question".to_string(), "Second answer".to_string()),
        ];

        let result = format_conversation_history(&messages, "Third question");

        // Check structure
        assert!(result.starts_with("<conversation_history>"));
        assert!(result.contains("[User]: First question"));
        assert!(result.contains("[Assistant]: First answer"));
        assert!(result.contains("[User]: Second question"));
        assert!(result.contains("[Assistant]: Second answer"));
        assert!(result.contains("</conversation_history>"));
        assert!(result.ends_with("[User]: Third question"));

        // Check order - first message should appear before second
        let first_idx = result.find("[User]: First question").unwrap();
        let second_idx = result.find("[User]: Second question").unwrap();
        assert!(first_idx < second_idx);
    }

    #[test]
    fn test_format_conversation_history_with_empty_response() {
        let messages = vec![ConversationMessage::new(
            "Question".to_string(),
            String::new(), // Empty response
        )];

        let result = format_conversation_history(&messages, "Follow up");

        assert!(result.contains("[User]: Question"));
        assert!(result.contains("[Assistant]: ")); // Empty response still included
        assert!(result.ends_with("[User]: Follow up"));
    }

    #[test]
    fn test_format_conversation_history_multiline_content() {
        let messages = vec![ConversationMessage::new(
            "Line 1\nLine 2".to_string(),
            "Response line 1\nResponse line 2".to_string(),
        )];

        let result = format_conversation_history(&messages, "New prompt");

        assert!(result.contains("[User]: Line 1\nLine 2"));
        assert!(result.contains("[Assistant]: Response line 1\nResponse line 2"));
    }

    #[test]
    fn test_resolve_permission_mode_cli_explicit() {
        // CLI value should be used directly
        let result = resolve_permission_mode(Some("acceptEdits"));
        assert_eq!(result, "acceptEdits");
    }

    #[test]
    fn test_resolve_permission_mode_cli_explicit_default() {
        // Even if CLI value matches default, it should be used (explicit override)
        let result = resolve_permission_mode(Some(DEFAULT_PERMISSION_MODE));
        assert_eq!(result, DEFAULT_PERMISSION_MODE);
    }

    #[test]
    fn test_resolve_permission_mode_none_returns_default() {
        // Without CLI, env, or config, returns the default
        // Note: This test is environment-dependent but should work in most test environments
        let result = resolve_permission_mode(None);
        // Result should be non-empty - either default or from env/config
        assert!(!result.is_empty());
    }
}
