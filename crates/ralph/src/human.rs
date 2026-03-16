//! Human-in-the-loop I/O shell.
//!
//! Imperative shell for human interaction: opens the editor for conversation
//! input and displays agent comments with soft-blocking terminal prompts.
//! All business logic (content building, response parsing) lives in
//! `ralph_core::transcript` — this module only handles I/O.

use std::io::Write;
use std::path::Path;

use ralph_core::transcript::{
    build_editor_content, parse_editor_response, CommentResponse, HumanResponse, TranscriptEntry,
};

/// Errors from human interaction I/O.
#[derive(Debug, thiserror::Error)]
pub enum HumanError {
    /// Failed to write the temporary editor file.
    #[error("Failed to write temp file: {0}")]
    WriteTempFile(std::io::Error),

    /// Failed to read the temporary editor file after editing.
    #[error("Failed to read temp file: {0}")]
    ReadTempFile(std::io::Error),

    /// Failed to spawn the editor process.
    #[error("Failed to spawn editor '{editor}': {source}")]
    SpawnEditor {
        editor: String,
        source: std::io::Error,
    },

    /// The editor exited with a non-zero status code.
    #[error("Editor '{editor}' exited with code {code}")]
    EditorFailed { editor: String, code: i32 },

    /// The editor was killed by a signal.
    #[error("Editor '{editor}' was killed by signal")]
    EditorSignaled { editor: String },

    /// Failed to read terminal input.
    #[error("Failed to read input: {0}")]
    PromptInput(std::io::Error),
}

/// Resolve which editor to use.
///
/// Checks `$VISUAL`, then `$EDITOR`, falling back to `"vi"`.
fn resolve_editor() -> String {
    for var in &["VISUAL", "EDITOR"] {
        if let Ok(val) = std::env::var(var) {
            if !val.is_empty() {
                return val;
            }
        }
    }
    "vi".to_string()
}

/// Spawn the editor and wait for it to exit.
fn spawn_editor(editor: &str, temp_path: &Path) -> Result<(), HumanError> {
    let status = std::process::Command::new(editor)
        .arg(temp_path)
        .status()
        .map_err(|source| HumanError::SpawnEditor {
            editor: editor.to_string(),
            source,
        })?;

    if status.success() {
        return Ok(());
    }

    match status.code() {
        Some(code) => Err(HumanError::EditorFailed {
            editor: editor.to_string(),
            code,
        }),
        None => Err(HumanError::EditorSignaled {
            editor: editor.to_string(),
        }),
    }
}

/// Open the editor for human input in a conversation loop.
///
/// 1. Builds editor content from the transcript (pure, from core)
/// 2. Writes to a temp file
/// 3. Opens the editor
/// 4. Reads back and parses the response (pure, from core)
pub fn open_editor_for_human(transcript: &[TranscriptEntry]) -> Result<HumanResponse, HumanError> {
    let content = build_editor_content(transcript);

    let mut temp_file = tempfile::Builder::new()
        .prefix("ralph-conversation-")
        .suffix(".md")
        .tempfile()
        .map_err(HumanError::WriteTempFile)?;

    temp_file
        .write_all(content.as_bytes())
        .map_err(HumanError::WriteTempFile)?;

    // Flush stdout/stderr so banners are visible before the editor takes over
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();

    let editor = resolve_editor();
    spawn_editor(&editor, temp_file.path())?;

    let edited_content =
        std::fs::read_to_string(temp_file.path()).map_err(HumanError::ReadTempFile)?;

    Ok(parse_editor_response(&edited_content))
}

/// Open the editor with a question/context for the human to respond to.
///
/// Used when an agent emits `<ralph-ask to="human">`. The question is
/// shown above the separator line as context.
pub fn open_editor_for_ask(question: &str) -> Result<HumanResponse, HumanError> {
    let context_entry = TranscriptEntry {
        speaker: ralph_core::transcript::Speaker::Persona("agent".to_string()),
        content: question.to_string(),
    };
    open_editor_for_human(&[context_entry])
}

/// Display a comment and soft-block until Enter or typed response.
///
/// Used when an agent emits `<ralph-comment to="human">`. Shows the
/// comment text and waits for the human to acknowledge or reply.
pub fn display_comment_and_wait(text: &str) -> Result<CommentResponse, HumanError> {
    eprintln!("\n{text}\n");
    eprint!("Press Enter to continue, or type a response: ");
    let _ = std::io::stderr().flush();

    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(HumanError::PromptInput)?;

    let trimmed = input.trim();
    if trimmed.is_empty() {
        Ok(CommentResponse::Continue)
    } else {
        Ok(CommentResponse::Reply(trimmed.to_string()))
    }
}
