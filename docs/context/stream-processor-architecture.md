# Stream Processor Architecture

The `stream_processor` module handles real-time parsing and rendering of Claude CLI's `--output-format stream-json` output. This document covers its internal architecture and patterns.

## Module Structure

Collecting lives in `ralph_core` (no terminal crates); rendering lives in `ralph`.

```
ralph_core::stream::blocks (crates/core/src/stream/blocks/)
├── collector.rs           # StreamCollector: stream-json lines to OutputBlocks
├── files.rs               # Files trait: read_text, exists, image
├── output_block.rs        # OutputBlock and its variants
├── result_handlers.rs     # build_result_block: tool result to output block
├── block_builders.rs      # Invocation blocks and the Write result variant
├── snapshot_capture.rs    # Edit, Write and NotebookEdit snapshots
├── diff.rs, utils.rs      # Pure helpers
└── types.rs               # StreamProcessorResult, VerboseToolsConfig, snapshots

ralph::stream_processor (crates/ralph/src/stream_processor/)
├── block_renderer.rs      # BlockRenderer: blocks to screen text, owns theme and separators
├── render_block.rs        # render_block: one output block to screen text
├── tool_results/          # Edit, Write and NotebookEdit formatters, tool errors
└── processor.rs           # StreamProcessor: collector + renderer (tests only)
```

## Key Patterns

### Collect, Then Render

`StreamCollector::process_line` parses a line and returns the new `OutputBlock`s;
`finish` flushes buffered prose and returns the last blocks plus a
`StreamProcessorResult`. It does no rendering. `BlockRenderer::render` turns
blocks into screen text through `render_block`, so the screen text is a pure
function of the blocks. `tests/block_parity.rs` checks this for every tool path
and text chunk. `StreamProcessor` (collector + renderer) exists for tests only.

### Files Trait

All file access during collection goes through `Files`. `ralph` passes
`DiskFiles`; `ralph_core` never touches the filesystem. Images referenced in
markdown are read at collect time: `TextBlock.images` holds an `ImageFile`
(path, resolved path, bytes). It is `serde(skip)`, so stored blocks carry no
image bytes. Ceiling: `DiskFiles` reads at most 10 MiB per image; a larger or
unreadable file gives `bytes: None`. Rendering never reads a file.

### render_context() Helper

When formatting tool output, use `BlockRenderer::render_context()` instead of manually checking the highlighting flag.

### ToolResultVariant Pattern

The `ToolResultVariant` enum has 14 variants. Each holds the data that its screen text needs:

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
| `NotebookEdit` | NotebookEdit | Jupyter cell diff, or no changes |
| `ToolError` | Read, Grep, Glob | Verbose tool error |
| `Default` | Other | Generic content display |

### Adding a New Tool

When adding support for a new tool:

1. Add a variant to `ToolResultVariant` in `output_block.rs` (core)
2. Fill it in `build_result_block` in `result_handlers.rs` (collect side: file reads go through `Files`)
3. Render it in `render_result` in `render_block.rs` (render side: no I/O)
4. Add a case to `tests/block_parity.rs`

## Parameter Extraction

For tools with many parameters (like Grep), use dedicated parameter structs:

```rust
// In ralph_core types.rs
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

Code fences render through `render_code_block` (`render/text.rs`). Shell fences (`bash`, `sh`, `zsh`, `shell`) first go through `ralph_core::shell_format::format_fence`, so they show like a Bash tool call; the stored `ParsedChunk` keeps the raw text.

### Flush points

Buffered prose is flushed (collected as blocks) at these points:

1. **New assistant message** — when a different `message_id` arrives, the previous message's prose is flushed before processing the new one
2. **Tool invocation** — prose is flushed before tool invocation display, so text appears in the correct order
3. **Code fence** — the `StreamingChunkBuffer` flushes accumulated prose when a code fence opens
4. **`finish()`** — any remaining prose is flushed and returned as the last blocks

## Turn Events

`Pump` (`crates/ralph_server/src/subprocess/spinner.rs`) collects with a `StreamCollector` and does not render or print. The client sets `SpinnerSubprocessConfig::show_tools` and `terminated`; `DiskFiles` (`crates/ralph_server/src/disk_files.rs`) is the `Files` impl backed by the filesystem. It emits `ralph_core::turn_event::TurnEvent`s to the `TurnSink` in `SpinnerSubprocessConfig::sink`; `ralph_server::chat::turn::Turn` emits to the same sink.

| Event | Sent when | `TerminalSink` does |
| --- | --- | --- |
| `Busy(Wait)` | a shown pump starts; Turn waits for the session lock or a nap | starts a spinner on a fresh clock |
| `Resume(Wait)` | output stops for `DEFAULT_GAP_THRESHOLD_MS` (`Wait::after_gap`: `Tool` with a pending tool, else `Thinking`) | restarts the spinner on the same clock |
| `Idle` | before output, on pause, at finish, on drop | stops the spinner and clears its line |
| `Blocks(Vec<OutputBlock>)` | the collector yields blocks (also at finish) | its `BlockRenderer` renders them; writes the text to stdout through the gate |
| `Stderr(String)` | a shown pump reads a child stderr line | writes it to stderr through the gate |
| `Rule`, `Warning(String)` | Turn draws a rule or warns | writes the themed rule at the chat width, or a warning line on stderr |

A silent pump (pings, the memory curator) emits nothing. The spinner thread still writes stdout directly, past the gate. Pump keeps the `OutputGate` only for the hold-panel pause and settle handshake: it emits `Idle`, then settles. `Blocks` carries data; only the sink renders it.

## Directive-Aware Streaming

Prose chunks are scanned for directive tags (`<ralph-{verb} to="...">...</ralph-{verb}>`) during both line-by-line streaming and flush processing. When directives are found, `split_text_around_directives()` splits the prose into alternating `Prose` and `Directive` sub-chunks.

- `Directive` chunks carry `verb` and `target` metadata alongside the payload content
- The renderer renders them as styled one-liners: `→ target (verb): "payload"`
- Directive blocks receive trailing newlines matching code/diff block behavior
- In `extract_response_text()`, directive chunks are reconstructed back to XML tags

The splitting happens in `collector.rs` at both streaming sites (line processing and flush). The reconstruction happens in `output_block.rs::text_block_to_response_text()`.

## Shared Markdown Skin

`markdown.rs` owns terminal markdown styling. A `Palette` holds every color for one role (assistant or agent); the `RolePalette` trait's `palette` method on `RoleTheme` resolves it purely from the role default plus `[theme.<role>]` overrides and returns warnings. `MarkdownTheme` pairs the `Palette` with its `MadSkin` (`markdown_skin`). `BlockRenderer` builds the assistant `MarkdownTheme` from `ThemeConfig::palette()`; the chat shell builds the agent one for `/inbox`. `render_text_block` and `render_markdown` take a `MarkdownTheme`.

## Table Enhancement

`enhance_tables()` in `render/text.rs` post-processes termimad table output to add:

- **Top/bottom borders** — `┌┬┐` / `└┴┘` derived from the separator line

termimad already constrains tables to terminal width and provides cell spacing via column alignment. `enhance_tables()` only adds top/bottom borders by character-substituting the separator (`├→┌`, `┼→┬`, `┤→┐`). No characters are added or removed — only substituted — so the table stays within terminal width.

This runs after `render_prose()` inside `render_text_block()`. Box-drawing characters are multi-byte UTF-8 (bytes > 0x7F), so they cannot appear inside ANSI escape sequences — making string splitting on `│`/`├` safe even in ANSI-coded output.

## Links

termimad has no link concept. `render/link.rs` rewrites `[label](url)` before `render_prose()` (`mark_links`) to `label ↗ url` wrapped in zero-width marker chars, so wrapping widths stay correct. After rendering, `paint_links` swaps the markers for ANSI: label underlined in the palette `link` color (default bright blue), URL in `url` (default bright cyan). Code spans, escaped `\[`, and images stay literal. The same pass marks memory references (`#` plus digits, e.g. `#303`) and paints them in `memory_ref` (default bright magenta). A reference must not follow a letter, digit, `_`, `#`, `&`, `/` or `\`, and must not run into a letter, digit or `_`; so `# Title`, `abc#12`, `&#123;` and URL fragments stay plain, as do refs in code spans and link labels. Signed numbers are marked too: a whole token of one sign (`+`, `-` or `−`) and ASCII digits, set off by whitespace or the text edges (`+204`, `-85`), is painted with the palette `done` slot for `+` and `blocked` for `-`/`−` (defaults green and red). Three exact whole-token pairs also match, `+204/-85`, `(+204/-85)` and `(4 files, +204/-85)` (`file` or `files` for any count, only inside parentheses): each number is painted, the `/`, parentheses and file count stay plain. `- item`, dates, ranges, `-85)`, `+12px`, `+204/-85,`, `(+204)` and `-85/+204` stay plain, as do table rows and headings (a colour reset there would end the heading colour). Git `--stat` lines use those same colours for the count and `(+)` or `(-)` in `N insertion(s)(+)` and `N deletion(s)(-)` and for the `+` and `-` runs of a `| <count> ++++--` bar, leaving the words, pipe, and bar count plain, while a `---` rule stays plain. Inside inline code spans the same rule applies and the code color is restored after the token. Non-terminal output keeps the raw markdown. When a styled span wraps, `paint_links` opens its color again after the newline.

## Prose Wrapping

`render_prose()` in `render/wrap.rs` renders prose at `wrap_width(terminal_size())`: the terminal width, capped by the chat wrap width (`[chat] wrap_width`, `RALPH_WRAP_WIDTH` or `--wrap-width`, see [configuration](../configuration.md)), and replaces termimad's space-only wrapping with UAX #14 line breaking (`unicode-linebreak`). It renders the markdown twice: once without a width, for the normal lines, and once at the terminal width, for the fitted table rows. Each normal line is broken by the pure `break_lines(text, first, rest)`: greedy fit over the UAX #14 break opportunities, widths from `unicode-width` (CJK and emoji are 2 columns), trailing spaces dropped. A segment wider than the line is cut at the column limit. Break opportunities are computed with the link markers removed, so they add no breaks and take no columns; a marker before a word moves to the next line with the word. Lines are sliced back into termimad compounds, so bold, italic and inline code styles stay on both sides of a break. List items and quotes continue with their follow-up indent. Code lines are not wrapped. Chat sets the cap once at start with `cap_wrap_width` (a process-wide `OnceLock`); `ralph ask` never sets it, so it always wraps at the terminal width. The same cap reaches the prompt area through reedline's `with_wrap_width`: the painter reports the capped width as `screen_width()`, and `PromptLines::soft_wrapped` inserts `\r\n` where a terminal that narrow would wrap, walking the same `advance_grapheme` rule as the layout math so rows, cursor and menus agree.
