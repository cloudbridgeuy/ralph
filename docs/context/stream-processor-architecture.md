# Stream Processor Architecture

The `stream_processor` module handles real-time parsing and rendering of Claude CLI's `--output-format stream-json` output. This document covers its internal architecture and patterns.

## Module Structure

```
stream_processor/
├── mod.rs                 # Module exports
├── processor.rs           # StreamProcessor struct and builder
├── event_handler.rs       # Event processing logic
├── result_handlers.rs     # Tool result handler functions
├── output_block.rs        # Output block types for replay
├── block_builders.rs      # Functions to build output blocks
├── tool_display/          # Tool invocation formatters
│   ├── mod.rs
│   ├── bash.rs
│   ├── grep.rs
│   ├── glob.rs
│   ├── read.rs
│   └── todowrite.rs
├── tool_results/          # Tool result formatters
│   ├── mod.rs
│   ├── bash.rs
│   ├── edit.rs
│   ├── grep.rs
│   ├── glob.rs
│   ├── read.rs
│   ├── todowrite.rs
│   └── write.rs
├── types.rs               # Configuration and type definitions
└── utils.rs               # Utility functions
```

## Key Patterns

### StreamProcessorBuilder

Use the builder pattern for constructing `StreamProcessor` instances:

```rust
use ralph::stream_processor::StreamProcessorBuilder;
use ralph::highlight::ThemeConfig;

let processor = StreamProcessorBuilder::new()
    .highlighting(true)
    .show_tools(true)
    .theme_config(ThemeConfig::new().with_theme("Monokai Extended"))
    .verbose_tools(VerboseToolsConfig::all())
    .build()?;
```

The builder provides sensible defaults:
- `highlighting`: Defaults to terminal detection
- `show_tools`: Defaults to the highlighting setting
- `theme_config`: Defaults to built-in theme
- `verbose_tools`: Defaults to none

### render_context() Helper

When formatting tool output, use the `render_context()` method instead of manually checking the highlighting flag:

```rust
// Good - use the helper
let output = render_grep_invocation(&processor.render_context(), &params);

// Avoid - duplicates the conditional pattern
let ctx = if processor.highlighting_enabled {
    RenderContext::terminal(&processor.code_highlighter)
} else {
    RenderContext::plain(&processor.code_highlighter)
};
```

### ToolResultVariant Pattern

The `ToolResultVariant` enum has 13 variants, one for each tool type that requires specialized result handling:

| Variant | Tool | Purpose |
|---------|------|---------|
| `Bash` | Bash | Command output with truncation |
| `EditBeforeAfter` | Edit | Before/after text replacement |
| `EditDiff` | Edit | Unified diff format |
| `EditNoChanges` | Edit | No-op indicator |
| `WriteNewFile` | Write | New file creation |
| `WriteOverwrite` | Write | File overwrite with diff |
| `WriteNoChanges` | Write | No-op indicator |
| `Read` | Read | File content with metadata |
| `Grep` | Grep | Search results with match count |
| `Glob` | Glob | File list with truncation |
| `TodoWrite` | TodoWrite | Task list confirmation |
| `NotebookEdit` | NotebookEdit | Jupyter cell diff |
| `Default` | Other | Generic content display |

### Adding a New Tool

When adding support for a new tool:

1. Add a variant to `ToolResultVariant` in `output_block.rs`
2. Add a builder function in `block_builders.rs`
3. Add a handler function in `result_handlers.rs`
4. Add a case to the match in `event_handler.rs`
5. Add formatter functions in `tool_display/` and `tool_results/`

### Result Handlers

Each tool type has a dedicated handler function in `result_handlers.rs`:

```rust
pub fn handle_grep_result(
    processor: &StreamProcessor,
    result: &ToolResult,
    invocation: &ToolInvocation,
) -> ToolResultOutput {
    let formatted = tool_results::format_tool_result_with_context(...);
    let block = build_grep_result_block(...);
    ToolResultOutput { formatted, block }
}
```

Handlers return `ToolResultOutput` containing both the formatted display string and the output block for replay serialization.

## Parameter Extraction

For tools with many parameters (like Grep), use dedicated parameter structs:

```rust
// In types.rs
pub struct GrepParams {
    pub pattern: String,
    pub path: Option<String>,
    pub output_mode: Option<String>,
    pub glob: Option<String>,
    pub file_type: Option<String>,
    pub case_insensitive: bool,
}

impl GrepParams {
    pub fn from_invocation_input(input: &serde_json::Value) -> Self {
        // Extract all fields in one place
    }
}
```

This consolidates extraction logic that would otherwise be duplicated across display and block builder functions.

## Output Blocks for Replay

The processor accumulates `OutputBlock` instances during execution for replay serialization:

```rust
pub enum OutputBlock {
    Text(TextBlock),           // Prose/code/diff chunks
    ToolInvocation(ToolInvocationBlock),
    ToolResult(ToolResultBlock),
    Separator,                 // Visual separator between responses
}
```

Each block captures **data**, not rendered strings, allowing re-rendering with different highlighting settings during replay.
