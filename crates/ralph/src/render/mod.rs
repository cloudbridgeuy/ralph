//! Shared rendering module for OutputBlock display.
//!
//! This module provides unified rendering utilities used by both live execution
//! (stream processor) and replay. By consolidating these functions, we ensure
//! consistent output formatting across both rendering paths.
//!
//! # Module Structure
//!
//! - `utils`: Pure utility functions for line normalization, file grouping, etc.
//! - `tool_renderers`: Shared tool rendering functions for invocations and results
//!
//! # Design Principles
//!
//! 1. **Pure Functions**: All utilities are stateless and free of side effects
//! 2. **Shared Logic**: Functions used by both stream processor and replay
//! 3. **Single Source of Truth**: No duplication between rendering paths
//! 4. **Data-Centric**: Renderers accept structured data, not processor references

pub mod tool_renderers;
pub mod utils;

// Re-export commonly used functions for convenience
pub use utils::{
    extract_language_from_path, extract_line_number, group_files_by_directory,
    highlight_grep_match, normalize_cat_n_format,
};

// Re-export tool renderer components
pub use tool_renderers::{
    ansi, render_bash_invocation, render_bash_result, render_content_block,
    render_default_invocation, render_default_result, render_edit_before_after, render_edit_diff,
    render_glob_invocation, render_glob_result, render_grep_invocation, render_grep_result,
    render_no_changes_message, render_notebook_edit, render_read_invocation, render_read_result,
    render_todowrite_invocation, render_todowrite_result, render_write_new_file,
    render_write_no_changes, GrepInvocationParams, NotebookEditParams, RenderContext,
    TodoDisplayItem,
};
