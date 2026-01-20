//! Shared tool rendering functions (Functional Core).
//!
//! This module contains pure rendering functions used by both stream processor
//! (live execution) and replay renderer. Functions accept data parameters directly
//! and return formatted strings, avoiding any I/O operations.
//!
//! # Design Principles
//!
//! 1. **Pure Functions**: All functions are stateless and free of side effects
//! 2. **Data-Centric**: Accept structured data, not processor/renderer references
//! 3. **Dual Mode**: Support both terminal (ANSI) and plain text output
//! 4. **Single Source of Truth**: One implementation for both rendering paths
//!
//! # Usage
//!
//! ```ignore
//! use ralph::render::{render_bash_invocation, RenderContext};
//!
//! let ctx = RenderContext::terminal(&highlighter);
//! let output = render_bash_invocation(&ctx, "ls -la", false);
//! ```

mod context;
mod invocations;
mod results;
mod types;

#[cfg(test)]
mod tests;

// Re-export context
pub use context::{ansi, RenderContext};

// Re-export types
pub use types::{GrepInvocationParams, NotebookEditParams, TodoDisplayItem};

// Re-export invocation renderers
pub use invocations::{
    render_bash_invocation, render_default_invocation, render_glob_invocation,
    render_grep_invocation, render_read_invocation, render_todowrite_invocation,
};

// Re-export result renderers
pub use results::{
    render_bash_result, render_content_block, render_default_result, render_edit_before_after,
    render_edit_diff, render_glob_result, render_grep_result, render_no_changes_message,
    render_notebook_edit, render_read_result, render_todowrite_result, render_write_new_file,
    render_write_no_changes,
};
