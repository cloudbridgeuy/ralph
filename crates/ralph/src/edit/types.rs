use crate::iteration::IterationError;
use crate::session::SessionError;

/// Errors that can occur during the edit workflow.
#[derive(thiserror::Error, Debug)]
pub enum EditError {
    #[error("Session error: {0}")]
    Session(#[from] SessionError),

    #[error("Iteration error: {0}")]
    Iteration(#[from] IterationError),

    #[error("Failed to write temp file: {0}")]
    WriteTempFile(#[source] std::io::Error),

    #[error("Editor '{editor}' failed with exit code {code}")]
    EditorFailed { editor: String, code: i32 },

    #[error("Editor '{editor}' was terminated by signal")]
    EditorSignaled { editor: String },

    #[error("Failed to spawn editor '{editor}': {source}")]
    SpawnEditor {
        editor: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to parse edited TOML: {0}")]
    ParseEditToml(String),

    #[error("Invalid role '{role}' at message {index}. Must be \"user\" or \"assistant\".")]
    InvalidRole { role: String, index: usize },

    #[error("Failed to read temp file: {0}")]
    ReadTempFile(#[source] std::io::Error),

    #[error("Failed to read retry prompt input: {0}")]
    PromptInput(#[source] std::io::Error),

    #[error("Failed to write iteration file: {0}")]
    WriteIterationFile(#[source] std::io::Error),

    #[error("Failed to delete iteration file: {0}")]
    DeleteIterationFile(#[source] std::io::Error),
}

/// A single message in the edit projection.
#[derive(Debug, Clone, PartialEq)]
pub struct EditMessage {
    pub role: String,
    pub content: String,
}

/// A planned change to an iteration file.
#[derive(Debug, Clone, PartialEq)]
pub enum IterationUpdate {
    /// Rewrite an existing iteration file with new prompt/response.
    Rewrite {
        sequence: u32,
        prompt: Option<String>,
        response: Option<String>,
    },
    /// Delete an iteration file.
    Delete { sequence: u32 },
    /// Create a new iteration file with synthetic defaults.
    Create {
        sequence: u32,
        prompt: Option<String>,
        response: Option<String>,
    },
}

/// Summary of changes applied.
#[derive(Debug, Clone, PartialEq)]
pub struct EditSummary {
    pub edited: usize,
    pub deleted: usize,
    pub added: usize,
}
