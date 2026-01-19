//! Output block types for replay serialization.
//!
//! This module defines the `OutputBlock` enum and its variants, which capture
//! all data needed to render output during execution. By serializing these blocks,
//! sessions can be replayed with identical visual output.
//!
//! # Design Principles
//!
//! - Each variant captures **data**, not rendered strings
//! - All variants derive Serialize/Deserialize for TOML storage
//! - Rendering is handled by separate functions that take OutputBlock variants
//! - The enum preserves the order of output blocks for faithful replay
//! - Enums are marked `#[non_exhaustive]` for forward compatibility when adding new variants

use ralph_core::chunk::ParsedChunk;
use serde::{Deserialize, Serialize};

/// A single block of output from an LLM interaction.
///
/// OutputBlock captures all the data needed to render a piece of output.
/// During execution, these blocks are accumulated and later serialized
/// to the session file for replay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum OutputBlock {
    /// Text output from the assistant (prose, code, or diff chunks).
    Text(TextBlock),

    /// A tool invocation display (e.g., `▶ Bash command`).
    ToolInvocation(ToolInvocationBlock),

    /// A tool result display.
    ToolResult(ToolResultBlock),

    /// Visual separator between assistant responses.
    Separator,
}

/// Text chunk from assistant output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextBlock {
    /// The parsed chunk containing content and type.
    pub chunk: ParsedChunk,
}

impl From<ParsedChunk> for TextBlock {
    fn from(chunk: ParsedChunk) -> Self {
        Self { chunk }
    }
}

/// Tool invocation display data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolInvocationBlock {
    /// The name of the tool being invoked.
    pub tool_name: String,

    /// The invocation variant with tool-specific data.
    #[serde(flatten)]
    pub variant: ToolInvocationVariant,
}

/// Tool-specific invocation data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "invocation_type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ToolInvocationVariant {
    /// Bash command invocation.
    Bash {
        /// The command being executed.
        command: String,
        /// Optional description of the command.
        description: Option<String>,
    },

    /// Grep search invocation (verbose mode).
    Grep {
        /// The regex pattern being searched.
        pattern: String,
        /// The search path.
        path: Option<String>,
        /// Output mode (files_with_matches, content, count).
        output_mode: Option<String>,
        /// Glob filter.
        glob: Option<String>,
        /// File type filter.
        file_type: Option<String>,
        /// Case insensitive flag.
        case_insensitive: bool,
    },

    /// Read file invocation (verbose mode).
    Read {
        /// Path to the file being read.
        file_path: String,
        /// Line offset if specified.
        offset: Option<u64>,
        /// Line limit if specified.
        limit: Option<u64>,
    },

    /// Glob pattern search invocation (verbose mode).
    Glob {
        /// The glob pattern.
        pattern: String,
        /// The search path.
        path: Option<String>,
    },

    /// TodoWrite invocation (verbose mode).
    TodoWrite {
        /// The todo items being written.
        todos: Vec<TodoItem>,
    },

    /// Default invocation for other tools.
    Default {
        /// The key argument to display (e.g., file_path, pattern).
        key_argument: Option<String>,
        /// Whether the argument is a path (shown in full) or content (truncated).
        is_path: bool,
    },
}

/// A single todo item for TodoWrite display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TodoItem {
    /// The todo content.
    pub content: String,
    /// The status (pending, in_progress, completed).
    pub status: String,
    /// The active form text (shown during execution).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_form: Option<String>,
}

/// Tool result display data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultBlock {
    /// The name of the tool that produced this result.
    pub tool_name: String,

    /// Whether this result represents an error.
    pub is_error: bool,

    /// The result variant with tool-specific data.
    #[serde(flatten)]
    pub variant: ToolResultVariant,
}

/// Tool-specific result data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result_type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ToolResultVariant {
    /// Bash command result.
    Bash {
        /// The output content (stdout or stderr).
        content: Option<String>,
        /// Whether output was truncated.
        truncated: bool,
    },

    /// Edit tool result with before/after display.
    EditBeforeAfter {
        /// Path to the edited file.
        file_path: String,
        /// The old content (before edit).
        old_content: String,
        /// The new content (after edit).
        new_content: String,
    },

    /// Edit tool result with unified diff display.
    EditDiff {
        /// Path to the edited file.
        file_path: String,
        /// The unified diff content.
        diff_content: String,
    },

    /// Edit tool result with no changes.
    EditNoChanges {
        /// Path to the file.
        file_path: String,
    },

    /// Write tool result for a new file.
    WriteNewFile {
        /// Path to the new file.
        file_path: String,
        /// The file content.
        content: String,
    },

    /// Write tool result for file overwrite.
    WriteOverwrite {
        /// Path to the file.
        file_path: String,
        /// Content before the write.
        before_content: String,
        /// Content after the write.
        after_content: String,
    },

    /// Write tool result with no changes.
    WriteNoChanges {
        /// Path to the file.
        file_path: String,
        /// Whether this was a new file (empty).
        is_new_file: bool,
    },

    /// Read tool result (verbose mode).
    Read {
        /// Path to the file read.
        file_path: String,
        /// The file content.
        content: String,
        /// Total number of lines.
        line_count: usize,
        /// Whether content was truncated.
        truncated: bool,
    },

    /// Grep search result (verbose mode).
    Grep {
        /// Number of matches found.
        match_count: usize,
        /// The output mode used.
        output_mode: String,
        /// The result content.
        content: String,
    },

    /// Glob search result (verbose mode).
    Glob {
        /// Number of files found.
        file_count: usize,
        /// The matched file paths grouped by directory.
        content: String,
        /// Whether results were truncated.
        truncated: bool,
    },

    /// TodoWrite result (verbose mode).
    TodoWrite {
        /// Success or error message.
        message: Option<String>,
    },

    /// NotebookEdit result.
    NotebookEdit {
        /// Path to the notebook.
        notebook_path: String,
        /// Cell identifier.
        cell_identifier: String,
        /// Cell type (code or markdown).
        cell_type: Option<String>,
        /// Edit mode (replace, insert, delete).
        edit_mode: String,
        /// The unified diff of the cell change.
        diff_content: String,
    },

    /// Default result for other tools.
    Default {
        /// The result content (potentially truncated).
        content: Option<String>,
    },
}

impl OutputBlock {
    /// Create a text block from a parsed chunk.
    pub fn text(chunk: ParsedChunk) -> Self {
        Self::Text(TextBlock::from(chunk))
    }

    /// Create a separator block.
    pub fn separator() -> Self {
        Self::Separator
    }

    /// Create a tool invocation block.
    pub fn tool_invocation(tool_name: impl Into<String>, variant: ToolInvocationVariant) -> Self {
        Self::ToolInvocation(ToolInvocationBlock {
            tool_name: tool_name.into(),
            variant,
        })
    }

    /// Create a tool result block.
    pub fn tool_result(
        tool_name: impl Into<String>,
        is_error: bool,
        variant: ToolResultVariant,
    ) -> Self {
        Self::ToolResult(ToolResultBlock {
            tool_name: tool_name.into(),
            is_error,
            variant,
        })
    }
}

// =============================================================================
// Builders for complex variants
// =============================================================================

/// Builder for `ToolInvocationVariant::Grep`.
///
/// Provides a fluent API for constructing Grep invocation variants
/// with sensible defaults for optional fields.
///
/// # Example
///
/// ```
/// use ralph::stream_processor::GrepInvocationBuilder;
///
/// let variant = GrepInvocationBuilder::new("fn main")
///     .path("src/")
///     .output_mode("content")
///     .case_insensitive(true)
///     .build();
/// ```
#[derive(Debug, Default)]
pub struct GrepInvocationBuilder {
    pattern: String,
    path: Option<String>,
    output_mode: Option<String>,
    glob: Option<String>,
    file_type: Option<String>,
    case_insensitive: bool,
}

impl GrepInvocationBuilder {
    /// Create a new builder with the required pattern.
    pub fn new(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            ..Default::default()
        }
    }

    /// Set the search path.
    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Set the output mode (files_with_matches, content, count).
    pub fn output_mode(mut self, mode: impl Into<String>) -> Self {
        self.output_mode = Some(mode.into());
        self
    }

    /// Set the glob filter.
    pub fn glob(mut self, glob: impl Into<String>) -> Self {
        self.glob = Some(glob.into());
        self
    }

    /// Set the file type filter.
    pub fn file_type(mut self, file_type: impl Into<String>) -> Self {
        self.file_type = Some(file_type.into());
        self
    }

    /// Set case insensitivity.
    pub fn case_insensitive(mut self, value: bool) -> Self {
        self.case_insensitive = value;
        self
    }

    /// Build the `ToolInvocationVariant::Grep` variant.
    pub fn build(self) -> ToolInvocationVariant {
        ToolInvocationVariant::Grep {
            pattern: self.pattern,
            path: self.path,
            output_mode: self.output_mode,
            glob: self.glob,
            file_type: self.file_type,
            case_insensitive: self.case_insensitive,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_processor::test_helpers::assert_toml_roundtrip;
    use ralph_core::chunk::ChunkType;

    #[test]
    fn test_output_block_text_serialization() {
        let block = OutputBlock::text(ParsedChunk {
            chunk_type: ChunkType::Prose,
            content: "Hello, world!".to_string(),
        });
        assert_toml_roundtrip(&block);
    }

    #[test]
    fn test_output_block_separator_serialization() {
        let block = OutputBlock::separator();
        assert_toml_roundtrip(&block);
    }

    #[test]
    fn test_tool_invocation_bash_serialization() {
        let block = OutputBlock::tool_invocation(
            "Bash",
            ToolInvocationVariant::Bash {
                command: "ls -la".to_string(),
                description: Some("List files".to_string()),
            },
        );
        assert_toml_roundtrip(&block);
    }

    #[test]
    fn test_tool_result_bash_serialization() {
        let block = OutputBlock::tool_result(
            "Bash",
            false,
            ToolResultVariant::Bash {
                content: Some("file1.txt\nfile2.txt".to_string()),
                truncated: false,
            },
        );
        assert_toml_roundtrip(&block);
    }

    #[test]
    fn test_tool_result_edit_before_after_serialization() {
        let block = OutputBlock::tool_result(
            "Edit",
            false,
            ToolResultVariant::EditBeforeAfter {
                file_path: "test.rs".to_string(),
                old_content: "let x = 1;".to_string(),
                new_content: "let x = 2;".to_string(),
            },
        );
        assert_toml_roundtrip(&block);
    }

    #[test]
    fn test_tool_result_write_new_file_serialization() {
        let block = OutputBlock::tool_result(
            "Write",
            false,
            ToolResultVariant::WriteNewFile {
                file_path: "new.rs".to_string(),
                content: "fn main() {}".to_string(),
            },
        );
        assert_toml_roundtrip(&block);
    }

    #[test]
    fn test_tool_invocation_grep_serialization() {
        let block = OutputBlock::tool_invocation(
            "Grep",
            GrepInvocationBuilder::new("fn main")
                .path("src/")
                .output_mode("content")
                .glob("*.rs")
                .case_insensitive(true)
                .build(),
        );
        assert_toml_roundtrip(&block);
    }

    #[test]
    fn test_tool_invocation_todowrite_serialization() {
        let block = OutputBlock::tool_invocation(
            "TodoWrite",
            ToolInvocationVariant::TodoWrite {
                todos: vec![
                    TodoItem {
                        content: "Fix bug".to_string(),
                        status: "in_progress".to_string(),
                        active_form: Some("Fixing bug".to_string()),
                    },
                    TodoItem {
                        content: "Write tests".to_string(),
                        status: "pending".to_string(),
                        active_form: None,
                    },
                ],
            },
        );
        assert_toml_roundtrip(&block);
    }

    #[test]
    fn test_code_chunk_serialization() {
        let block = OutputBlock::text(ParsedChunk {
            chunk_type: ChunkType::Code {
                language: Some("rust".to_string()),
            },
            content: "fn main() {}".to_string(),
        });
        assert_toml_roundtrip(&block);
    }

    #[test]
    fn test_diff_chunk_serialization() {
        let block = OutputBlock::text(ParsedChunk {
            chunk_type: ChunkType::Diff,
            content: "+added\n-removed".to_string(),
        });
        assert_toml_roundtrip(&block);
    }

    // =============================================================================
    // Builder tests
    // =============================================================================

    #[test]
    fn test_grep_builder_minimal() {
        let variant = GrepInvocationBuilder::new("fn main").build();

        match variant {
            ToolInvocationVariant::Grep {
                pattern,
                path,
                output_mode,
                glob,
                file_type,
                case_insensitive,
            } => {
                assert_eq!(pattern, "fn main");
                assert!(path.is_none());
                assert!(output_mode.is_none());
                assert!(glob.is_none());
                assert!(file_type.is_none());
                assert!(!case_insensitive);
            }
            _ => panic!("Expected Grep variant"),
        }
    }

    #[test]
    fn test_grep_builder_all_options() {
        let variant = GrepInvocationBuilder::new("fn main")
            .path("src/")
            .output_mode("content")
            .glob("*.rs")
            .file_type("rust")
            .case_insensitive(true)
            .build();

        match variant {
            ToolInvocationVariant::Grep {
                pattern,
                path,
                output_mode,
                glob,
                file_type,
                case_insensitive,
            } => {
                assert_eq!(pattern, "fn main");
                assert_eq!(path, Some("src/".to_string()));
                assert_eq!(output_mode, Some("content".to_string()));
                assert_eq!(glob, Some("*.rs".to_string()));
                assert_eq!(file_type, Some("rust".to_string()));
                assert!(case_insensitive);
            }
            _ => panic!("Expected Grep variant"),
        }
    }

    #[test]
    fn test_grep_builder_serialization_roundtrip() {
        let variant = GrepInvocationBuilder::new("pattern")
            .path("src/")
            .case_insensitive(true)
            .build();
        let block = OutputBlock::tool_invocation("Grep", variant);
        assert_toml_roundtrip(&block);
    }
}
