//! Data structures for startup and iteration display.

use crate::iteration::ConversationMessage;
use std::path::{Path, PathBuf};

/// Version of the ralph binary (from Cargo.toml).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Information to display at startup.
#[derive(Debug)]
pub struct StartupInfo {
    /// The session slug (generated or user-provided).
    pub slug: String,
    /// Total number of stories in the PRD.
    pub total_stories: usize,
    /// Number of pending stories.
    pub pending_stories: usize,
    /// Number of completed stories.
    pub completed_stories: usize,
    /// Maximum iterations to run.
    pub max_iterations: usize,
    /// Whether max_iterations was explicitly provided by user.
    pub iterations_from_arg: bool,
    /// Path to PRD file (only shown if custom).
    pub custom_prd_path: Option<PathBuf>,
    /// Path to design file (only shown if custom).
    pub custom_design_path: Option<PathBuf>,
    /// Path to progress file (only shown if custom).
    pub custom_progress_path: Option<PathBuf>,
    /// Whether a custom command template is used.
    pub custom_command: bool,
    /// Whether a custom prompt is used.
    pub custom_prompt: bool,
    /// Whether a custom completion marker is used.
    pub custom_completion_marker: bool,
    /// Whether an additional prompt was provided.
    pub custom_additional_prompt: bool,
    /// Session directory path.
    pub session_dir: PathBuf,
}

impl StartupInfo {
    /// Check if any custom configuration is being used.
    pub(super) fn has_custom_config(&self) -> bool {
        self.custom_prd_path.is_some()
            || self.custom_design_path.is_some()
            || self.custom_progress_path.is_some()
            || self.custom_command
            || self.custom_prompt
            || self.custom_completion_marker
            || self.custom_additional_prompt
    }
}

/// Information for iteration header display.
#[derive(Debug)]
pub struct IterationHeader {
    /// Current iteration number (1-indexed).
    pub iteration: usize,
    /// Maximum iterations, if known.
    pub max_iterations: Option<usize>,
    /// Number of pending stories at the start of this iteration.
    pub pending_stories: usize,
}

/// Information for iteration summary display.
#[derive(Debug)]
pub struct IterationSummary {
    /// Current iteration number (1-indexed).
    pub iteration: usize,
    /// Cost in USD for this iteration (from result event).
    pub cost_usd: Option<f64>,
    /// Duration in milliseconds (from result event).
    pub duration_ms: Option<u64>,
    /// Model name used for this iteration.
    pub model: Option<String>,
    /// Input tokens used.
    pub input_tokens: Option<u64>,
    /// Output tokens generated.
    pub output_tokens: Option<u64>,
}

/// Information for final run summary display.
#[derive(Debug, Clone)]
pub struct RunSummary {
    /// The session slug.
    pub slug: String,
    /// Total number of iterations completed.
    pub iterations_completed: usize,
    /// Reason for completion (if any).
    pub completion_reason: Option<String>,
    /// Total cost across all iterations (USD).
    pub total_cost_usd: Option<f64>,
    /// Total duration across all iterations (wall clock time from start).
    pub total_duration_ms: Option<u64>,
    /// Total input tokens across all iterations.
    pub total_input_tokens: Option<u64>,
    /// Total output tokens across all iterations.
    pub total_output_tokens: Option<u64>,
    /// Final pending story count.
    pub final_pending_stories: usize,
}

/// Information for ask command summary display.
#[derive(Debug, Clone)]
pub struct AskSummary {
    /// The session slug.
    pub slug: String,
    /// Whether the ask succeeded (exit code 0).
    pub success: bool,
    /// Cost in USD.
    pub cost_usd: Option<f64>,
    /// Duration in milliseconds.
    pub duration_ms: Option<u64>,
    /// Input tokens used.
    pub input_tokens: Option<u64>,
    /// Output tokens generated.
    pub output_tokens: Option<u64>,
}

/// Metadata about an attached file for prompt display.
#[derive(Debug, Clone)]
pub struct AttachedFile {
    /// Path to the file.
    pub path: PathBuf,
    /// Human-readable description of the file's purpose.
    pub description: &'static str,
}

impl AttachedFile {
    /// Create an attached file with a known description based on file name.
    ///
    /// Uses standard descriptions for known context files:
    /// - `design.md` → "Design document"
    /// - `prd.toml` → "Product requirements"
    /// - `progress.txt` → "Progress notes"
    /// - Other files → "Attached file"
    pub fn new(path: PathBuf) -> Self {
        let description = Self::description_for_path(&path);
        Self { path, description }
    }

    /// Get the description for a file based on its name.
    fn description_for_path(path: &Path) -> &'static str {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(|name| match name {
                "design.md" => "Design document",
                "prd.toml" => "Product requirements",
                "progress.txt" => "Progress notes",
                _ => "Attached file",
            })
            .unwrap_or("Attached file")
    }
}

/// Information for prompt display before iterations begin.
#[derive(Debug, Clone)]
pub struct PromptDisplay<'a> {
    /// The prompt text to display.
    pub prompt: &'a str,
    /// Files attached to the prompt (for table display).
    pub attached_files: Vec<AttachedFile>,
}

impl<'a> PromptDisplay<'a> {
    /// Create a PromptDisplay by extracting file references from the prompt text.
    ///
    /// This parses `@/path/to/file` references from the prompt and creates
    /// AttachedFile entries for each. Useful for replay when we only have
    /// the stored prompt text.
    pub fn from_prompt(prompt: &'a str) -> Self {
        let attached_files = extract_file_references(prompt);
        Self {
            prompt,
            attached_files,
        }
    }

    /// Get the prompt text with file references stripped.
    ///
    /// This is useful for display purposes where the file references
    /// are shown in a separate table.
    pub fn stripped_prompt(&self) -> String {
        strip_file_references(self.prompt)
    }
}

/// Parse `@/path` file references from a prompt string.
///
/// Returns both the stripped prompt (without file references) and
/// the list of AttachedFiles found.
fn parse_prompt_file_references(prompt: &str) -> (String, Vec<AttachedFile>) {
    let mut stripped = String::with_capacity(prompt.len());
    let mut files = Vec::new();
    let mut chars = prompt.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '@' {
            // Check if this looks like a file reference (@ followed by / or letter)
            if let Some(&next) = chars.peek() {
                if next == '/' || next.is_alphabetic() {
                    // Collect the path until whitespace
                    let mut path = String::new();
                    while let Some(&c) = chars.peek() {
                        if c.is_whitespace() {
                            break;
                        }
                        path.push(c);
                        chars.next();
                    }
                    if !path.is_empty() {
                        files.push(AttachedFile::new(PathBuf::from(path)));
                    }
                    continue;
                }
            }
        }
        stripped.push(ch);
    }

    (clean_blank_lines(&stripped), files)
}

/// Extract `@/path` file references from a prompt string.
///
/// Returns a Vec of AttachedFile for each `@` followed by a path.
fn extract_file_references(prompt: &str) -> Vec<AttachedFile> {
    parse_prompt_file_references(prompt).1
}

/// Strip @/path file references from the prompt for display.
///
/// This removes inline file references that start with `@` followed by a path,
/// as these are shown in the attached files table instead.
pub(super) fn strip_file_references(prompt: &str) -> String {
    parse_prompt_file_references(prompt).0
}

/// A single turn in a conversation history for display.
#[derive(Debug, Clone)]
pub struct ConversationTurn {
    /// The user's prompt for this turn.
    pub prompt: String,
    /// The assistant's response for this turn.
    pub response: String,
    /// The iteration number (1-indexed).
    pub iteration: u32,
}

/// Conversation history for display.
#[derive(Debug, Clone)]
pub struct ConversationHistory {
    /// The session slug.
    pub slug: String,
    /// The conversation turns in chronological order.
    pub turns: Vec<ConversationTurn>,
}

impl ConversationHistory {
    /// Create conversation history from iteration messages.
    ///
    /// Transforms a list of conversation messages into a display-ready history
    /// by adding 1-indexed iteration numbers to each turn.
    pub fn from_messages(slug: String, messages: Vec<ConversationMessage>) -> Self {
        let turns = messages
            .into_iter()
            .enumerate()
            .map(|(i, msg)| ConversationTurn {
                prompt: msg.prompt,
                response: msg.response,
                iteration: (i + 1) as u32,
            })
            .collect();
        Self { slug, turns }
    }

    /// Check if the conversation history is empty.
    pub fn is_empty(&self) -> bool {
        self.turns.is_empty()
    }
}

/// Collapse multiple consecutive blank lines into a single blank line.
pub(super) fn clean_blank_lines(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prev_blank = false;

    for line in text.lines() {
        let is_blank = line.trim().is_empty();
        if is_blank {
            if !prev_blank {
                result.push('\n');
            }
            prev_blank = true;
        } else {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(line);
            prev_blank = false;
        }
    }

    result
}
