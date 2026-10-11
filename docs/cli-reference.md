# CLI Reference

Complete reference for all ralph commands and options.

## Global Options

```
ralph [OPTIONS] <COMMAND>
```

| Option | Description |
|--------|-------------|
| `--version` | Print version information |
| `--help` | Print help information |

## Commands

| Command | Description |
|---------|-------------|
| `sessions` | List all sessions across all projects |
| `replay` | Replay a session's output with syntax highlighting |
| `themes` | List available syntax highlighting themes |
| `ask` | Send a single-shot prompt to the LLM ([details](context/ralph-ask.md)) |
| `chat` | Chat with a persistent, compressed memory; default when no command is given ([details](context/ralph-chat.md)) |

---

## ralph sessions

List all sessions across all projects.

### Usage

```
ralph sessions [OPTIONS]
```

### Options

| Option | Description |
|--------|-------------|
| `--project <PATH>` | Filter sessions by project path (substring match) |
| `--outcome <STATUS>` | Filter sessions by outcome status |

### Outcome Values

| Value | Description |
|-------|-------------|
| `in_progress` | Session is currently running or crashed without cleanup |
| `completed` | Session completed successfully |
| `aborted` | Session was aborted (no current command sets it) |
| `failed` | Session failed |
| `interrupted` | Session was interrupted by signal (Ctrl+C) |

### Examples

```bash
# List all sessions
ralph sessions

# Filter by project path
ralph sessions --project /path/to/project

# Filter by outcome
ralph sessions --outcome completed

# Combine filters
ralph sessions --project myproject --outcome in_progress
```

---

## ralph replay

Replay a session's output with syntax highlighting.

### Usage

```
ralph replay [OPTIONS] <SLUG>
```

### Arguments

| Argument | Description |
|----------|-------------|
| `<SLUG>` | Session identifier (slug) to replay. Use `ralph sessions` to list available sessions. |

### Options

| Option | Short | Default | Description |
|--------|-------|---------|-------------|
| `--iteration <N>` | `-i` | All | Only replay a specific iteration (1-indexed). If omitted, all iterations are replayed in order. |
| `--theme <NAME>` | | Config file | Syntax highlighting theme. Use a built-in theme name or path to a custom `.tmTheme` file. |
| `--no-background` | | `false` | Disable background colors in syntax highlighting. |
| `--no-prompt` | | `false` | Do not show the stored prompt before iteration 1. |
| `--delay <SECONDS>` | | none | Pause after each output block. Fractional seconds are allowed (e.g., `0.5`). |

### Examples

```bash
# Replay entire session
ralph replay quiet-mountain

# Replay specific iteration
ralph replay quiet-mountain -i 3

# Replay with custom theme
ralph replay quiet-mountain --theme "Monokai Extended"

# Replay without background colors
ralph replay quiet-mountain --no-background

# Replay with a half-second pause between blocks
ralph replay quiet-mountain --delay 0.5
```

---

## ralph themes

List available syntax highlighting themes.

### Usage

```
ralph themes
```

### Description

Displays all built-in themes that can be used with the `--theme` flag. Custom `.tmTheme` files can also be loaded by specifying a file path instead of a theme name.

### Built-in Themes

- `base16-eighties.dark`
- `base16-mocha.dark`
- `base16-ocean.dark` (default)
- `base16-ocean.light`
- `InspiredGitHub`
- `Monokai Extended`
- `Solarized (dark)`
- `Solarized (light)`

---

## Environment Variables

Environment variables can be used instead of CLI flags. CLI flags take precedence over environment variables.

| Variable | Equivalent Flag | Description |
|----------|-----------------|-------------|
| `RALPH_CONFIG_DIR` | N/A | Override config directory location |
| `RALPH_DATA_DIR` | N/A | Override data directory location (sessions) |
| `RALPH_THEME` | `--theme` | Set syntax highlighting theme |
| `RALPH_NO_BACKGROUND` | `--no-background` | Disable theme backgrounds (set to `1`) |
| `RALPH_PERMISSION_MODE` | `ask --permission-mode` | Permission mode for `ask` |
| `RALPH_CHAT_SKILLS` | `chat --skill` | `:`-separated skill directories for chat |

### Examples

```bash
# Set theme via environment variable
export RALPH_THEME="Solarized (dark)"
ralph ask 'your prompt'

# Use custom config directory
export RALPH_CONFIG_DIR=/custom/config
ralph ask 'your prompt'

# One-off theme override
RALPH_THEME="Monokai Extended" ralph ask 'your prompt'
```

---

## Exit Codes

| Code | Meaning |
|------|---------|
| `0` | Success - all operations completed successfully |
| `1` | Error - operation failed (see error message for details) |

---

## See Also

- [Configuration](configuration.md) - Detailed configuration file documentation
- [ralph themes](#ralph-themes) - List available syntax highlighting themes
