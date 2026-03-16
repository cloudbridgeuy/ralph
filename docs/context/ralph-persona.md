# ralph persona Command

This document describes the `ralph persona` command, which enables conversations with persona-configured Claude instances using named agent files.

## Code Locations

| Component | File | Description |
|-----------|------|-------------|
| CLI args | `crates/ralph/src/cli/mod.rs` | PersonaArgs, PersonaAction, PersonaInvokeArgs |
| Core types | `crates/core/src/persona.rs` | PersonaInfo, PersonaSource (Project, User, Strategy), frontmatter parsing, dedup |
| Discovery & display | `crates/ralph/src/persona.rs` | Agent file resolution, scanning, formatting, strategy-aware discovery |
| Strategy loading | `crates/ralph/src/strategy/mod.rs` | `load_team_strategy()` — loads `.claude/strategy.toml` if present |
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

`ralph persona --list` discovers available personas and displays them in a columnar table sorted alphabetically by name.

When `.claude/strategy.toml` exists, discovery reads agent files from the strategy's `[agents]` table exclusively — no directory scanning occurs. Personas are tagged with `"strategy"` source. The strategy key is the canonical name, even if the agent file's frontmatter `name` differs.

When no strategy file exists, discovery scans both project-local and user-level directories for `.md` files with valid YAML frontmatter. Project personas shadow user personas with the same name.

```
NAME              SOURCE    DESCRIPTION
────              ────────  ───────────
architect         strategy  System architect
developer         strategy  Code writer
product-manager   strategy  Product manager
```

Agent files must have YAML frontmatter delimited by `---` with at least `name` and `description` fields. Files without valid frontmatter are silently skipped.

## Agent File Resolution

Agent files are resolved through a multi-step lookup:

1. **Strategy file**: If `.claude/strategy.toml` exists, resolve `<name>` against the `[agents]` table and load from the specified path
2. **Project-local**: `.claude/agents/{name}.md`
3. **User-level**: `~/.claude/agents/{name}.md`

If a strategy file exists but the agent name is not in its `[agents]` table, the command fails with an error listing the available agents from the strategy.

If no strategy file exists, the existing project-local and user-level search is used (backward compatible).

If no agent file is found at any location, the command fails with an error showing the expected path.

### Error Messages

- **Agent file missing on disk**: "Agent file not found for persona '{name}'. Expected at: {path}"
- **Name not in strategy**: "Persona '{name}' is not defined in the project strategy. Available agents: {list}. Add it to .claude/strategy.toml [agents] table."

## Session Scoping

Persona sessions are scoped by persona name. When using `--continue` without `--session`, ralph finds the most recent session for the **same persona** in the current project. This prevents accidentally continuing an `ask` session from a `persona` command or vice versa.

The persona name is stored in both `SessionEntry.persona` and `SessionMetadata.persona` fields.

## Shared Invocation Engine

Both `ralph ask` and `ralph persona` delegate to the shared invocation engine in `invoke.rs`. The key difference is the `persona` field in `InvocationConfig`:

- `ask`: `persona: None` — uses `--permission-mode` flag
- `persona`: `persona: Some(name)` — uses `--agent` flag

The `build_shared_invocation_config()` function in `main.rs` handles the shared logic (clone resolution, continuation, theme, verbose tools, prompt resolution) for both commands through `InvocationConfigParams`.

## Orchestration

After a persona invocation completes, ralph scans the output for orchestration directives (`<ralph-ask>`, `<ralph-handover>`). If directives are found, ralph automatically invokes the target personas, manages the response flow, and may continue the originator's session with aggregated results.

This happens transparently — the user sees routing status lines and a budget summary when orchestration occurs. The persona does not need to be configured for orchestration; ralph injects directive syntax automatically via `--append-system-prompt`, so all personas can emit directives without explicit configuration.

Budget, directive format, and orchestration modes are documented in [Orchestration](orchestration.md).
