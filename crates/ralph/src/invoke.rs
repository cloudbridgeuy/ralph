//! Shared invocation engine for ask and persona commands.
//!
//! Both `ralph ask` and `ralph persona` delegate to this module.
//! The engine handles: session setup, conversation history, command building,
//! subprocess invocation, and iteration log writing.

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

/// Default permission mode (allows tool execution).
pub const DEFAULT_PERMISSION_MODE: &str = "bypassPermissions";

/// Configuration for a shared invocation.
#[derive(Debug, Clone)]
pub struct InvocationConfig {
    /// The user's prompt text.
    pub prompt: String,
    /// Timeout for the subprocess in seconds.
    pub timeout_secs: u64,
    /// Theme configuration for syntax highlighting.
    pub theme_config: ThemeConfig,
    /// Verbose tools configuration.
    pub verbose_tools: VerboseToolsConfig,
    /// Absolute path to the project directory.
    pub project_path: PathBuf,
    /// Optional user-provided session slug (for new sessions).
    pub slug: Option<String>,
    /// Continuation info if resuming a session.
    pub continuation: Option<ContinuationInfo>,
    /// Clone info if branching from an existing session.
    pub clone: Option<CloneInfo>,
    /// Permission mode for claude CLI.
    pub permission_mode: String,
    /// Persona name. None = ask (no agent), Some = persona (uses --agent flag).
    /// Controls both the CLI --agent flag and session metadata persona scoping.
    pub persona: Option<String>,
}

/// Info for continuing an existing session.
#[derive(Debug, Clone)]
pub struct ContinuationInfo {
    pub slug: String,
    pub next_sequence: u32,
    pub session_dir: PathBuf,
}

/// Info for cloning a session.
#[derive(Debug, Clone)]
pub struct CloneInfo {
    pub source_slug: String,
    pub source_session_dir: PathBuf,
}

/// Result of a successful invocation.
#[derive(Debug)]
pub struct InvocationResult {
    pub slug: String,
    pub iteration_count: u32,
    pub exit_code: i32,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// Extracted text response from output blocks. Consumed by the orchestrator to scan for directives.
    pub response_text: Option<String>,
    /// Persona name that produced this result. None for plain ask invocations.
    pub persona: Option<String>,
}

/// Errors that can occur during invocation.
#[derive(Debug, thiserror::Error)]
pub enum InvocationError {
    #[error("No prompt provided. Use: ralph ask 'your prompt here'")]
    NoPrompt,
    #[error("Subprocess failed: {0}")]
    Subprocess(#[from] SubprocessError),
    #[error("Session initialization failed: {0}")]
    Session(#[from] SessionError),
    #[error("Failed to write iteration log: {0}")]
    Iteration(#[from] IterationError),
}

/// Build the claude CLI command string.
///
/// Pure function — no I/O.
pub fn build_command(prompt: &str, permission_mode: &str, agent: Option<&str>) -> String {
    let escaped = prompt.replace('\'', "'\"'\"'");
    match agent {
        Some(name) => format!(
            "claude --verbose --agent {} --output-format stream-json -p '{}'",
            name, escaped
        ),
        None => format!(
            "claude --verbose --permission-mode {} --output-format stream-json -p '{}'",
            permission_mode, escaped
        ),
    }
}

/// Format conversation history for prepending to a new prompt.
///
/// Pure function — no I/O.
pub fn format_conversation_history(messages: &[ConversationMessage], new_prompt: &str) -> String {
    if messages.is_empty() {
        return new_prompt.to_string();
    }

    let mut parts = vec!["<conversation_history>".to_string()];

    for message in messages {
        parts.push(format!("[User]: {}", message.prompt));
        parts.push(String::new());
        parts.push(format!("[Assistant]: {}", message.response));
        parts.push(String::new());
    }

    parts.push("</conversation_history>".to_string());
    parts.push(String::new());
    parts.push(format!("[User]: {}", new_prompt));

    parts.join("\n")
}

/// Build an iteration log from subprocess result.
///
/// Pure function — no I/O.
fn build_iteration_log(
    result: &StreamingSubprocessResult,
    prompt: &str,
    sequence: u32,
    started_at: DateTime<Utc>,
    completed_at: DateTime<Utc>,
) -> IterationLog {
    let metadata = LogMetadata::from_extracted(
        result.stream_result.metadata.clone(),
        result.stream_result.costs.clone(),
    );
    let tool_calls = LogToolCall::from_interactions(&result.stream_result.tool_interactions);
    let chunks = Chunk::from_parsed_chunks(&result.stream_result.chunks);
    let response = extract_response_text(&result.stream_result.output_blocks);

    IterationLog {
        sequence,
        started_at,
        completed_at,
        exit_code: result.exit_code,
        pending_before: 0,
        pending_after: 0,
        prompt: Some(prompt.to_string()),
        response,
        metadata: metadata.into_option(),
        tool_calls,
        chunks,
        output_blocks: result.stream_result.output_blocks.clone(),
    }
}

/// Extract cost and token metrics from an iteration log.
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

/// Shared invocation engine.
///
/// Handles session setup, conversation history, command building,
/// subprocess invocation, and iteration log writing.
pub fn invoke(config: InvocationConfig) -> Result<InvocationResult, InvocationError> {
    if config.prompt.is_empty() {
        return Err(InvocationError::NoPrompt);
    }

    let started_at = Utc::now();
    let result_persona = config.persona.clone();
    let persona = config.persona.as_deref();

    // Determine session info: clone, continue existing, or create new
    let (slug, session_dir, sequence, conversation_history) =
        if let Some(ref clone_info) = config.clone {
            let logs = load_session_iterations(&clone_info.source_session_dir)?;
            let messages = extract_conversation_messages(&logs);

            let (new_slug, new_session_dir) = session::initialize_session_with_clone(
                config.slug.as_deref(),
                &config.project_path,
                Some(config.prompt.clone()),
                &clone_info.source_slug,
                persona,
            )?;

            (new_slug, new_session_dir, 1, messages)
        } else if let Some(ref continuation) = config.continuation {
            let logs = load_session_iterations(&continuation.session_dir)?;
            let messages = extract_conversation_messages(&logs);

            (
                continuation.slug.clone(),
                continuation.session_dir.clone(),
                continuation.next_sequence,
                messages,
            )
        } else {
            let (slug, session_dir) = session::initialize_session(
                config.slug.as_deref(),
                &config.project_path,
                Some(config.prompt.clone()),
                persona,
            )?;
            (slug, session_dir, 1, Vec::new())
        };

    let full_prompt = format_conversation_history(&conversation_history, &config.prompt);
    let command = build_command(
        &full_prompt,
        &config.permission_mode,
        config.persona.as_deref(),
    );

    let subprocess_config = SpinnerSubprocessConfig {
        command,
        timeout_secs: config.timeout_secs,
        theme_config: config.theme_config,
        session_elapsed_ms: 0,
        verbose_tools: config.verbose_tools,
        session_info: SpinnerSessionInfo {
            persona: config.persona.clone(),
            slug: Some(slug.clone()),
            current_iteration: Some(sequence as usize),
            max_iterations: None,
        },
    };

    let outcome = invoke_subprocess_with_spinner_config(&subprocess_config);
    let result = outcome.subprocess_result?;

    let completed_at = Utc::now();

    let iteration_log =
        build_iteration_log(&result, &config.prompt, sequence, started_at, completed_at);

    write_iteration_log(&session_dir, &iteration_log)?;

    let (cost_usd, duration_ms, input_tokens, output_tokens) =
        extract_metrics_from_log(&iteration_log);

    let response_text = iteration_log.response.clone();

    Ok(InvocationResult {
        slug,
        iteration_count: sequence,
        exit_code: result.exit_code,
        cost_usd,
        duration_ms,
        input_tokens,
        output_tokens,
        response_text,
        persona: result_persona,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_command_without_agent() {
        let cmd = build_command("hello", "bypassPermissions", None);
        assert!(cmd.contains("--permission-mode bypassPermissions"));
        assert!(cmd.contains("-p 'hello'"));
        assert!(!cmd.contains("--agent"));
    }

    #[test]
    fn build_command_with_agent() {
        let cmd = build_command("hello", "bypassPermissions", Some("coach"));
        assert!(cmd.contains("--agent coach"));
        assert!(cmd.contains("-p 'hello'"));
        // When agent is set, permission mode is NOT included (agent file controls it)
        assert!(!cmd.contains("--permission-mode"));
    }

    #[test]
    fn build_command_escapes_single_quotes() {
        let cmd = build_command("it's a test", "default", None);
        assert!(cmd.contains("'\"'\"'"));
    }

    #[test]
    fn format_conversation_history_empty() {
        let result = format_conversation_history(&[], "new prompt");
        assert_eq!(result, "new prompt");
    }

    #[test]
    fn format_conversation_history_with_messages() {
        let messages = vec![ConversationMessage::new(
            "old question".to_string(),
            "old answer".to_string(),
        )];
        let result = format_conversation_history(&messages, "new question");
        assert!(result.contains("<conversation_history>"));
        assert!(result.contains("[User]: old question"));
        assert!(result.contains("[Assistant]: old answer"));
        assert!(result.contains("</conversation_history>"));
        assert!(result.contains("[User]: new question"));
    }
}
