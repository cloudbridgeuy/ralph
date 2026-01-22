//! Type definitions for the stream processor module.
//!
//! Contains configuration structs and result types used throughout
//! the stream processing pipeline.

use ralph_core::chunk::ParsedChunk;
use ralph_core::stream::{IterationCosts, IterationMetadata, ToolInteraction};
use std::collections::HashSet;

use super::output_block::OutputBlock;

/// Configuration for verbose tool output.
///
/// Controls which tools display verbose (full) output instead of truncated summaries.
/// Tool name matching is case-insensitive.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VerboseToolsConfig {
    /// If true, all tools are verbose
    all_verbose: bool,
    /// Set of tool names (lowercase) that should be verbose
    verbose_tools: HashSet<String>,
    /// Warnings about unknown tool names
    warnings: Vec<String>,
}

/// Known tool names for validation
const KNOWN_TOOLS: &[&str] = &[
    "read",
    "edit",
    "write",
    "glob",
    "grep",
    "bash",
    "task",
    "webfetch",
    "notebookedit",
    "todowrite",
    "websearch",
    "askuserquestion",
    "skill",
];

impl VerboseToolsConfig {
    /// Create a new empty config (no tools verbose).
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a config where all tools are verbose.
    pub fn all() -> Self {
        Self {
            all_verbose: true,
            verbose_tools: HashSet::new(),
            warnings: Vec::new(),
        }
    }

    /// Parse from a CLI argument value.
    ///
    /// - `None` -> no verbose tools
    /// - `Some("*")` -> all tools verbose
    /// - `Some("grep,bash")` -> specific tools verbose
    ///
    /// Returns the config and optionally warns about unknown tool names.
    pub fn from_arg(arg: Option<&str>) -> Self {
        match arg {
            None => Self::new(),
            Some("*") => Self::all(),
            Some(tools_str) => {
                let mut config = Self::new();
                let mut warnings = Vec::new();

                for tool in tools_str.split(',') {
                    let tool_lower = tool.trim().to_lowercase();
                    if tool_lower.is_empty() {
                        continue;
                    }

                    // Check if it's a known tool
                    if !KNOWN_TOOLS.contains(&tool_lower.as_str()) {
                        warnings.push(format!(
                            "Unknown tool name: '{}'. Known tools: {}",
                            tool,
                            KNOWN_TOOLS.join(", ")
                        ));
                    }

                    config.verbose_tools.insert(tool_lower);
                }

                config.warnings = warnings;
                config
            }
        }
    }

    /// Check if verbose output is enabled for a specific tool.
    ///
    /// Tool name matching is case-insensitive.
    pub fn is_verbose(&self, tool_name: &str) -> bool {
        if self.all_verbose {
            return true;
        }
        self.verbose_tools.contains(&tool_name.to_lowercase())
    }

    /// Get any warnings generated during parsing.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Check if any verbose tools are configured.
    pub fn has_any(&self) -> bool {
        self.all_verbose || !self.verbose_tools.is_empty()
    }
}

/// Result of processing a complete stream.
#[derive(Debug, Default)]
pub struct StreamProcessorResult {
    /// Parsed chunks from assistant output (prose, code, diff).
    pub chunks: Vec<ParsedChunk>,
    /// Metadata extracted from system init event.
    pub metadata: IterationMetadata,
    /// Costs extracted from result event.
    pub costs: IterationCosts,
    /// Tool interactions (calls correlated with results).
    pub tool_interactions: Vec<ToolInteraction>,
    /// Raw accumulated text (for completion marker detection).
    pub raw_text: String,
    /// Accumulated output blocks for replay serialization.
    ///
    /// Each block captures the data needed to re-render a piece of output.
    /// Blocks are ordered as they appeared during live execution.
    pub output_blocks: Vec<OutputBlock>,
}

/// Result of extracting a key argument from a tool invocation.
///
/// Contains the argument value and metadata about whether it should be
/// displayed in full (e.g., file paths) or truncated (e.g., file content).
#[derive(Debug, Clone, PartialEq)]
pub struct KeyArgument {
    /// The argument value.
    pub value: String,
    /// Whether this is a file path that should be shown in full.
    pub is_path: bool,
}

/// Snapshot of file content captured before an Edit tool execution.
///
/// Used to generate diffs by comparing the file content before and after
/// the Edit tool runs, since Claude CLI returns success messages rather
/// than diff content.
///
/// Contains both the full file content (for unified diff fallback) and
/// the old_string/new_string from the Edit tool input (for before/after display).
#[derive(Debug, Clone)]
pub struct EditSnapshot {
    /// Path to the file being edited.
    pub file_path: String,
    /// Content of the file before the edit (None if file didn't exist).
    pub content: Option<String>,
    /// The text being replaced (from Edit tool input).
    pub old_string: Option<String>,
    /// The replacement text (from Edit tool input).
    pub new_string: Option<String>,
}

/// Snapshot of file content captured before a Write tool execution.
///
/// Used to generate diffs by comparing the file content before and after
/// the Write tool runs. Unlike Edit, Write can create new files or completely
/// overwrite existing ones.
#[derive(Debug, Clone)]
pub struct WriteSnapshot {
    /// Path to the file being written.
    pub file_path: String,
    /// Content of the file before the write (None if file didn't exist).
    pub content: Option<String>,
    /// Whether the file existed before the write.
    pub file_existed: bool,
}

/// Snapshot of notebook cell content captured before a NotebookEdit tool execution.
///
/// Used to generate diffs by comparing the cell content before and after
/// the NotebookEdit tool runs. NotebookEdit can modify existing cells,
/// insert new cells, or delete cells in Jupyter notebooks.
#[derive(Debug, Clone)]
pub struct NotebookSnapshot {
    /// Path to the notebook file being edited.
    pub notebook_path: String,
    /// Cell identifier (cell_id if provided, otherwise stringified cell_number).
    pub cell_identifier: String,
    /// Content of the cell before the edit (None if cell didn't exist).
    pub content: Option<String>,
    /// The edit mode being performed (replace, insert, delete).
    pub edit_mode: String,
    /// The type of cell (code or markdown).
    pub cell_type: Option<String>,
}

/// Extracted parameters from a Grep tool invocation.
///
/// Used for both display formatting and output block building.
#[derive(Debug, Clone, Default)]
pub struct GrepParams {
    /// The search pattern (required).
    pub pattern: String,
    /// Search path (optional).
    pub path: Option<String>,
    /// Output mode: "files_with_matches", "content", or "count".
    pub output_mode: Option<String>,
    /// Glob filter pattern.
    pub glob: Option<String>,
    /// File type filter.
    pub file_type: Option<String>,
    /// Case insensitive flag.
    pub case_insensitive: bool,
}

impl GrepParams {
    /// Extract Grep parameters from a tool invocation's input.
    ///
    /// This is a pure function that extracts all relevant fields from
    /// the JSON input object.
    pub fn from_invocation_input(input: &serde_json::Value) -> Self {
        Self {
            pattern: input
                .get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            path: input.get("path").and_then(|v| v.as_str()).map(String::from),
            output_mode: input
                .get("output_mode")
                .and_then(|v| v.as_str())
                .map(String::from),
            glob: input.get("glob").and_then(|v| v.as_str()).map(String::from),
            file_type: input.get("type").and_then(|v| v.as_str()).map(String::from),
            case_insensitive: input.get("-i").and_then(|v| v.as_bool()).unwrap_or(false),
        }
    }
}

#[cfg(test)]
mod grep_params_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_grep_params_minimal() {
        let input = json!({"pattern": "fn main"});
        let params = GrepParams::from_invocation_input(&input);

        assert_eq!(params.pattern, "fn main");
        assert!(params.path.is_none());
        assert!(params.output_mode.is_none());
        assert!(params.glob.is_none());
        assert!(params.file_type.is_none());
        assert!(!params.case_insensitive);
    }

    #[test]
    fn test_grep_params_full() {
        let input = json!({
            "pattern": "fn main",
            "path": "src/",
            "output_mode": "content",
            "glob": "*.rs",
            "type": "rust",
            "-i": true
        });
        let params = GrepParams::from_invocation_input(&input);

        assert_eq!(params.pattern, "fn main");
        assert_eq!(params.path, Some("src/".to_string()));
        assert_eq!(params.output_mode, Some("content".to_string()));
        assert_eq!(params.glob, Some("*.rs".to_string()));
        assert_eq!(params.file_type, Some("rust".to_string()));
        assert!(params.case_insensitive);
    }

    #[test]
    fn test_grep_params_empty_input() {
        let input = json!({});
        let params = GrepParams::from_invocation_input(&input);

        assert_eq!(params.pattern, "");
        assert!(!params.case_insensitive);
    }
}
