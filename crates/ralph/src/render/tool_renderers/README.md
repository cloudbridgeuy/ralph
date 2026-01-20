# Tool Renderers Module

Pure rendering functions used by both stream processor (live execution) and replay renderer.

## Design Principles

1. **Pure Functions**: All functions are stateless and free of side effects
2. **Data-Centric**: Accept structured data, not processor/renderer references
3. **Dual Mode**: Support both terminal (ANSI) and plain text output
4. **Single Source of Truth**: One implementation for both rendering paths

## Structure

```
tool_renderers/
├── mod.rs              # Module declarations + public API re-exports
├── context.rs          # ANSI color constants + RenderContext struct
├── types.rs            # Shared parameter structs
├── invocations/        # Tool invocation renderers
│   ├── bash.rs         # render_bash_invocation
│   ├── glob.rs         # render_glob_invocation
│   ├── grep.rs         # render_grep_invocation
│   ├── read.rs         # render_read_invocation
│   ├── todowrite.rs    # render_todowrite_invocation
│   └── default.rs      # render_default_invocation (fallback)
├── results/            # Tool result renderers
│   ├── bash.rs         # render_bash_result
│   ├── edit.rs         # render_edit_* (before/after, diff, content_block)
│   ├── write.rs        # render_write_* (new file, no changes)
│   ├── read.rs         # render_read_result
│   ├── grep.rs         # render_grep_result
│   ├── glob.rs         # render_glob_result
│   ├── todowrite.rs    # render_todowrite_result
│   ├── notebook.rs     # render_notebook_edit
│   └── default.rs      # render_default_result (fallback)
└── tests.rs            # All unit tests
```

## Usage

```rust
use ralph::render::{render_bash_invocation, RenderContext};

let ctx = RenderContext::terminal(&highlighter);
let output = render_bash_invocation(&ctx, "ls -la");
```

All render functions:
- Take a `RenderContext` reference as the first parameter
- Return a `String` with the formatted output
- Support both terminal (ANSI codes) and plain text modes via `ctx.terminal`

## Pattern Reference

This module follows the "Splitting Large Modules" pattern documented in
`docs/context/rust-cli-project-structure.md`. The structure mirrors
`stream_processor/tool_display/` and `stream_processor/tool_results/`.
