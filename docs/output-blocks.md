# Output Blocks

This document explains how output blocks flow through ralph during execution.

## Overview

During live execution, ralph renders content to stdout while it also accumulates structured `OutputBlock` variants. Nothing writes the blocks to disk. `extract_response_text()` joins the text blocks into the final response text that chat turns and naps use.

## Data Flow

```
Claude subprocess (--output-format stream-json)
        │
        ▼
StreamProcessor::process_line() → parse_stream_line() → StreamEvent
        │
        ▼
handle_event()
    ├── Render to stdout (syntax highlighting)
    └── Build OutputBlock → push to output_blocks
        │
        ▼
StreamProcessor::finish() → StreamProcessorResult { output_blocks }
        │
        ▼
extract_response_text(&output_blocks) → Option<String>
```

## OutputBlock Enum

The `OutputBlock` enum captures all renderable content:

```rust
#[non_exhaustive]
pub enum OutputBlock {
    Text(TextBlock),           // Prose, code, or diff text
    ToolInvocation(ToolInvocationBlock),  // Tool header display
    ToolResult(ToolResultBlock),          // Tool output display
    Separator,                 // Visual separator between responses
}
```

### Text Blocks

Text blocks wrap `ParsedChunk` from ralph_core:

```rust
pub struct TextBlock {
    pub chunk: ParsedChunk,  // Prose, Code, or Diff chunk type
}
```

### Tool Invocation Blocks

Each tool type has a specific variant for capturing invocation data:

| Tool | Variant | Captured Data |
|------|---------|---------------|
| Bash | `Bash` | command, description |
| Grep | `Grep` | pattern, path, output_mode, glob, file_type, case_insensitive |
| Read | `Read` | file_path, offset, limit |
| Glob | `Glob` | pattern, path |
| TodoWrite | `TodoWrite` | todos (Vec of TodoItem) |
| Others | `Default` | key_argument, is_path |

The `Grep` variant uses `GrepInvocationBuilder` for fluent construction with sensible defaults.

### Tool Result Blocks

Tool results have variants to capture before/after states:

| Tool | Variants | Purpose |
|------|----------|---------|
| Bash | `Bash` | content, truncated flag |
| Edit | `EditBeforeAfter`, `EditDiff`, `EditNoChanges` | Before/after display, unified diff, or no-op |
| Write | `WriteNewFile`, `WriteOverwrite`, `WriteNoChanges` | New file, overwrite, or no-op (includes is_new_file flag) |
| Read | `Read` | file_path, content, line_count, truncated |
| Grep | `Grep` | match_count, output_mode, content |
| Glob | `Glob` | file_count, content, truncated |
| TodoWrite | `TodoWrite` | message |
| NotebookEdit | `NotebookEdit` | notebook_path, cell_identifier, diff_content, etc. |
| Others | `Default` | content |

## Key Implementation Files

| File | Responsibility |
|------|----------------|
| `stream_processor/output_block.rs` | OutputBlock enum and all variant definitions |
| `stream_processor/block_builders.rs` | Helper functions to build OutputBlocks from tool data |
| `stream_processor/processor.rs` | Accumulates blocks during streaming |
| `stream_processor/event_handler.rs` | Pushes blocks during event processing |

## Live Execution

For each event, `StreamProcessor` renders formatted content to stdout and pushes the matching `OutputBlock`:

```rust
// In event_handler.rs - both actions happen together
if self.show_tool_invocations {
    let formatted = format_tool_invocation(...);
    output_parts.push(formatted);              // Render
    let block = build_tool_invocation_block(...);
    self.output_blocks.push(block);            // Accumulate
}
```

## Design Decisions

### Why Store Data, Not Rendered Strings?

Output blocks capture **structured data** rather than pre-rendered strings:

- **Testability**: Block data can be asserted without parsing ANSI codes
- **Text extraction**: The response text comes from text blocks, not from ANSI output

### Why Tool-Specific Variants?

Each tool has unique display requirements:

- **Edit/Write**: Need before/after content for diff display
- **Grep**: Has pattern, path, and multiple options to display
- **Bash**: Has command and optional description. Single-line commands are pretty-printed when wider than the terminal, or always when they chain statements with `;` (a short command with `;` also splits at `&&`, `||` and `&`), by `ralph_core::shell_format::format_command` into a highlighted `sh` block: one statement per line on `;`, `&&`, `||`, indented loop and `if` bodies, and long pipelines broken before `|`. Long commands keep the command and subcommands on line one, then one flag group per two-space-indented line, with ` \` continuations except on the last line. Quotes are never split; indivisible values may exceed the width. Short commands without `;`, multi-line commands, and unparsable commands render unchanged.
- **TodoWrite**: Has structured todo items with status

Generic storage would lose this structure.

### Snapshot Pattern for Before/After

Edit, Write, and NotebookEdit tools use "snapshots" captured before execution:

```rust
// Before tool executes, capture current state
let snapshot = EditSnapshot {
    file_path: path.clone(),
    original_content: fs::read_to_string(&path).ok(),
    old_string: old_string.clone(),
    new_string: new_string.clone(),
};

// After tool executes, snapshot enables before/after display
// without re-reading the file system
```

### Non-Exhaustive Enums

All block enums use `#[non_exhaustive]`:

```rust
#[non_exhaustive]
pub enum OutputBlock { ... }

#[non_exhaustive]
pub enum ToolInvocationVariant { ... }

#[non_exhaustive]
pub enum ToolResultVariant { ... }
```

This allows adding new variants without breaking downstream code.
