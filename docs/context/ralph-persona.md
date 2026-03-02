# ralph persona Command

This document describes the `ralph persona` command, which enables conversations with persona-configured Claude instances using named agent files.

## Code Locations

| Component | File | Description |
|-----------|------|-------------|
| CLI args | `crates/ralph/src/cli/mod.rs` | PersonaArgs, PersonaAction, PersonaInvokeArgs |
| Core types | `crates/core/src/persona.rs` | PersonaInfo, PersonaSource, frontmatter parsing, dedup |
| Discovery & display | `crates/ralph/src/persona.rs` | Agent file resolution, scanning, formatting |
| Execution | `crates/ralph/src/main.rs` | execute_persona(), execute_persona_list() |
| Shared engine | `crates/ralph/src/invoke.rs` | InvocationConfig, invoke() |

## Purpose

The `ralph persona` command allows interacting with Claude through named agent configurations. Agent files define persona-specific system prompts, tool permissions, and behavioral constraints. Unlike `ralph ask` which uses `--permission-mode`, personas use Claude's `--agent` flag.

## CLI Flags

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `[PERSONA]` | positional | - | Agent name (resolves to `.claude/agents/{name}.md`) |
| `[PROMPT]` | positional | stdin | The prompt to send |
| `--list` | flag | false | List all available personas (conflicts with PERSONA) |
| `-S, --session` | string | auto-generated | Session name |
| `-c, --continue` | flag | false | Continue most recent session for this persona |
| `--theme` | string | config/env | Syntax highlighting theme |
| `--no-background` | flag | false | Disable theme background colors |
| `--timeout` | u64 | 600 | Subprocess timeout in seconds |
| `--verbose-tools` | string | none | Enable verbose output for specific tools |
| `--no-prompt` | flag | false | Suppress prompt display |
| `--history` | flag | false | Display conversation history |
| `--clone-session` | flag | false | Clone from existing session |

## Persona Discovery (`--list`)

`ralph persona --list` scans both discovery locations for `.md` files with valid YAML frontmatter (`name` and `description` fields). Project personas shadow user personas with the same name. Output is a columnar table sorted alphabetically by name:

```
NAME     SOURCE   DESCRIPTION
────     ───────  ───────────
dev      project  Development assistant
review   user     Code review persona
```

Agent files must have YAML frontmatter delimited by `---` with at least `name` and `description` fields. Files without valid frontmatter are silently skipped.

## Agent File Resolution

Agent files are searched in order:

1. **Project-local**: `.claude/agents/{name}.md`
2. **User-level**: `~/.claude/agents/{name}.md`

If neither exists, the command fails with an error showing the expected path.

## Session Scoping

Persona sessions are scoped by persona name. When using `--continue` without `--session`, ralph finds the most recent session for the **same persona** in the current project. This prevents accidentally continuing an `ask` session from a `persona` command or vice versa.

The persona name is stored in both `SessionEntry.persona` and `SessionMetadata.persona` fields.

## Shared Invocation Engine

Both `ralph ask` and `ralph persona` delegate to the shared invocation engine in `invoke.rs`. The key difference is the `persona` field in `InvocationConfig`:

- `ask`: `persona: None` — uses `--permission-mode` flag
- `persona`: `persona: Some(name)` — uses `--agent` flag

The `build_shared_invocation_config()` function in `main.rs` handles the shared logic (clone resolution, continuation, theme, verbose tools, prompt resolution) for both commands through `InvocationConfigParams`.
