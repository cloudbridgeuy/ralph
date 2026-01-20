# Rendering Code Architecture Audit

This document catalogs the rendering functions in both the stream processor (live execution) and replay renderer paths, identifying duplication and opportunities for consolidation.

## Architecture Overview

The codebase has **two distinct rendering paths**:

1. **Live Execution Path**: `stream_processor/` - Renders output in real-time as Claude executes
2. **Replay Path**: `replay_renderer.rs` - Renders stored `OutputBlock` data for session replay

```
LIVE EXECUTION PATH:
━━━━━━━━━━━━━━━━━━━━
JSON Stream Input
    ↓
event_handler.rs: handle_event()
    ↓
[For each tool invocation]
    ├─→ tool_display::format_tool_invocation()
    ├─→ build_tool_invocation_block() → OutputBlock
    ↓
[For each tool result]
    ├─→ tool_results::format_tool_result_with_context()
    ├─→ build_*_result_block() → OutputBlock
    ↓
Output to stdout (string)
Accumulate OutputBlocks for replay storage


REPLAY PATH:
━━━━━━━━━━━
Stored OutputBlocks (from TOML)
    ↓
ReplayRenderer::render_tool_invocation/render_tool_result()
    ├─→ Private render_*_invocation() methods
    ├─→ Private render_*_result() methods
    ↓
Output to stdout (string)
```

---

## Tool Invocation Rendering Functions

### Live Execution (Stream Processor)

**Location**: `crates/ralph/src/stream_processor/tool_display/`

| Tool | Function | File |
|------|----------|------|
| **Bash** | `format_bash_tool_invocation()` | `bash.rs` |
| **Grep** | `format_grep_tool_invocation_verbose()` | `grep.rs` |
| **Read** | `format_read_tool_invocation_verbose()` | `read.rs` |
| **Glob** | `format_glob_tool_invocation_verbose()` | `glob.rs` |
| **TodoWrite** | `format_todowrite_tool_invocation_verbose()` | `todowrite.rs` |
| **Default/All** | `format_tool_invocation()` | `mod.rs` |

**Key Details**:
- Router function `format_tool_invocation()` dispatches to tool-specific handlers
- Bash always receives special formatting (regex highlighting)
- Grep, Read, Glob, TodoWrite only activate in verbose mode
- Default fallback for unknown tools

### Replay Rendering (ReplayRenderer)

**Location**: `crates/ralph/src/replay_renderer.rs`

| Tool | Method |
|------|--------|
| **Bash** | `render_bash_invocation()` |
| **Grep** | `render_grep_invocation()` |
| **Read** | `render_read_invocation()` |
| **Glob** | `render_glob_invocation()` |
| **TodoWrite** | `render_todowrite_invocation()` |
| **Default** | `render_default_invocation()` |
| **Dispatcher** | `render_tool_invocation()` |
| **Text Blocks** | `render_text()` |

---

## Tool Result Rendering Functions

### Live Execution (Stream Processor)

**Location**: `crates/ralph/src/stream_processor/tool_results/`

| Tool | Function | File |
|------|----------|------|
| **Bash** | `format_bash_tool_result()` | `bash.rs` |
| **Edit** | `format_edit_result_with_snapshot()` | `edit.rs` |
| **Edit** | `format_edit_diff_result()` | `edit.rs` |
| **Write** | `format_write_result_with_snapshot()` | `write.rs` |
| **Read** | `format_read_tool_result_verbose()` | `read.rs` |
| **Grep** | `format_grep_tool_result_verbose()` | `grep.rs` |
| **Glob** | `format_glob_tool_result_verbose()` | `glob.rs` |
| **TodoWrite** | `format_todowrite_tool_result_verbose()` | `todowrite.rs` |
| **NotebookEdit** | `format_notebook_result_with_snapshot()` | `notebookedit.rs` |
| **Default/Router** | `format_tool_result_with_context()` | `mod.rs` |

### Replay Rendering (ReplayRenderer)

| Tool | Method |
|------|--------|
| **Bash** | `render_bash_result()` |
| **Edit** | `render_edit_before_after()` |
| **Edit** | `render_edit_diff()` |
| **Edit** | `render_no_changes_message()` |
| **Write** | `render_write_new_file()` |
| **Write** | `render_write_overwrite()` |
| **Write** | `render_write_no_changes()` |
| **Read** | `render_read_result()` |
| **Grep** | `render_grep_result()` |
| **Glob** | `render_glob_result()` |
| **TodoWrite** | `render_todowrite_result()` |
| **NotebookEdit** | `render_notebook_edit()` |
| **Default** | `render_default_result()` |
| **Dispatcher** | `render_tool_result()` |

---

## Duplication Analysis

### Critical Duplications (Must Address)

#### 1. Line Number Normalization (cat -n format)
- **Stream Processor**: `tool_results/read.rs` - `extract_line_number()`, `normalize_cat_n_format()`
- **Replay Renderer**: `replay_renderer.rs` - identical functions
- **Status**: FULLY DUPLICATED (~45 lines of function code)

#### 2. File Directory Grouping
- **Stream Processor**: `tool_results/glob.rs` - `group_files_by_directory()`
- **Replay Renderer**: `replay_renderer.rs` - identical function
- **Status**: FULLY DUPLICATED (~20 lines)

#### 3. Grep Match Highlighting
- **Stream Processor**: `tool_results/grep.rs` - `highlight_grep_match()`
- **Replay Renderer**: `replay_renderer.rs` - identical function
- **Status**: FULLY DUPLICATED (~30 lines)
- **Note**: Stream processor version has an unused `pattern` parameter; consolidation should use the simpler replay renderer signature

#### 4. Language Detection from Path
- **Stream Processor**: `stream_processor/utils.rs` - `extract_language_from_path()`
- **Replay Renderer**: `replay_renderer.rs` - `extract_language_from_path()`
- **Status**: PARTIALLY DUPLICATED (replay version is a 25-line subset of the 70-line stream processor version)

### Medium Duplications (Should Consider)

> Note: Overlap percentages estimate logical similarity (same algorithm/structure with minor differences in I/O handling).

#### 5. Read Result Formatting
- **Stream Processor**: `tool_results/read.rs` - `format_read_tool_result_verbose()`
- **Replay Renderer**: `replay_renderer.rs` - `render_read_result()`
- **Status**: ~200 lines with 80% overlap

#### 6. Grep Result Formatting
- **Stream Processor**: `tool_results/grep.rs` - `format_grep_tool_result_verbose()`
- **Replay Renderer**: `replay_renderer.rs` - `render_grep_result()`
- **Status**: ~130 lines with 75% overlap

#### 7. Glob Result Formatting
- **Stream Processor**: `tool_results/glob.rs` - `format_glob_tool_result_verbose()`
- **Replay Renderer**: `replay_renderer.rs` - `render_glob_result()`
- **Status**: ~100 lines with 80% overlap

#### 8. Content Block Display (before/after)
- **Stream Processor**: `tool_results/edit.rs`, `tool_results/write.rs` - block formatters
- **Replay Renderer**: `replay_renderer.rs` - `render_content_block()`
- **Status**: ~100 lines with 60% overlap

---

## Shared Utilities

### Current Utilities in Stream Processor

**Location**: `crates/ralph/src/stream_processor/utils.rs`

| Function | Used By | Notes |
|----------|---------|-------|
| `extract_key_argument()` | Both paths | Extracts key args from tool invocations |
| `truncate_string()` | Both paths | Truncates long strings |
| `truncate_multiline()` | Stream processor | Truncates multiline output |
| `extract_language_from_path()` | Stream processor | More complete implementation |

**Problem**: Replay renderer cannot access these utilities because they are nested inside `stream_processor/`.

---

## Checklist: Functions to Consolidate

### Phase 1: Extract Shared Utilities

- [ ] Move `extract_line_number()` to shared module
- [ ] Move `normalize_cat_n_format()` to shared module
- [ ] Move `group_files_by_directory()` to shared module
- [ ] Move `highlight_grep_match()` to shared module
- [ ] Consolidate `extract_language_from_path()` implementations

### Phase 2: Create Shared Rendering Module

- [ ] Create `crates/ralph/src/render.rs` (or `rendering/mod.rs`)
- [ ] Define rendering functions that accept `impl std::io::Write`
- [ ] Implement `OutputBlock` rendering for all variants
- [ ] Add unit tests using `Vec<u8>` as the writer

### Phase 3: Migrate Live Execution

- [ ] Update stream processor to import shared rendering module
- [ ] Replace direct stdout rendering with calls to shared module
- [ ] Verify live execution output is unchanged
- [ ] Run `cargo test` to confirm all tests pass

### Phase 4: Migrate Replay

- [ ] Update replay to import shared rendering module
- [ ] Replace replay_renderer formatting logic with calls to shared module
- [ ] Verify replay output matches live execution
- [ ] Run `cargo test` to confirm all tests pass

### Phase 5: Remove Duplicates

- [ ] Delete duplicated rendering functions from replay_renderer
- [ ] Delete duplicated utility functions
- [ ] Run `cargo xtask lint` to confirm no dead code warnings

---

## Recommended Module Structure

```
crates/ralph/src/
├── render/                    [NEW - shared rendering]
│   ├── mod.rs                 [public exports]
│   ├── utils.rs               [shared utilities: normalize_cat_n, group_files, etc.]
│   ├── tool_invocations.rs    [tool invocation rendering]
│   ├── tool_results.rs        [tool result rendering]
│   └── text_blocks.rs         [prose/code/diff rendering]
├── stream_processor/          [live execution - orchestration only]
│   ├── tool_display/          [keep: tool-specific display logic]
│   ├── tool_results/          [refactor: delegate to render/]
│   └── utils.rs               [move to render/utils.rs]
└── replay_renderer.rs         [refactor: delegate to render/]
```

---

## Files Involved

### Stream Processor (Live Execution)
- `crates/ralph/src/stream_processor/mod.rs`
- `crates/ralph/src/stream_processor/event_handler.rs`
- `crates/ralph/src/stream_processor/output_block.rs`
- `crates/ralph/src/stream_processor/tool_display/*.rs`
- `crates/ralph/src/stream_processor/tool_results/*.rs`
- `crates/ralph/src/stream_processor/utils.rs`

### Replay Rendering
- `crates/ralph/src/replay_renderer.rs`

### Shared Utilities (used by both)
- `crates/ralph/src/diff_highlight.rs`
- `crates/ralph/src/highlight.rs`
