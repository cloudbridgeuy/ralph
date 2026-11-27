//! Shared tool rendering functions (Functional Core).
//!
//! This module contains pure rendering functions used by the stream processor
//! during live execution. Functions accept data parameters directly
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
pub use context::{ansi, Diagrams, RenderContext, SaveDiagram, Viewport};

// Re-export types
pub use types::{GrepInvocationParams, NotebookEditParams, ScriptResultParams, TodoDisplayItem};

// Re-export invocation renderers
pub use invocations::{
    render_bash_invocation, render_default_invocation, render_glob_invocation,
    render_grep_invocation, render_read_invocation, render_script_invocation,
    render_todowrite_invocation,
};

// Re-export result renderers
pub use results::{
    render_bash_result, render_default_result, render_glob_result, render_grep_result,
    render_read_result, render_script_result, render_todowrite_result,
};
