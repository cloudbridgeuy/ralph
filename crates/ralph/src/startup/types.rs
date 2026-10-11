//! Data structures for startup and iteration display.

use crate::iteration::ConversationMessage;
use std::path::PathBuf;

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
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            description: "Attached file",
        }
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
