# ralph ask Command

This document provides comprehensive documentation for the `ralph ask` command, which provides single-shot prompts to Claude with session persistence.

## Code Locations

| Component | File | Description |
|-----------|------|-------------|
| CLI args | `crates/ralph/src/cli/mod.rs` | AskArgs struct definition |
| Permission mode | `crates/ralph/src/ask.rs` | resolve_permission_mode() |
| Shared engine | `crates/ralph/src/invoke.rs` | InvocationConfig, invoke(), command building |
| Prompt source | `crates/ralph/src/prompt_source.rs` | classify_prompt_source(), read_from_source() |
| Execution | `crates/ralph/src/main.rs` | execute_ask(), build_invocation_config() |
| Session | `crates/ralph/src/session.rs` | Session management and persistence |
| Iteration logs | `crates/ralph/src/iteration/writer.rs` | Conversation history extraction |
| Config | `crates/ralph/src/config.rs` | AppConfig with AskSection |

## Purpose

The `ralph ask` command provides a streamlined interface for quick LLM interactions. Unlike `ralph run` which processes PRD stories iteratively, `ask` sends a single prompt and displays the response. Key features:

- **Session persistence**: Each invocation creates or continues a session for replay
- **Conversation history**: Continue previous sessions with context preserved
- **Session cloning**: Branch off from existing sessions into new ones
- **Permission control**: Configurable tool execution modes

## CLI Flags

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `[PROMPT]` | positional | stdin | The prompt to send (inline, file path, or `-` for stdin) |
| `-S, --session` | string | auto-generated | Session name (new sessions) or session to continue (with `--continue`) |
| `-c, --continue` | flag | false | Continue an existing session instead of creating new |
| `--clone-session` | flag | false | Clone history from existing session into a new one |
| `--history` | flag | false | Display conversation history for the session |
| `--theme` | string | `Monokai Extended` | Syntax highlighting theme |
| `--no-background` | flag | false | Disable theme background colors |
| `--timeout` | u64 | 600 | Subprocess timeout in seconds |
| `--verbose-tools` | string | none | Verbose output for tools (omit value for all, or specify names) |
| `--no-prompt` | flag | false | Suppress prompt display before execution |
| `--permission-mode` | string | `bypassPermissions` | Permission mode for tool execution |

### Verbose Tools Examples

```bash
ralph ask --verbose-tools 'prompt'       # Enable for all tools
ralph ask --verbose-tools=grep,bash 'prompt'  # Enable for specific tools
```

## Prompt Resolution

The prompt can be provided from multiple sources:

| Source | Example | Description |
|--------|---------|-------------|
| Positional argument | `ralph ask 'What is 2+2?'` | Inline prompt text |
| File path | `ralph ask /path/to/prompt.txt` | Read prompt from file (if path exists) |
| Stdin explicit | `ralph ask -` | Read from stdin (explicit) |
| Stdin piped | `echo 'prompt' \| ralph ask` | Read from stdin when piped (automatic) |

### Resolution Priority

1. If argument is `-`, read from stdin
2. If argument is a valid file path, read from file
3. If argument is any other string, use as inline prompt
4. If no argument and stdin is piped (non-terminal), read from stdin
5. If no argument and stdin is terminal, error (no prompt provided)

## Session Management

### Session Storage

Sessions use the same storage as `ralph run`. See [ralph-run.md](ralph-run.md) for storage locations.

### Creating New Sessions

```bash
# Auto-generated session name
ralph ask 'What is Rust?'

# User-provided session name
ralph ask --session my-query 'What is Rust?'
```

Session names must be lowercase with format `adjective-noun` (e.g., `quiet-mountain`).

### Continuing Sessions

The `--continue` flag continues an existing session, preserving conversation history:

```bash
# Continue most recent session for current project
ralph ask --continue 'Follow-up question'

# Continue a specific session
ralph ask --session my-query --continue 'Follow-up question'
```

When continuing:
- Previous prompts and responses are loaded from iteration logs
- History is formatted as `<conversation_history>` XML block
- New prompt is appended after history
- Iteration sequence number increments

### Cloning Sessions

The `--clone-session` flag creates a new session with history from an existing one:

```bash
# Clone from specific session
ralph ask --session my-query --clone-session 'New direction'

# Clone from most recent session
ralph ask --continue --clone-session 'Branch question'
```

Cloning differs from continuing:
- Creates a **new** session with auto-generated name
- Original session remains unchanged
- New session's metadata records `cloned_from` source
- Useful for exploring alternative directions

### Session Lifecycle

```
┌─────────────────────────────────────────────────────────────┐
│ SESSION CREATION                                            │
│ 1. Resolve slug (user-provided or auto-generated)           │
│ 2. Create session directory                                 │
│ 3. Write session.toml metadata                              │
│ 4. Add entry to global sessions index                       │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ EXECUTION                                                   │
│ 1. Build command with permission mode                       │
│ 2. Format prompt (with conversation history if continuing)  │
│ 3. Invoke claude subprocess with spinner                    │
│ 4. Process streaming JSON output                            │
│ 5. Write iteration log                                      │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ FINALIZATION                                                │
│ 1. Update session outcome (Completed or Failed)             │
│ 2. Display summary (cost, duration, tokens)                 │
└─────────────────────────────────────────────────────────────┘
```

## Conversation History

### Viewing History

The `--history` flag displays conversation history for a session:

```bash
# View history for specific session
ralph ask --session my-query --history

# View history for most recent session
ralph ask --continue --history
```

Output format:
```
╭─── Conversation History: my-query ────────────────────────────╮

Turn 1:
[User]: What is Rust?

[Assistant]: Rust is a systems programming language...

Turn 2:
[User]: How do I handle errors?

[Assistant]: Rust uses the Result type...

╰───────────────────────────────────────────────────────────────╯
```

### History with Continuation

You can view history and continue in one command:

```bash
ralph ask --continue --history 'Another question'
```

This displays history first, then proceeds with the new prompt.

### History Format in Prompts

When continuing a session, history is formatted as:

```xml
<conversation_history>
[User]: Previous prompt 1

[Assistant]: Previous response 1

[User]: Previous prompt 2

[Assistant]: Previous response 2
</conversation_history>

[User]: New prompt here
```

## Permission Modes

The `--permission-mode` flag controls how Claude handles tool execution:

| Mode | Description | Use Case |
|------|-------------|----------|
| `default` | Requires approval for all tools | Maximum control |
| `acceptEdits` | Auto-accepts file edits, requires approval for others | Trusted editing |
| `plan` | Read-only mode, no tools can modify files | Research/exploration |
| `bypassPermissions` | Auto-accepts all tool executions | **Default for ask** |

### Security Considerations

`bypassPermissions` is the default for `ask` because:
- Single-shot queries often need quick tool access
- User is present and watching output
- Session can be interrupted with Ctrl+C

For untrusted environments, use a more restrictive mode:

```bash
ralph ask --permission-mode default 'Analyze this code'
```

### Configuration Precedence

Permission mode is resolved in this order:

1. CLI flag (`--permission-mode`)
2. Environment variable (`RALPH_PERMISSION_MODE`)
3. Config file (`~/.config/ralph/config.toml` under `[ask]` section)
4. Default (`bypassPermissions`)

Config file example:
```toml
[ask]
permission_mode = "acceptEdits"
```

## Theme Configuration

Theme settings follow the same precedence as `ralph run`:

1. CLI flag (`--theme`, `--no-background`)
2. Environment variable (`RALPH_THEME`, `RALPH_NO_BACKGROUND`)
3. Config file (if present)
4. Default: `Monokai Extended`

## Output Patterns

### Prompt Display

Before execution (unless `--no-prompt`):
```
╭─── Prompt ────────────────────────────────────────────────────╮
│ What is Rust?                                                 │
╰───────────────────────────────────────────────────────────────╯
```

### Streaming Response

Response is streamed with syntax highlighting for code blocks.

### Summary

After completion:
```
  cost: $0.0234  duration: 5.2s  model: claude-sonnet-4-20250514
  tokens: 1,234 in / 567 out
  session: quiet-mountain
```

## Interaction with Other Commands

### ralph sessions

Lists all sessions including those created by `ask`:

```bash
ralph sessions
```

### ralph iterations

Lists iterations, including ask command iterations:

```bash
ralph iterations --session my-query
```

### ralph replay

Replays ask sessions with full output:

```bash
ralph replay my-query
```

## Examples

### Basic Usage

```bash
# Simple question
ralph ask 'What is the capital of France?'

# Multi-line prompt from heredoc
ralph ask <<'EOF'
Review this function for potential improvements:

fn add(a: i32, b: i32) -> i32 {
    a + b
}
EOF

# Prompt from file
ralph ask /path/to/detailed-prompt.txt
```

### Session Continuation

```bash
# Start a conversation
ralph ask --session code-review 'Review src/main.rs'

# Continue the conversation
ralph ask --session code-review --continue 'Focus on error handling'

# Continue with the most recent session
ralph ask --continue 'What about performance?'
```

### Session Cloning

```bash
# Start a session
ralph ask --session original 'Implement feature X'

# Clone to explore alternative
ralph ask --session original --clone-session 'Try a different approach'

# Original session preserved, new session created with history
```

### History Review

```bash
# View history without sending new prompt
ralph ask --session my-chat --history

# View history then continue
ralph ask --continue --history 'Based on our discussion...'
```

### Piping

```bash
# Pipe prompt from another command
cat requirements.txt | ralph ask 'Analyze these dependencies'

# Pipe through multiple tools
git diff | ralph ask 'Review this diff'

# Use with process substitution
ralph ask < <(echo "Combined prompt from $(pwd)")
```

### Permission Control

```bash
# Read-only exploration
ralph ask --permission-mode plan 'What files are in src/?'

# Trusted editing
ralph ask --permission-mode acceptEdits 'Add error handling to main.rs'

# Full automation (default)
ralph ask 'Refactor the test suite'
```

### Theme and Display

```bash
# Custom theme
ralph ask --theme 'Solarized (dark)' 'prompt'

# No background colors
ralph ask --no-background 'prompt'

# Hide prompt display
ralph ask --no-prompt 'prompt'

# Verbose tool output
ralph ask --verbose-tools=bash 'Run the tests'
```

## Related Documentation

- [ralph run Command](ralph-run.md) - PRD-driven iteration loop
- [Runtime Control](runtime-control.md) - Signal-based interruption (applies to ask)
- [Prompt Template System](prompt-template.md) - For understanding prompt formats
