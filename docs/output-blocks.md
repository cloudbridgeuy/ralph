# Output Block Integration and Data Flow

This document explains how output blocks flow through ralph during execution and replay.

## Overview

Output blocks are the serialization format for terminal output. During live execution, ralph renders content to stdout while simultaneously accumulating structured `OutputBlock` variants. These blocks are serialized to session TOML files and can be replayed to recreate the original output.

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           LIVE EXECUTION                                     │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  Claude subprocess (--output-format stream-json)                            │
│              │                                                               │
│              ▼                                                               │
│  StreamProcessor::process_line()                                            │
│              │                                                               │
│              ▼                                                               │
│  parse_stream_line() → StreamEvent                                          │
│              │                                                               │
│              ▼                                                               │
│  handle_event()                                                              │
│      ├─────────────────────────────┬─────────────────────────────┐          │
│      ▼                             ▼                             ▼          │
│  Render to stdout           Build OutputBlock            Push to Vec        │
│  (syntax highlighting)      (capture data)          (accumulate blocks)     │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           SERIALIZATION                                      │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  StreamProcessor::finish() → StreamProcessorResult { output_blocks }        │
│              │                                                               │
│              ▼                                                               │
│  IterationLog { output_blocks: Vec<OutputBlock>, ... }                      │
│              │                                                               │
│              ▼                                                               │
│  write_iteration_log() → .sessions/<slug>/iteration-N.toml                  │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                             REPLAY                                           │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  fs::read_to_string(iteration-N.toml)                                       │
│              │                                                               │
│              ▼                                                               │
│  toml::from_str() → IterationLog { output_blocks }                          │
│              │                                                               │
│              ▼                                                               │
│  ReplayRenderer::render(block) for each block                               │
│              │                                                               │
│              ▼                                                               │
│  print!() → stdout (recreates original output)                              │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
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
| `iteration/log.rs` | IterationLog struct with output_blocks field |
| `replay.rs` | Loads and dispatches blocks for replay |
| `replay_renderer.rs` | Renders OutputBlocks back to formatted strings |

## Data Flow Details

### Phase 1: Live Execution

During execution, `StreamProcessor` handles Claude's JSON output:

1. **Event Parsing**: Each line from Claude is parsed into a `StreamEvent`
2. **Parallel Processing**: For each event:
   - Render formatted content to stdout (with syntax highlighting)
   - Build corresponding `OutputBlock` variant
   - Push block to `output_blocks` vector

```rust
// In event_handler.rs - both actions happen together
if self.show_tool_invocations {
    let formatted = format_tool_invocation(...);
    output_parts.push(formatted);              // Render
    let block = build_tool_invocation_block(...);
    self.output_blocks.push(block);            // Accumulate
}
```

### Phase 2: Serialization

After execution completes (or on interrupt):

1. **Finish**: `StreamProcessor::finish()` returns `StreamProcessorResult` containing `output_blocks`
2. **Log Creation**: `IterationLog` is created with the output_blocks
3. **TOML Write**: `write_iteration_log()` serializes to `iteration-N.toml`

The TOML format uses tagged enums with `#[serde(flatten)]` to merge variant fields at the same level as common fields:

```toml
[[output_blocks]]
type = "text"
[output_blocks.chunk]
chunk_type = { Prose = {} }
content = "Hello, world!"

[[output_blocks]]
type = "tool_invocation"
tool_name = "Bash"
invocation_type = "bash"
command = "ls -la"
description = "List files"

[[output_blocks]]
type = "tool_result"
tool_name = "Edit"
is_error = false
result_type = "edit_before_after"
file_path = "test.rs"
old_content = "fn old() {}"
new_content = "fn new() {}"
```

### Phase 3: Replay

When replaying a session:

1. **Load**: Read iteration TOML file and deserialize `IterationLog`
2. **Check Format**: Use `output_blocks` if present, fall back to `chunks` for legacy sessions
3. **Render**: `ReplayRenderer` converts each block back to formatted output
4. **Display**: Print to stdout, recreating the original terminal experience

```rust
// In replay.rs
if !log.output_blocks.is_empty() {
    replay_output_blocks(&log, highlighter, is_terminal);
} else {
    replay_chunks(&log, highlighter, is_terminal);  // Legacy fallback
}
```

Legacy sessions stored only text content in `chunks` (prose, code, diff). The newer `output_blocks` format captures full tool invocation and result data for complete replay fidelity.

## Design Decisions

### Why Store Data, Not Rendered Strings?

Output blocks capture **structured data** rather than pre-rendered strings:

- **Re-rendering**: Blocks can be rendered with different themes or terminal settings
- **Testability**: Block data can be asserted without parsing ANSI codes
- **Flexibility**: Future replay features can present data differently

### Why Tool-Specific Variants?

Each tool has unique display requirements:

- **Edit/Write**: Need before/after content for diff display
- **Grep**: Has pattern, path, and multiple options to display
- **Bash**: Has command and optional description
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

This ensures replay has complete data without file system access.

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

## Interrupt Handling

On SIGTERM or SIGINT during execution:

1. Signal handler sets `INTERRUPTED` flag
2. Subprocess drains remaining output
3. `StreamProcessor::finish()` captures accumulated blocks
4. Partial `IterationLog` is written with blocks up to interruption
5. Session is marked as "interrupted"

This ensures no output is lost even on early termination.

## Testing

Output block serialization is tested with round-trip assertions:

```rust
// In test_helpers.rs
pub fn assert_toml_roundtrip<T: Serialize + DeserializeOwned + PartialEq>(value: &T) {
    let serialized = toml::to_string(value).unwrap();
    let deserialized: T = toml::from_str(&serialized).unwrap();
    assert_eq!(*value, deserialized);
}
```

Each variant has tests verifying TOML serialization/deserialization.
