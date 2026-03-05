# ralph edit Command

The `ralph edit` subcommand opens a session's conversation history in `$EDITOR` for interactive editing.

## How It Works

1. Resolves the target session (by slug or most recent)
2. Loads iteration logs and projects them as a TOML `[[messages]]` array
3. Opens the TOML in the user's editor (`$VISUAL` → `$EDITOR` → `vi`)
4. On save, parses the edited TOML and diffs against the original
5. Applies changes back to iteration files (rewrite, delete, or create)

If the edited TOML has parse errors, the user is prompted to retry (reopen editor) or abort.

## Module Structure

The edit module follows **Functional Core - Imperative Shell** and is split across four files:

```
crates/ralph/src/edit/
├── mod.rs     — Module glue, re-exports, imperative shell (editor spawning, file I/O, updates)
├── types.rs   — EditError, EditMessage, IterationUpdate, EditSummary
├── core.rs    — Pure functions (message conversion, TOML generation/parsing, diff planning)
└── tests.rs   — Unit tests for all core functions
```

### Functional Core (`core.rs`)

Five pure functions that handle all business logic without side effects:

| Function | Purpose |
|----------|---------|
| `iterations_to_messages` | Convert `IterationLog[]` → flat `EditMessage[]` |
| `messages_to_edit_toml` | Generate TOML string from messages |
| `parse_edit_toml` | Parse TOML back into messages (with validation) |
| `pair_messages_to_iterations` | Re-pair flat messages into (prompt, response) tuples |
| `plan_iteration_updates` | Diff old vs new iterations → `IterationUpdate[]` plan |

### Imperative Shell (`mod.rs`)

Handles all I/O: editor resolution, process spawning, file read/write, retry prompts, and the main `execute_edit` entry point.

## TOML Format

```toml
# Session: my-session
# Editing conversation history. Each [[messages]] block is one turn.

[[messages]]
role = "user"
content = """
Your prompt here
"""

[[messages]]
role = "assistant"
content = """
The response here
"""
```

Users can edit content, delete blocks, or add new blocks. Roles must be `"user"` or `"assistant"`.

## CLI

```
ralph edit [SLUG]
```

- `SLUG` — Session to edit. Defaults to the most recent session for the current project.
