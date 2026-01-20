//! Session replay functionality (Imperative Shell).
//!
//! This module provides the ability to replay captured session output with
//! syntax highlighting. It reads iteration logs from a session directory
//! and re-renders the chunks with appropriate formatting.
//!
//! # Example
//!
//! ```no_run
//! use ralph::replay::replay_session;
//!
//! // Replay all iterations from a session
//! replay_session("quiet-mountain", None).unwrap();
//!
//! // Replay only iteration 3
//! replay_session("quiet-mountain", Some(3)).unwrap();
//! ```

use crate::diff_highlight::highlight_with_basic_colors;
use crate::highlight::{Highlighter, ThemeConfig, ThemeError};
use crate::iteration::IterationLog;
use crate::replay_renderer::ReplayRenderer;
use crate::session::{load_sessions_index, session_dir};
use crate::startup::{display_prompt, PromptDisplay};
use ralph_core::session::SessionMetadata;
use std::fs;
use std::io::IsTerminal;
use std::path::PathBuf;

/// Error type for replay operations.
#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    /// Session not found in sessions index.
    #[error("Session '{slug}' not found. Run 'ralph sessions' to list available sessions.")]
    SessionNotFound { slug: String },

    /// Failed to read session directory.
    #[error("Failed to read session directory at {path}: {source}")]
    ReadSessionDir {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// Failed to read iteration log.
    #[error("Failed to read iteration log at {path}: {source}")]
    ReadIterationLog {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// Failed to parse iteration log.
    #[error("Failed to parse iteration log: {0}")]
    ParseIterationLog(#[from] toml::de::Error),

    /// No iteration logs found in session.
    #[error("No iteration logs found in session '{slug}'.")]
    NoIterations { slug: String },

    /// Iteration not found.
    #[error(
        "Iteration {iteration} not found in session '{slug}'. Session has {total} iteration(s)."
    )]
    IterationNotFound {
        slug: String,
        iteration: u32,
        total: usize,
    },

    /// Failed to load sessions index.
    #[error("Failed to load sessions index: {0}")]
    LoadSessionsIndex(#[from] crate::session::SessionError),

    /// Failed to configure theme.
    #[error("Failed to configure theme: {0}")]
    ThemeError(#[from] ThemeError),

    /// Failed to read session metadata.
    #[error("Failed to read session metadata at {path}: {source}")]
    ReadSessionMetadata {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// Failed to parse session metadata.
    #[error("Failed to parse session metadata: {0}")]
    ParseSessionMetadata(String),
}

/// Result of a replay operation.
#[derive(Debug)]
pub struct ReplayResult {
    /// The session slug that was replayed.
    pub slug: String,
    /// Number of iterations replayed.
    pub iterations_replayed: usize,
}

/// Configuration for replay options.
#[derive(Debug, Default, Clone)]
pub struct ReplayOptions {
    /// Optional specific iteration to replay (1-indexed).
    pub iteration: Option<u32>,
    /// Optional theme configuration.
    pub theme_config: Option<ThemeConfig>,
    /// Whether to show the prompt before iterations (default: true).
    pub show_prompt: bool,
}

impl ReplayOptions {
    /// Create new replay options with default values.
    pub fn new() -> Self {
        Self {
            iteration: None,
            theme_config: None,
            show_prompt: true,
        }
    }

    /// Set the iteration to replay.
    pub fn with_iteration(mut self, iteration: Option<u32>) -> Self {
        self.iteration = iteration;
        self
    }

    /// Set the theme configuration.
    pub fn with_theme(mut self, theme_config: Option<ThemeConfig>) -> Self {
        self.theme_config = theme_config;
        self
    }

    /// Set whether to show the prompt.
    pub fn with_show_prompt(mut self, show_prompt: bool) -> Self {
        self.show_prompt = show_prompt;
        self
    }
}

/// Replay a session's output to stdout.
///
/// Reads iteration logs from the session directory and re-renders the
/// captured chunks with syntax highlighting.
///
/// # Arguments
///
/// * `slug` - Session identifier to replay
/// * `iteration` - Optional specific iteration to replay (1-indexed).
///   If None, all iterations are replayed.
///
/// # Returns
///
/// * `Ok(ReplayResult)` - Information about the replay
/// * `Err(ReplayError)` - If the session or iteration is not found
///
/// # Example
///
/// ```no_run
/// use ralph::replay::replay_session;
///
/// // Replay all iterations
/// let result = replay_session("my-session", None).unwrap();
/// println!("Replayed {} iterations", result.iterations_replayed);
///
/// // Replay specific iteration
/// replay_session("my-session", Some(2)).unwrap();
/// ```
pub fn replay_session(slug: &str, iteration: Option<u32>) -> Result<ReplayResult, ReplayError> {
    let options = ReplayOptions::new().with_iteration(iteration);
    replay_session_with_options(slug, options)
}

/// Replay a session's output with custom theme configuration.
///
/// Like [`replay_session`], but allows specifying a custom theme.
///
/// # Arguments
///
/// * `slug` - Session identifier to replay
/// * `iteration` - Optional specific iteration to replay (1-indexed)
/// * `theme_config` - Optional theme configuration. If None, uses environment variables.
pub fn replay_session_with_theme(
    slug: &str,
    iteration: Option<u32>,
    theme_config: Option<ThemeConfig>,
) -> Result<ReplayResult, ReplayError> {
    let options = ReplayOptions::new()
        .with_iteration(iteration)
        .with_theme(theme_config);
    replay_session_with_options(slug, options)
}

/// Replay a session's output with full options.
///
/// This is the main implementation that supports all replay options including
/// theme configuration and prompt display control.
///
/// # Arguments
///
/// * `slug` - Session identifier to replay
/// * `options` - Replay configuration options
pub fn replay_session_with_options(
    slug: &str,
    options: ReplayOptions,
) -> Result<ReplayResult, ReplayError> {
    // Look up session in sessions index
    let index = load_sessions_index()?;

    if !index.slug_exists(slug) {
        return Err(ReplayError::SessionNotFound {
            slug: slug.to_string(),
        });
    }

    // Get session directory
    let session_path = session_dir(slug);

    // Find all iteration logs
    let iteration_logs = find_iteration_logs(&session_path, slug)?;

    if iteration_logs.is_empty() {
        return Err(ReplayError::NoIterations {
            slug: slug.to_string(),
        });
    }

    // Filter to specific iteration if requested
    let logs_to_replay = if let Some(n) = options.iteration {
        let log = iteration_logs.iter().find(|(seq, _)| *seq == n);
        match log {
            Some((_, path)) => vec![(n, path.clone())],
            None => {
                return Err(ReplayError::IterationNotFound {
                    slug: slug.to_string(),
                    iteration: n,
                    total: iteration_logs.len(),
                });
            }
        }
    } else {
        iteration_logs
    };

    // Use provided theme config or fall back to config file + environment variables
    let effective_theme = options
        .theme_config
        .unwrap_or_else(ThemeConfig::from_config_and_env);
    let highlighter = Highlighter::with_config(effective_theme)?;
    let is_terminal = std::io::stdout().is_terminal();

    // Display the prompt before iterations if enabled and we're starting from iteration 1
    // (i.e., not replaying a specific later iteration)
    let should_show_prompt =
        options.show_prompt && (options.iteration.is_none() || options.iteration == Some(1));

    if should_show_prompt {
        try_display_session_prompt(&session_path);
    }

    for (seq, path) in &logs_to_replay {
        replay_iteration(*seq, path, &highlighter, is_terminal)?;
    }

    Ok(ReplayResult {
        slug: slug.to_string(),
        iterations_replayed: logs_to_replay.len(),
    })
}

/// Try to read session metadata and display the prompt if available.
///
/// This is a best-effort operation - errors are silently ignored to handle
/// older sessions that may not have stored prompts.
fn try_display_session_prompt(session_path: &std::path::Path) {
    let session_toml_path = session_path.join("session.toml");
    let content = match fs::read_to_string(&session_toml_path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let metadata = match SessionMetadata::from_toml(&content) {
        Ok(m) => m,
        Err(_) => return,
    };
    if let Some(ref prompt) = metadata.prompt {
        let prompt_display = PromptDisplay { prompt };
        display_prompt(&prompt_display);
    }
}

/// Find all iteration logs in a session directory.
///
/// Returns a sorted list of (sequence_number, path) pairs.
fn find_iteration_logs(
    session_path: &PathBuf,
    slug: &str,
) -> Result<Vec<(u32, PathBuf)>, ReplayError> {
    let entries = fs::read_dir(session_path).map_err(|e| ReplayError::ReadSessionDir {
        path: session_path.display().to_string(),
        source: e,
    })?;

    let mut logs: Vec<(u32, PathBuf)> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            // Match iteration-N.toml files
            if name.starts_with("iteration-") && name.ends_with(".toml") {
                if let Ok(seq) = name
                    .trim_start_matches("iteration-")
                    .trim_end_matches(".toml")
                    .parse::<u32>()
                {
                    logs.push((seq, path));
                }
            }
        }
    }

    // Warn if no logs found (but session exists)
    if logs.is_empty() {
        return Err(ReplayError::NoIterations {
            slug: slug.to_string(),
        });
    }

    // Sort by sequence number
    logs.sort_by_key(|(seq, _)| *seq);

    Ok(logs)
}

/// Replay a single iteration to stdout.
fn replay_iteration(
    sequence: u32,
    path: &PathBuf,
    highlighter: &Highlighter,
    is_terminal: bool,
) -> Result<(), ReplayError> {
    // Read and parse iteration log
    let content = fs::read_to_string(path).map_err(|e| ReplayError::ReadIterationLog {
        path: path.display().to_string(),
        source: e,
    })?;

    let log: IterationLog = toml::from_str(&content)?;

    // Print iteration header
    println!("\n{}", "─".repeat(60));
    println!("Iteration {}", sequence);
    println!("{}", "─".repeat(60));

    // Print metadata if available
    if let Some(ref metadata) = log.metadata {
        if let Some(ref model) = metadata.model {
            println!("Model: {}", model);
        }
        if let Some(cost) = metadata.cost_usd {
            println!("Cost: ${:.4}", cost);
        }
        if let Some(duration) = metadata.duration_ms {
            let seconds = duration as f64 / 1000.0;
            println!("Duration: {:.1}s", seconds);
        }
        if let Some(ref usage) = metadata.usage {
            println!(
                "Tokens: {} in / {} out",
                usage.input_tokens, usage.output_tokens
            );
        }
    }

    println!();

    // Prefer output_blocks for replay (newer format with full tool rendering)
    // Fall back to chunks for older session files without output_blocks
    if !log.output_blocks.is_empty() {
        replay_output_blocks(&log, highlighter, is_terminal);
    } else {
        replay_chunks(&log, highlighter, is_terminal);
    }

    Ok(())
}

/// Replay using output_blocks (newer format with full tool rendering).
fn replay_output_blocks(log: &IterationLog, highlighter: &Highlighter, is_terminal: bool) {
    let renderer = ReplayRenderer::new(highlighter.clone(), is_terminal);

    for block in &log.output_blocks {
        let rendered = renderer.render(block);
        print!("{}", rendered);
    }
}

/// Replay using chunks (legacy format, text-only).
fn replay_chunks(log: &IterationLog, highlighter: &Highlighter, is_terminal: bool) {
    for chunk in &log.chunks {
        match chunk.chunk_type.as_str() {
            "prose" => {
                // Prose: print as-is
                println!("{}", chunk.content);
            }
            "code" => {
                // Code: apply syntax highlighting
                let highlighted = if is_terminal {
                    highlighter.highlight(&chunk.content, chunk.language.as_deref())
                } else {
                    chunk.content.clone()
                };
                println!("```{}", chunk.language.as_deref().unwrap_or(""));
                print!("{}", highlighted);
                println!("```");
            }
            "diff" => {
                // Diff: apply diff highlighting
                let highlighted = if is_terminal {
                    highlight_with_basic_colors(&chunk.content)
                } else {
                    chunk.content.clone()
                };
                print!("{}", highlighted);
            }
            _ => {
                // Unknown chunk type: print as-is
                println!("{}", chunk.content);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iteration::{write_iteration_log, Chunk};
    use chrono::Utc;
    use ralph_core::session::{SessionEntry, SessionsIndex};
    use tempfile::TempDir;

    fn create_test_iteration(sequence: u32) -> IterationLog {
        IterationLog {
            sequence,
            started_at: Utc::now(),
            completed_at: Utc::now(),
            exit_code: 0,
            pending_before: 5,
            pending_after: 4,
            metadata: None,
            tool_calls: vec![],
            chunks: vec![
                Chunk::prose(format!("This is iteration {}", sequence)),
                Chunk::code("fn main() {}".to_string(), Some("rust".to_string())),
            ],
            output_blocks: vec![],
        }
    }

    #[test]
    fn test_find_iteration_logs_empty_dir() {
        let temp_dir = TempDir::new().unwrap();
        let session_path = temp_dir.path().join("empty-session");
        fs::create_dir_all(&session_path).unwrap();

        let result = find_iteration_logs(&session_path, "empty-session");
        assert!(matches!(result, Err(ReplayError::NoIterations { .. })));
    }

    #[test]
    fn test_find_iteration_logs_sorts_by_sequence() {
        let temp_dir = TempDir::new().unwrap();
        let session_path = temp_dir.path().to_path_buf();

        // Create iteration logs out of order
        let log1 = create_test_iteration(1);
        let log3 = create_test_iteration(3);
        let log2 = create_test_iteration(2);

        write_iteration_log(&session_path, &log3).unwrap();
        write_iteration_log(&session_path, &log1).unwrap();
        write_iteration_log(&session_path, &log2).unwrap();

        let logs = find_iteration_logs(&session_path, "test").unwrap();

        assert_eq!(logs.len(), 3);
        assert_eq!(logs[0].0, 1);
        assert_eq!(logs[1].0, 2);
        assert_eq!(logs[2].0, 3);
    }

    #[test]
    fn test_find_iteration_logs_ignores_non_iteration_files() {
        let temp_dir = TempDir::new().unwrap();
        let session_path = temp_dir.path().to_path_buf();

        // Create iteration log
        let log = create_test_iteration(1);
        write_iteration_log(&session_path, &log).unwrap();

        // Create other files that should be ignored
        fs::write(session_path.join("session.toml"), "").unwrap();
        fs::write(session_path.join("iteration-1.diff"), "").unwrap();
        fs::write(session_path.join("notes.txt"), "").unwrap();

        let logs = find_iteration_logs(&session_path, "test").unwrap();

        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].0, 1);
    }

    #[test]
    fn test_replay_session_not_found() {
        // This test may fail if the user has a real session with this name
        // but it's a reasonable test for the error case
        let result = replay_session("nonexistent-session-xyz", None);
        assert!(matches!(result, Err(ReplayError::SessionNotFound { .. })));
    }

    #[test]
    fn test_replay_error_display_session_not_found() {
        let err = ReplayError::SessionNotFound {
            slug: "my-session".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("my-session"));
        assert!(msg.contains("not found"));
    }

    #[test]
    fn test_replay_error_display_iteration_not_found() {
        let err = ReplayError::IterationNotFound {
            slug: "my-session".to_string(),
            iteration: 5,
            total: 3,
        };
        let msg = err.to_string();
        assert!(msg.contains("5"));
        assert!(msg.contains("3"));
    }

    #[test]
    fn test_replay_error_display_no_iterations() {
        let err = ReplayError::NoIterations {
            slug: "empty-session".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("empty-session"));
        assert!(msg.contains("No iteration logs"));
    }

    #[test]
    fn test_sessions_index_lookup() {
        // Verify that slug_exists works correctly
        let mut index = SessionsIndex::new();
        index.add_session(SessionEntry::new(
            "test-session".to_string(),
            PathBuf::from("/test/project"),
        ));

        assert!(index.slug_exists("test-session"));
        assert!(!index.slug_exists("nonexistent"));
    }
}
