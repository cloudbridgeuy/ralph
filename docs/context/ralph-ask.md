# ralph ask Command

This document provides comprehensive documentation for the `ralph ask` command, which sends a single-shot prompt to Claude. Nothing is stored.

## Code Locations

| Component | File | Description |
|-----------|------|-------------|
| CLI args | `crates/ralph/src/cli/mod.rs` | AskArgs struct definition |
| Permission mode | `crates/ralph/src/invoke.rs`, `crates/ralph/src/config.rs` | resolve_permission_mode(), AppConfig::permission_mode() |
| Shared engine | `crates/ralph/src/invoke.rs` | InvocationConfig, invoke(), command building |
| Prompt source | `crates/ralph/src/prompt_source.rs` | classify_prompt_source(), read_from_source() |
| Execution | `crates/ralph/src/main.rs` | execute_ask(), build_invocation_config(), resolve_ask_prompt() |
| Summary | `crates/ralph/src/startup/display.rs` | display_ask_summary() |
| Config | `crates/ralph/src/config.rs` | AppConfig with top-level permission_mode |

## Purpose

The `ralph ask` command provides a streamlined interface for quick LLM interactions. `ask` sends a single prompt and displays the response. It keeps no session, history, or log on disk; each call is independent. Use [ralph chat](ralph-chat.md) for a conversation that persists.

## CLI Flags

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `[PROMPT]` | positional | stdin | The prompt to send (inline, file path, or `-` for stdin) |
| `--theme` | string | `base16-ocean.dark` | Syntax highlighting theme |
| `--no-background` | flag | false | Disable theme background colors |
| `--timeout` | u64 | 600 | Subprocess timeout in seconds |
| `--verbose-tools` | string | none | Verbose output for tools (omit value for all, or specify names) |
| `--no-prompt` | flag | false | Suppress prompt display before execution |
| `--permission-mode` | string | `auto` | Permission mode for tool execution |

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

## Permission Modes

The `--permission-mode` flag sets `claude --permission-mode`. `ask`, `ralph chat`, and chat's claude workers share one setting and one default, `auto`. Choices are claude's: `acceptEdits`, `auto`, `bypassPermissions`, `manual`, `dontAsk`, `plan` (see `claude --help`).

```bash
ralph ask --permission-mode plan 'Analyze this code'
```

### Configuration Precedence

1. CLI flag (`--permission-mode`)
2. Environment variable (`RALPH_PERMISSION_MODE`, ignored when empty)
3. Top-level `permission_mode` in `~/.config/ralph/config.toml`
4. Default (`auto`)

`[ask] permission_mode` is no longer read.

Config file example:
```toml
[ask]
permission_mode = "acceptEdits"
```

## Theme Configuration

Theme settings follow this precedence:

1. CLI flag (`--theme`, `--no-background`)
2. Environment variable (`RALPH_THEME`, `RALPH_NO_BACKGROUND`)
3. Config file (if present)
4. Default: `base16-ocean.dark`

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
✓ Ask Complete

─── Metrics ───
Cost: $0.0234
Duration: 5.2s
Tokens: 1.2K input | 567 output
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

- [ralph chat Command](ralph-chat.md) - Persistent chat with compressed memory
