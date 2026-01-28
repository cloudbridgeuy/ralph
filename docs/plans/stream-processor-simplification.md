# Stream Processor Simplification Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use executing-plans to implement this plan task-by-task.

**Goal:** Simplify the stream processor and rendering modules by extracting common patterns, consolidating duplicate code, and reducing constructor proliferation.

**Architecture:** Follow Functional Core - Imperative Shell pattern. Extract pure utility functions for common operations (format_duration, RenderContext creation, grep parameter extraction). Use builder pattern for StreamProcessor construction. Keep all display formatting as pure functions with I/O handled by the imperative shell.

**Tech Stack:** Rust, serde, syntect (highlighting), termimad (markdown)

---

## Task 1: Consolidate `format_duration` into Shared Utility

Three identical implementations exist:
- `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/iterations.rs:387`
- `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/sessions_display.rs:134`
- `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/startup/formatters.rs:20`

### Step 1.1: Create shared formatting module

Create file `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/formatting.rs`:

```rust
//! Shared pure formatting functions (Functional Core).
//!
//! This module contains stateless formatting utilities used across
//! multiple modules. All functions are pure with no side effects.

/// Format duration from milliseconds to human-readable string.
///
/// # Formatting rules
/// - 0-999ms -> "Xms"
/// - 1000-59999ms -> "X.Xs" (e.g., "45.2s")
/// - 60000+ ms -> "Xm Ys" (e.g., "1m 23s")
///
/// # Examples
///
/// ```
/// use ralph::formatting::format_duration;
///
/// assert_eq!(format_duration(500), "500ms");
/// assert_eq!(format_duration(1500), "1.5s");
/// assert_eq!(format_duration(125000), "2m 5s");
/// ```
pub fn format_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        let minutes = ms / 60_000;
        let seconds = (ms % 60_000) / 1000;
        format!("{}m {}s", minutes, seconds)
    }
}

/// Format token count with thousands suffix for readability.
///
/// # Examples
///
/// ```
/// use ralph::formatting::format_token_count;
///
/// assert_eq!(format_token_count(500), "500");
/// assert_eq!(format_token_count(1500), "1.5K");
/// assert_eq!(format_token_count(1_500_000), "1.50M");
/// ```
pub fn format_token_count(tokens: u64) -> String {
    if tokens < 1000 {
        tokens.to_string()
    } else if tokens < 1_000_000 {
        format!("{:.1}K", tokens as f64 / 1000.0)
    } else {
        format!("{:.2}M", tokens as f64 / 1_000_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration_milliseconds() {
        assert_eq!(format_duration(0), "0ms");
        assert_eq!(format_duration(500), "500ms");
        assert_eq!(format_duration(999), "999ms");
    }

    #[test]
    fn test_format_duration_seconds() {
        assert_eq!(format_duration(1000), "1.0s");
        assert_eq!(format_duration(1500), "1.5s");
        assert_eq!(format_duration(45200), "45.2s");
        assert_eq!(format_duration(59999), "60.0s");
    }

    #[test]
    fn test_format_duration_minutes() {
        assert_eq!(format_duration(60_000), "1m 0s");
        assert_eq!(format_duration(125_000), "2m 5s");
        assert_eq!(format_duration(3_723_000), "62m 3s");
    }

    #[test]
    fn test_format_token_count_under_thousand() {
        assert_eq!(format_token_count(0), "0");
        assert_eq!(format_token_count(500), "500");
        assert_eq!(format_token_count(999), "999");
    }

    #[test]
    fn test_format_token_count_thousands() {
        assert_eq!(format_token_count(1000), "1.0K");
        assert_eq!(format_token_count(1500), "1.5K");
        assert_eq!(format_token_count(999_999), "1000.0K");
    }

    #[test]
    fn test_format_token_count_millions() {
        assert_eq!(format_token_count(1_000_000), "1.00M");
        assert_eq!(format_token_count(1_500_000), "1.50M");
    }
}
```

### Step 1.2: Register module in main.rs

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/main.rs`:

Add after line 17 (`pub mod markdown;`):
```rust
pub mod formatting;
```

### Step 1.3: Run tests for new module

```bash
cargo test --package ralph formatting::tests
```

Expected output: All 6 tests pass.

### Step 1.4: Update iterations.rs to use shared function

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/iterations.rs`:

1. Add import at top with other use statements:
```rust
use crate::formatting::format_duration;
```

2. Delete the local `format_duration` function (lines 386-397).

### Step 1.5: Update sessions_display.rs to use shared function

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/sessions_display.rs`:

1. Change import from local to shared:
```rust
use crate::formatting::format_duration;
```

2. Delete the local `format_duration` function (lines 127-144) AND its doc comment.

3. Keep `format_token_count` if it differs or also migrate it.

### Step 1.6: Update startup/formatters.rs

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/startup/formatters.rs`:

1. Replace entire file contents with:
```rust
//! Pure formatting functions for display values.

// Re-export shared formatting functions for backward compatibility
pub use crate::formatting::{format_duration, format_token_count};
```

### Step 1.7: Run full test suite

```bash
cargo test --package ralph
```

Expected: All existing tests continue to pass.

---

## Task 2: Add `render_context()` Helper to StreamProcessor

The pattern `if processor.highlighting_enabled { RenderContext::terminal(...) } else { RenderContext::plain(...) }` appears 15+ times.

### Step 2.1: Add helper method to StreamProcessor

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/stream_processor/processor.rs`:

Add this method after `is_tool_verbose` (around line 390):

```rust
    /// Create a render context based on the processor's highlighting setting.
    ///
    /// Returns a terminal context (with ANSI codes) when highlighting is enabled,
    /// or a plain context otherwise.
    pub fn render_context(&self) -> crate::render::RenderContext<'_> {
        if self.highlighting_enabled {
            crate::render::RenderContext::terminal(&self.code_highlighter)
        } else {
            crate::render::RenderContext::plain(&self.code_highlighter)
        }
    }
```

### Step 2.2: Run tests to verify method compiles

```bash
cargo test --package ralph stream_processor
```

### Step 2.3: Update tool_display/bash.rs

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/stream_processor/tool_display/bash.rs`:

Replace lines 28-32:
```rust
    let ctx = if processor.highlighting_enabled {
        RenderContext::terminal(&processor.code_highlighter)
    } else {
        RenderContext::plain(&processor.code_highlighter)
    };
```

With:
```rust
    let ctx = processor.render_context();
```

Remove unused import `RenderContext` from the use statement if no longer needed directly.

### Step 2.4: Update tool_display/grep.rs

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/stream_processor/tool_display/grep.rs`:

Replace lines 52-56 with:
```rust
    let ctx = processor.render_context();
```

Remove `RenderContext` from the use statement.

### Step 2.5: Update tool_display/glob.rs

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/stream_processor/tool_display/glob.rs`:

Replace lines 31-35 with:
```rust
    let ctx = processor.render_context();
```

### Step 2.6: Update tool_display/read.rs

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/stream_processor/tool_display/read.rs`:

Replace lines 32-36 with:
```rust
    let ctx = processor.render_context();
```

### Step 2.7: Update tool_display/todowrite.rs

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/stream_processor/tool_display/todowrite.rs`:

Replace lines 39-43 with:
```rust
    let ctx = processor.render_context();
```

### Step 2.8: Update tool_results/bash.rs

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/stream_processor/tool_results/bash.rs`:

Replace lines 30-34 with:
```rust
    let ctx = processor.render_context();
```

### Step 2.9: Update tool_results/grep.rs

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/stream_processor/tool_results/grep.rs`:

Replace lines 50-54 with:
```rust
    let ctx = processor.render_context();
```

### Step 2.10: Update tool_results/glob.rs

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/stream_processor/tool_results/glob.rs`:

Replace lines 42-46 with:
```rust
    let ctx = processor.render_context();
```

### Step 2.11: Update tool_results/read.rs

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/stream_processor/tool_results/read.rs`:

Replace lines 54-58 with:
```rust
    let ctx = processor.render_context();
```

### Step 2.12: Update tool_results/todowrite.rs

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/stream_processor/tool_results/todowrite.rs`:

Replace lines 30-34 with:
```rust
    let ctx = processor.render_context();
```

### Step 2.13: Run full test suite

```bash
cargo test --package ralph
```

Expected: All tests pass.

---

## Task 3: Extract Grep Parameter Extraction

Grep parameter extraction is duplicated in:
- `stream_processor/tool_display/grep.rs` (lines 21-39)
- `stream_processor/block_builders.rs` (lines 37-63)

### Step 3.1: Create GrepParams struct in stream_processor/types.rs

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/stream_processor/types.rs`:

Add after the existing structs:

```rust
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
            case_insensitive: input
                .get("-i")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        }
    }
}
```

### Step 3.2: Export GrepParams from stream_processor/mod.rs

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/stream_processor/mod.rs`:

Add `GrepParams` to the test re-exports:
```rust
#[cfg(test)]
pub use types::{GrepParams, KeyArgument};
```

### Step 3.3: Add tests for GrepParams

Add to the test section in `types.rs` or create a new test module:

```rust
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
```

### Step 3.4: Run tests for GrepParams

```bash
cargo test --package ralph grep_params
```

### Step 3.5: Update tool_display/grep.rs to use GrepParams

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/stream_processor/tool_display/grep.rs`:

Replace parameter extraction with:
```rust
use super::super::types::GrepParams;

pub fn format_grep_tool_invocation_verbose(
    processor: &StreamProcessor,
    invocation: &ToolInvocation,
) -> String {
    let params = GrepParams::from_invocation_input(&invocation.input);

    // Build params struct for shared renderer
    let render_params = GrepInvocationParams {
        pattern: &params.pattern,
        path: params.path.as_deref(),
        output_mode: params.output_mode.as_deref(),
        glob: params.glob.as_deref(),
        file_type: params.file_type.as_deref(),
        case_insensitive: params.case_insensitive,
    };

    let ctx = processor.render_context();
    render_grep_invocation(&ctx, &render_params)
}
```

### Step 3.6: Update block_builders.rs to use GrepParams

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/stream_processor/block_builders.rs`:

In `build_tool_invocation_block`, replace the "Grep" match arm:
```rust
        "Grep" => {
            let params = super::types::GrepParams::from_invocation_input(&invocation.input);

            let mut builder = GrepInvocationBuilder::new(&params.pattern);
            if let Some(path) = &params.path {
                builder = builder.path(path);
            }
            if let Some(mode) = &params.output_mode {
                builder = builder.output_mode(mode);
            }
            if let Some(glob) = &params.glob {
                builder = builder.glob(glob);
            }
            if let Some(ft) = &params.file_type {
                builder = builder.file_type(ft);
            }
            if params.case_insensitive {
                builder = builder.case_insensitive(true);
            }

            builder.build()
        }
```

### Step 3.7: Run tests

```bash
cargo test --package ralph
```

---

## Task 4: Fix ThemeConfig Double-Load Issue

In `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/highlight.rs:132-156`, `AppConfig::load()` is called twice.

### Step 4.1: Fix from_config_and_env method

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/highlight.rs`:

Replace the `from_config_and_env` method (lines 132-156):

```rust
    /// Create a theme configuration from config file and environment variables.
    ///
    /// Loads configuration in this order (higher priority sources override lower):
    /// 1. Environment variables (`RALPH_THEME`, `RALPH_NO_BACKGROUND`)
    /// 2. Config file (`~/.config/ralph/config.toml`)
    /// 3. Default values
    ///
    /// If the config file doesn't exist or fails to parse, falls back to env + defaults.
    pub fn from_config_and_env() -> Self {
        // Load config file once
        let app_config = crate::config::AppConfig::load().ok();

        // Extract config file settings
        let config_theme = app_config.as_ref().and_then(|c| c.theme.name.clone());
        let config_no_background = app_config.map(|c| c.theme.no_background).unwrap_or(false);

        // Get environment variable values
        let env_theme = std::env::var(RALPH_THEME_ENV).ok();
        let env_no_background = std::env::var(RALPH_NO_BACKGROUND_ENV)
            .map(|v| !v.is_empty())
            .unwrap_or(false);

        // Env vars take precedence over config file
        let theme = env_theme.or(config_theme);
        let no_background = env_no_background || config_no_background;

        Self {
            theme,
            no_background,
        }
    }
```

### Step 4.2: Run tests

```bash
cargo test --package ralph highlight
```

---

## Task 5: Extract `handle_event` Match Arms into Dedicated Handlers

The `handle_event` method in `event_handler.rs` is 230+ lines with a massive match statement. Extract tool result building into separate handler functions.

### Step 5.1: Create result_handlers.rs module

Create file `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/stream_processor/result_handlers.rs`:

```rust
//! Tool result handling functions (Functional Core).
//!
//! Each function handles a specific tool type's result processing,
//! returning both the formatted output and the output block for replay.

use ralph_core::stream::{ToolInvocation, ToolResult};

use super::block_builders::{
    build_bash_result_block, build_default_result_block, build_edit_before_after_block,
    build_edit_diff_block, build_glob_result_block, build_grep_result_block,
    build_notebook_edit_block, build_read_result_block, build_todowrite_result_block,
    build_write_result_block,
};
use super::output_block::OutputBlock;
use super::processor::StreamProcessor;
use super::tool_results;
use super::types::{EditSnapshot, NotebookSnapshot, WriteSnapshot};
use super::utils::{count_non_empty_lines, is_content_truncated};

/// Result of processing a tool result.
pub struct ToolResultOutput {
    /// Formatted string for terminal display.
    pub formatted: String,
    /// Output block for replay serialization.
    pub block: OutputBlock,
}

/// Handle Edit tool result with snapshot.
pub fn handle_edit_result(
    processor: &StreamProcessor,
    result: &ToolResult,
    invocation: &ToolInvocation,
    snapshot: Option<&EditSnapshot>,
) -> ToolResultOutput {
    if let Some(snap) = snapshot {
        let has_diff_content = result
            .content
            .as_ref()
            .map(|c| ralph_core::chunk::is_unfenced_diff(c))
            .unwrap_or(false);

        if has_diff_content {
            // Result contains diff - use diff formatting and block
            let formatted = tool_results::format_tool_result_with_context(
                processor,
                result,
                Some(invocation.clone()),
            );
            let block = build_edit_diff_block(
                &snap.file_path,
                result.content.as_deref().unwrap_or(""),
            );
            ToolResultOutput { formatted, block }
        } else {
            // No diff in result - generate from snapshot
            let formatted =
                tool_results::format_edit_result_with_snapshot(processor, snap.clone());
            let block = build_edit_before_after_block(snap);
            ToolResultOutput { formatted, block }
        }
    } else {
        handle_default_result(processor, result, Some(invocation))
    }
}

/// Handle Write tool result with snapshot.
pub fn handle_write_result(
    processor: &StreamProcessor,
    result: &ToolResult,
    invocation: &ToolInvocation,
    snapshot: Option<&WriteSnapshot>,
) -> ToolResultOutput {
    if let Some(snap) = snapshot {
        let formatted =
            tool_results::format_write_result_with_snapshot(processor, snap.clone());
        // Read the new file content to determine which variant to use
        let new_content = std::fs::read_to_string(&snap.file_path).ok();
        let block = build_write_result_block(snap, new_content.as_deref());
        ToolResultOutput { formatted, block }
    } else {
        handle_default_result(processor, result, Some(invocation))
    }
}

/// Handle NotebookEdit tool result with snapshot.
pub fn handle_notebook_result(
    processor: &StreamProcessor,
    result: &ToolResult,
    invocation: &ToolInvocation,
    snapshot: Option<&NotebookSnapshot>,
) -> ToolResultOutput {
    if let Some(snap) = snapshot {
        let formatted =
            tool_results::format_notebook_result_with_snapshot(processor, snap.clone());
        let new_source = invocation
            .input
            .get("new_source")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let block = build_notebook_edit_block(snap, new_source);
        ToolResultOutput { formatted, block }
    } else {
        handle_default_result(processor, result, Some(invocation))
    }
}

/// Handle Bash tool result.
pub fn handle_bash_result(
    processor: &StreamProcessor,
    result: &ToolResult,
    invocation: Option<&ToolInvocation>,
) -> ToolResultOutput {
    let formatted = tool_results::format_tool_result_with_context(
        processor,
        result,
        invocation.cloned(),
    );
    let block = build_bash_result_block(result.content.as_deref(), result.is_error);
    ToolResultOutput { formatted, block }
}

/// Handle Read tool result.
pub fn handle_read_result(
    processor: &StreamProcessor,
    result: &ToolResult,
    invocation: &ToolInvocation,
) -> ToolResultOutput {
    let formatted = tool_results::format_tool_result_with_context(
        processor,
        result,
        Some(invocation.clone()),
    );
    let content = result.content.as_deref().unwrap_or("");
    let line_count = content.lines().count();
    let truncated = is_content_truncated(content);
    let file_path = invocation
        .input
        .get("file_path")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let block = build_read_result_block(file_path, content, line_count, truncated);
    ToolResultOutput { formatted, block }
}

/// Handle Grep tool result.
pub fn handle_grep_result(
    processor: &StreamProcessor,
    result: &ToolResult,
    invocation: &ToolInvocation,
) -> ToolResultOutput {
    let formatted = tool_results::format_tool_result_with_context(
        processor,
        result,
        Some(invocation.clone()),
    );
    let content = result.content.as_deref().unwrap_or("");
    let match_count = count_non_empty_lines(content);
    let output_mode = invocation
        .input
        .get("output_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("files_with_matches");
    let block = build_grep_result_block(match_count, output_mode, content);
    ToolResultOutput { formatted, block }
}

/// Handle Glob tool result.
pub fn handle_glob_result(
    processor: &StreamProcessor,
    result: &ToolResult,
    invocation: &ToolInvocation,
) -> ToolResultOutput {
    let formatted = tool_results::format_tool_result_with_context(
        processor,
        result,
        Some(invocation.clone()),
    );
    let content = result.content.as_deref().unwrap_or("");
    let file_count = count_non_empty_lines(content);
    let truncated = is_content_truncated(content);
    let block = build_glob_result_block(file_count, content, truncated);
    ToolResultOutput { formatted, block }
}

/// Handle TodoWrite tool result.
pub fn handle_todowrite_result(
    processor: &StreamProcessor,
    result: &ToolResult,
    invocation: Option<&ToolInvocation>,
) -> ToolResultOutput {
    let formatted = tool_results::format_tool_result_with_context(
        processor,
        result,
        invocation.cloned(),
    );
    let block = build_todowrite_result_block(result.content.as_deref());
    ToolResultOutput { formatted, block }
}

/// Handle default tool result (unknown tools or errors).
pub fn handle_default_result(
    processor: &StreamProcessor,
    result: &ToolResult,
    invocation: Option<&ToolInvocation>,
) -> ToolResultOutput {
    let formatted = tool_results::format_tool_result_with_context(
        processor,
        result,
        invocation.cloned(),
    );
    let tool_name = invocation
        .map(|i| i.name.as_str())
        .unwrap_or("Unknown");
    let block = build_default_result_block(tool_name, result.content.as_deref(), result.is_error);
    ToolResultOutput { formatted, block }
}
```

### Step 5.2: Register module in stream_processor/mod.rs

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/stream_processor/mod.rs`:

Add after `mod event_handler;`:
```rust
mod result_handlers;
```

### Step 5.3: Refactor handle_event to use handlers

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/stream_processor/event_handler.rs`:

Add import at top:
```rust
use super::result_handlers::{
    handle_bash_result, handle_default_result, handle_edit_result, handle_glob_result,
    handle_grep_result, handle_notebook_result, handle_read_result, handle_todowrite_result,
    handle_write_result,
};
```

Replace the large match statement in the User event handler (lines 172-361) with:

```rust
                        let output = match &invocation {
                            Some(inv) if inv.name == "Edit" && !result.is_error => {
                                handle_edit_result(self, result, inv, edit_snapshot.as_ref())
                            }
                            Some(inv) if inv.name == "Write" && !result.is_error => {
                                handle_write_result(self, result, inv, write_snapshot.as_ref())
                            }
                            Some(inv) if inv.name == "NotebookEdit" && !result.is_error => {
                                handle_notebook_result(self, result, inv, notebook_snapshot.as_ref())
                            }
                            Some(inv) if inv.name == "Bash" => {
                                handle_bash_result(self, result, Some(inv))
                            }
                            Some(inv) if inv.name == "Read" && !result.is_error => {
                                handle_read_result(self, result, inv)
                            }
                            Some(inv) if inv.name == "Grep" && !result.is_error => {
                                handle_grep_result(self, result, inv)
                            }
                            Some(inv) if inv.name == "Glob" && !result.is_error => {
                                handle_glob_result(self, result, inv)
                            }
                            Some(inv) if inv.name == "TodoWrite" => {
                                handle_todowrite_result(self, result, Some(inv))
                            }
                            _ => {
                                handle_default_result(self, result, invocation.as_ref())
                            }
                        };
                        output_parts.push(output.formatted);
                        self.output_blocks.push(output.block);
```

Remove now-unused imports:
- `build_*` functions that are now only used in result_handlers
- Direct block builder imports

### Step 5.4: Run tests

```bash
cargo test --package ralph stream_processor
```

---

## Task 6: Implement Builder Pattern for StreamProcessor

The StreamProcessor has 7 constructors. Replace with a builder pattern.

### Step 6.1: Create StreamProcessorBuilder

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/stream_processor/processor.rs`:

Add after the `StreamProcessor` struct definition (around line 87):

```rust
/// Builder for constructing StreamProcessor instances.
///
/// Provides a fluent API for configuring all StreamProcessor options
/// with sensible defaults.
///
/// # Example
///
/// ```
/// use ralph::stream_processor::StreamProcessorBuilder;
/// use ralph::highlight::ThemeConfig;
///
/// let processor = StreamProcessorBuilder::new()
///     .highlighting(true)
///     .show_tools(true)
///     .theme_config(ThemeConfig::new().with_theme("Monokai Extended"))
///     .build()
///     .unwrap();
/// ```
#[derive(Debug, Default)]
pub struct StreamProcessorBuilder {
    highlighting: Option<bool>,
    show_tools: Option<bool>,
    theme_config: Option<ThemeConfig>,
    verbose_tools: Option<VerboseToolsConfig>,
}

impl StreamProcessorBuilder {
    /// Create a new builder with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set whether to enable syntax highlighting.
    ///
    /// If not set, defaults to terminal detection (enabled if stdout is a terminal).
    pub fn highlighting(mut self, enabled: bool) -> Self {
        self.highlighting = Some(enabled);
        self
    }

    /// Set whether to display tool invocations.
    ///
    /// If not set, defaults to the highlighting setting.
    pub fn show_tools(mut self, enabled: bool) -> Self {
        self.show_tools = Some(enabled);
        self
    }

    /// Set the theme configuration for syntax highlighting.
    pub fn theme_config(mut self, config: ThemeConfig) -> Self {
        self.theme_config = Some(config);
        self
    }

    /// Set the verbose tools configuration.
    pub fn verbose_tools(mut self, config: VerboseToolsConfig) -> Self {
        self.verbose_tools = Some(config);
        self
    }

    /// Build the StreamProcessor.
    ///
    /// # Returns
    ///
    /// * `Ok(StreamProcessor)` - Successfully configured processor
    /// * `Err(ThemeError)` - If the theme configuration is invalid
    pub fn build(self) -> Result<StreamProcessor, ThemeError> {
        let is_terminal = std::io::stdout().is_terminal();

        // Determine highlighting: explicit setting > terminal detection
        let highlighting_enabled = self.highlighting.unwrap_or(is_terminal);

        // Determine show_tools: explicit setting > highlighting setting
        let show_tool_invocations = self.show_tools.unwrap_or(highlighting_enabled);

        // Build highlighter
        let code_highlighter = match self.theme_config {
            Some(config) => Highlighter::with_config(config)?,
            None => Highlighter::new(),
        };

        // Get verbose tools config
        let verbose_tools_config = self.verbose_tools.unwrap_or_default();

        Ok(StreamProcessor {
            events: Vec::new(),
            text_buffer: String::new(),
            chunk_buffer: StreamingChunkBuffer::new(),
            code_highlighter,
            markdown_renderer: MarkdownRenderer::new(),
            highlighting_enabled,
            show_tool_invocations,
            current_message_id: None,
            collected_chunks: Vec::new(),
            parse_errors: Vec::new(),
            tool_correlator: ToolCorrelator::new(),
            has_emitted_output: false,
            response_count: 0,
            pending_invocations: HashMap::new(),
            verbose_tools_config,
            pending_edit_snapshots: HashMap::new(),
            pending_write_snapshots: HashMap::new(),
            pending_notebook_snapshots: HashMap::new(),
            output_blocks: Vec::new(),
        })
    }
}
```

### Step 6.2: Update existing constructors to use builder

Keep `new()` and `with_highlighting()` for backward compatibility, but implement using builder:

```rust
impl StreamProcessor {
    /// Create a new stream processor.
    ///
    /// Automatically detects terminal support for highlighting.
    /// Tool invocations are displayed by default when highlighting is enabled.
    pub fn new() -> Self {
        // Builder with defaults will detect terminal
        StreamProcessorBuilder::new()
            .build()
            .unwrap_or_else(|_| {
                // Fallback: this should never happen with default config
                Self::fallback_new()
            })
    }

    /// Fallback constructor when builder fails (should not happen with defaults).
    fn fallback_new() -> Self {
        let is_terminal = std::io::stdout().is_terminal();
        Self {
            events: Vec::new(),
            text_buffer: String::new(),
            chunk_buffer: StreamingChunkBuffer::new(),
            code_highlighter: Highlighter::new(),
            markdown_renderer: MarkdownRenderer::new(),
            highlighting_enabled: is_terminal,
            show_tool_invocations: is_terminal,
            current_message_id: None,
            collected_chunks: Vec::new(),
            parse_errors: Vec::new(),
            tool_correlator: ToolCorrelator::new(),
            has_emitted_output: false,
            response_count: 0,
            pending_invocations: HashMap::new(),
            verbose_tools_config: VerboseToolsConfig::new(),
            pending_edit_snapshots: HashMap::new(),
            pending_write_snapshots: HashMap::new(),
            pending_notebook_snapshots: HashMap::new(),
            output_blocks: Vec::new(),
        }
    }

    /// Create a processor with highlighting explicitly enabled/disabled.
    pub fn with_highlighting(enabled: bool) -> Self {
        StreamProcessorBuilder::new()
            .highlighting(enabled)
            .build()
            .unwrap_or_else(|_| Self::fallback_new())
    }

    /// Create a processor with custom settings.
    pub fn with_options(highlighting: bool, show_tools: bool) -> Self {
        StreamProcessorBuilder::new()
            .highlighting(highlighting)
            .show_tools(show_tools)
            .build()
            .unwrap_or_else(|_| Self::fallback_new())
    }
}
```

### Step 6.3: Mark old constructors as deprecated (optional)

Add deprecation notices to guide users toward the builder:

```rust
    /// Create a processor with full configuration.
    ///
    /// # Deprecated
    ///
    /// Use `StreamProcessorBuilder` instead for clearer configuration.
    #[deprecated(since = "0.x.0", note = "Use StreamProcessorBuilder instead")]
    pub fn with_full_config(...) { ... }
```

### Step 6.4: Export builder from mod.rs

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/stream_processor/mod.rs`:

Update the public exports:
```rust
pub use processor::{StreamProcessor, StreamProcessorBuilder};
```

### Step 6.5: Run tests

```bash
cargo test --package ralph stream_processor
```

---

## Task 7: Document ToolResultVariant Pattern (Low Priority)

The 12 variants in ToolResultVariant may be acceptable complexity. Add documentation explaining the pattern.

### Step 7.1: Add comprehensive module documentation

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/stream_processor/output_block.rs`:

Expand the module doc comment (lines 1-14):

```rust
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
//!
//! # Variant Count Rationale
//!
//! `ToolResultVariant` has 12 variants, one for each tool type that requires
//! specialized result handling. This is intentional:
//!
//! - **Type safety**: Each variant enforces the correct data structure for its tool
//! - **Exhaustive matching**: Adding a new tool requires handling in all renderers
//! - **Serialization stability**: Each variant serializes to a distinct TOML structure
//!
//! The variants are:
//!
//! | Variant | Tool | Purpose |
//! |---------|------|---------|
//! | `Bash` | Bash | Command output with truncation indicator |
//! | `EditBeforeAfter` | Edit | Before/after text replacement |
//! | `EditDiff` | Edit | Unified diff format |
//! | `EditNoChanges` | Edit | No-op indicator |
//! | `WriteNewFile` | Write | New file creation |
//! | `WriteOverwrite` | Write | File overwrite with diff |
//! | `WriteNoChanges` | Write | No-op indicator |
//! | `Read` | Read | File content with metadata |
//! | `Grep` | Grep | Search results with match count |
//! | `Glob` | Glob | File list with truncation |
//! | `TodoWrite` | TodoWrite | Task list confirmation |
//! | `NotebookEdit` | NotebookEdit | Jupyter cell diff |
//! | `Default` | Other | Generic content display |
//!
//! Adding a new tool variant requires:
//! 1. Add variant to `ToolResultVariant`
//! 2. Add builder in `block_builders.rs`
//! 3. Add handler in `result_handlers.rs`
//! 4. Add renderer in `render/tool_renderers/results/`
```

### Step 7.2: Run cargo doc to verify

```bash
cargo doc --package ralph --no-deps
```

---

## Verification Checklist

After completing all tasks, run the full verification suite:

```bash
# Format check
cargo fmt --check

# Lint check
cargo clippy --all-targets --all-features -- -D warnings

# All tests
cargo test --workspace

# Documentation build
cargo doc --no-deps
```

All commands should pass without errors or warnings.

---

**Plan complete and saved to `/Users/guzmanmonne/Projects/Rust/ralph/docs/plans/stream-processor-simplification.md`. Two execution options:**

**1. Subagent-Driven (this session)** - I dispatch fresh subagent per task, review between tasks, fast iteration

**2. Parallel Session (separate)** - Open new session with executing-plans, batch execution with checkpoints

**Which approach?**
