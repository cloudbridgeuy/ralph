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
| `iterations` | List all iterations across all sessions |
| `replay` | Replay a session's output with syntax highlighting |
| `themes` | List available syntax highlighting themes |

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
| `completed` | All stories completed successfully |
| `aborted` | User chose to abort at failure prompt |
| `failed` | Session failed after exhausting retries |
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

## ralph iterations

List all iterations across all sessions.

### Usage

```
ralph iterations [OPTIONS]
```

### Options

| Option | Description |
|--------|-------------|
| `--session <SLUG>` | Filter iterations by session slug |
| `--project <PATH>` | Filter iterations by project path (substring match) |
| `--outcome <STATUS>` | Filter iterations by outcome |

### Outcome Values

| Value | Description |
|-------|-------------|
| `completed` | Iteration completed with exit code 0 |
| `failed` | Iteration failed with non-zero exit code |

### Examples

```bash
# List all iterations
ralph iterations

# Filter by session
ralph iterations --session quiet-mountain

# Filter by project
ralph iterations --project /path/to/project

# Filter by outcome
ralph iterations --outcome failed

# Combine filters
ralph iterations --session my-session --outcome completed
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

### Examples

```bash
# Set theme via environment variable
export RALPH_THEME="Solarized (dark)"
ralph strategy execute prd-loop

# Use custom config directory
export RALPH_CONFIG_DIR=/custom/config
ralph strategy execute prd-loop

# One-off theme override
RALPH_THEME="Monokai Extended" ralph strategy execute prd-loop
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
