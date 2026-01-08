//! Type definitions for the stream processor module.
//!
//! Contains configuration structs and result types used throughout
//! the stream processing pipeline.

use ralph_core::chunk::ParsedChunk;
use ralph_core::stream::{IterationCosts, IterationMetadata, ToolInteraction};
use std::collections::HashSet;

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
#[derive(Debug, Clone)]
pub struct EditSnapshot {
    /// Path to the file being edited.
    pub file_path: String,
    /// Content of the file before the edit (None if file didn't exist).
    pub content: Option<String>,
}
