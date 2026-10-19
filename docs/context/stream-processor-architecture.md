# Stream Processor Architecture

The `stream_processor` module handles real-time parsing and rendering of Claude CLI's `--output-format stream-json` output. This document covers its internal architecture and patterns.

## Module Structure

```
stream_processor/
├── mod.rs                 # Module exports
├── processor.rs           # StreamProcessor struct and builder
├── event_handler.rs       # Event processing logic
├── result_handlers.rs     # Tool result handler functions
├── output_block.rs        # Output block types
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

Handlers return `ToolResultOutput` containing both the formatted display string and the output block.

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

## Output Blocks

The processor accumulates `OutputBlock` instances during execution:

```rust
pub enum OutputBlock {
    Text(TextBlock),           // Prose/code/diff chunks
    ToolInvocation(ToolInvocationBlock),
    ToolResult(ToolResultBlock),
    Separator,                 // Visual separator between responses
}
```

Each block captures **data**, not rendered strings. `extract_response_text()` joins the prose blocks into the final response text for chat turns and naps.

## Block-Buffered Prose Rendering

Prose text uses **block buffering** (`prose_threshold = usize::MAX`) so that `render_prose()` receives complete multi-line content. This enables proper markdown block rendering (tables, lists, paragraphs) rather than line-by-line rendering.

### Flush points

Buffered prose is flushed (rendered and emitted) at these points:

1. **New assistant message** — when a different `message_id` arrives, the previous message's prose is flushed before processing the new one
2. **Tool invocation** — prose is flushed before tool invocation display, so text appears in the correct order
3. **Code fence** — the `StreamingChunkBuffer` flushes accumulated prose when a code fence opens
4. **`finish()`** — any remaining prose is flushed via `final_output` in `StreamProcessorResult`

### final_output

`StreamProcessorResult.final_output` contains the last rendered prose from `finish()`. Callers (spinner, timeout) must `print!` this value before returning the result.

## Directive-Aware Streaming

Prose chunks are scanned for directive tags (`<ralph-{verb} to="...">...</ralph-{verb}>`) during both line-by-line streaming and flush processing. When directives are found, `split_text_around_directives()` splits the prose into alternating `Prose` and `Directive` sub-chunks.

- `Directive` chunks carry `verb` and `target` metadata alongside the payload content
- The stream processor renders them as styled one-liners: `→ target (verb): "payload"`
- Directive blocks receive trailing newlines matching code/diff block behavior
- In `extract_response_text()`, directive chunks are reconstructed back to XML tags

The splitting happens in `event_handler.rs` at both streaming sites (line processing and flush). The reconstruction happens in `output_block.rs::text_block_to_response_text()`.

## Shared Markdown Skin

`markdown.rs` owns terminal markdown styling. A `Palette` holds every color for one role (assistant or agent); `RoleTheme::palette` resolves it purely from the role default plus `[theme.<role>]` overrides and returns warnings. `MarkdownTheme` pairs the `Palette` with its `MadSkin` (`markdown_skin`). `StreamProcessor` builds the assistant `MarkdownTheme` from `ThemeConfig::palette()`; the chat shell builds the agent one for `/inbox`. `render_text_block` and `render_markdown` take a `MarkdownTheme`.

## Table Enhancement

`enhance_tables()` in `render/text.rs` post-processes termimad table output to add:

- **Top/bottom borders** — `┌┬┐` / `└┴┘` derived from the separator line

termimad already constrains tables to terminal width and provides cell spacing via column alignment. `enhance_tables()` only adds top/bottom borders by character-substituting the separator (`├→┌`, `┼→┬`, `┤→┐`). No characters are added or removed — only substituted — so the table stays within terminal width.

This runs after `render_prose()` inside `render_text_block()`. Box-drawing characters are multi-byte UTF-8 (bytes > 0x7F), so they cannot appear inside ANSI escape sequences — making string splitting on `│`/`├` safe even in ANSI-coded output.

## Links

termimad has no link concept. `render/link.rs` rewrites `[label](url)` before `render_prose()` (`mark_links`) to `label ↗ url` wrapped in zero-width marker chars, so wrapping widths stay correct. After rendering, `paint_links` swaps the markers for ANSI: label underlined in the palette `link` color (default bright blue), URL in `url` (default bright cyan). Code spans, escaped `\[`, and images stay literal. The same pass marks memory references (`#` plus digits, e.g. `#303`) and paints them in `memory_ref` (default bright magenta). A reference must not follow a letter, digit, `_`, `#`, `&`, `/` or `\`, and must not run into a letter, digit or `_`; so `# Title`, `abc#12`, `&#123;` and URL fragments stay plain, as do refs in code spans and link labels. Signed numbers are marked too: a whole token of one sign (`+`, `-` or `−`) and ASCII digits, set off by whitespace or the text edges (`+204`, `-85`), is painted with the palette `done` slot for `+` and `blocked` for `-`/`−` (defaults green and red). Three exact whole-token pairs also match, `+204/-85`, `(+204/-85)` and `(4 files, +204/-85)` (`file` or `files` for any count, only inside parentheses): each number is painted, the `/`, parentheses and file count stay plain. `- item`, dates, ranges, `-85)`, `+12px`, `+204/-85,`, `(+204)` and `-85/+204` stay plain, as do table rows and headings (a colour reset there would end the heading colour). Git `--stat` lines use those same colours for the count and `(+)` or `(-)` in `N insertion(s)(+)` and `N deletion(s)(-)` and for the `+` and `-` runs of a `| <count> ++++--` bar, leaving the words, pipe, and bar count plain, while a `---` rule stays plain. Inside inline code spans the same rule applies and the code color is restored after the token. Non-terminal output keeps the raw markdown. When a styled span wraps, `paint_links` opens its color again after the newline.

## Prose Wrapping

`render_prose()` in `render/wrap.rs` renders prose at `wrap_width(terminal_size())`: the terminal width, capped by the chat wrap width (`[chat] wrap_width`, `RALPH_WRAP_WIDTH` or `--wrap-width`, see [configuration](../configuration.md)), and replaces termimad's space-only wrapping with UAX #14 line breaking (`unicode-linebreak`). It renders the markdown twice: once without a width, for the normal lines, and once at the terminal width, for the fitted table rows. Each normal line is broken by the pure `break_lines(text, first, rest)`: greedy fit over the UAX #14 break opportunities, widths from `unicode-width` (CJK and emoji are 2 columns), trailing spaces dropped. A segment wider than the line is cut at the column limit. Break opportunities are computed with the link markers removed, so they add no breaks and take no columns; a marker before a word moves to the next line with the word. Lines are sliced back into termimad compounds, so bold, italic and inline code styles stay on both sides of a break. List items and quotes continue with their follow-up indent. Code lines are not wrapped. Chat sets the cap once at start with `cap_wrap_width` (a process-wide `OnceLock`); `ralph ask` never sets it, so it always wraps at the terminal width. The same cap reaches the prompt area through reedline's `with_wrap_width`: the painter reports the capped width as `screen_width()`, and `PromptLines::soft_wrapped` inserts `\r\n` where a terminal that narrow would wrap, walking the same `advance_grapheme` rule as the layout math so rows, cursor and menus agree.
