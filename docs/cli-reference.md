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
| `run` | Run the iteration loop to process user stories |
| `sessions` | List all sessions across all projects |
| `iterations` | List all iterations across all sessions |
| `replay` | Replay a session's output with syntax highlighting |
| `themes` | List available syntax highlighting themes |

---

## ralph run

Run the iteration loop to process user stories from a PRD.

### Usage

```
ralph run [OPTIONS] [ITERATIONS]
```

### Arguments

| Argument | Description |
|----------|-------------|
| `[ITERATIONS]` | Maximum number of iterations to run. Defaults to the number of pending stories in the PRD. The loop exits early if all stories are completed or the completion marker is found in output. |

### Options

#### Session Options

| Option | Short | Default | Description |
|--------|-------|---------|-------------|
| `--slug <SLUG>` | `-s` | Auto-generated | Session identifier. Used to name the session directory for logs. Auto-generated as adjective-noun (e.g., "quiet-mountain") if omitted. |

#### Prompt Options

| Option | Short | Default | Description |
|--------|-------|---------|-------------|
| `--prompt <PROMPT>` | `-p` | Built-in template | Custom prompt template. Supports file path, `-` for stdin, or inline string. Placeholders: `{design_file}`, `{prd_file}`, `{progress_file}` |
| `--additional-prompt <TEXT>` | `-a` | None | Additional instructions to append to the prompt. Supports file path, `-` for stdin, or inline string. |
| `--command <TEMPLATE>` | `-c` | `claude --permission-mode acceptEdits --output-format stream-json -p {prompt}` | Custom LLM invocation command template with `{prompt}` placeholder. |

#### File Paths

| Option | Default | Description |
|--------|---------|-------------|
| `--design <PATH>` | `.local/designs/design.md` | Design document path |
| `--prd <PATH>` | `.local/plans/prd.toml` | PRD (Product Requirements Document) file path |
| `--progress <PATH>` | `.local/plans/progress.txt` | Progress notes file path |

#### Execution Options

| Option | Default | Description |
|--------|---------|-------------|
| `--max-attempts <N>` | `3` | Maximum failure recovery attempts. Number of times to automatically re-attempt if the LLM subprocess fails. After exhausting all attempts, prompts user for action. |
| `--timeout <SECONDS>` | `600` | Timeout for LLM subprocess in seconds. If exceeded, the subprocess is killed and treated as a failure (retry logic applies). |
| `--completion-marker <STRING>` | `<promise>COMPLETE</promise>` | Custom completion marker. When found in LLM output, exits the loop immediately. |

#### Progress Summarization

| Option | Default | Description |
|--------|---------|-------------|
| `--progress-max-lines <N>` | `1000` | Maximum lines in progress file before auto-summarization. Set to 0 to disable. |
| `--summarize-command <CMD>` | `claude -p {prompt}` | Command to invoke for progress file summarization. |
| `--summarize-prompt <PROMPT>` | Built-in template | Custom prompt for summarization. Placeholders: `{progress_file}`, `{progress_content}` |
| `--no-summarize` | `false` | Disable automatic progress file summarization entirely. |

#### Display Options

| Option | Default | Description |
|--------|---------|-------------|
| `--theme <NAME>` | `base16-ocean.dark` | Syntax highlighting theme. Use a built-in theme name or path to a custom `.tmTheme` file. Run `ralph themes` to list available themes. Can also be set via `RALPH_THEME` environment variable. |
| `--no-background` | `false` | Disable background colors in syntax highlighting. Allows terminal's default background to show through. Can also be set via `RALPH_NO_BACKGROUND` environment variable. |
| `--verbose-tools [TOOLS]` | Disabled | Enable verbose output for specific tools. Without value, enables for all tools. Accepts comma-separated list of tool names (case-insensitive). |

### Examples

```bash
# Run with default settings (iterations = pending story count)
ralph run

# Run exactly 5 iterations
ralph run 5

# Run with custom session name
ralph run --slug my-feature

# Run with custom prompt file
ralph run --prompt ./custom-prompt.txt

# Run with additional instructions
ralph run -a "Focus on error handling"

# Run with verbose output for Read and Grep tools
ralph run --verbose-tools=read,grep

# Run with all verbose tools enabled
ralph run --verbose-tools

# Run with custom theme and no background
ralph run --theme "Solarized (dark)" --no-background

# Run with longer timeout (20 minutes)
ralph run --timeout 1200

# Run with custom completion marker
ralph run --completion-marker "DONE"

# Disable progress auto-summarization
ralph run --no-summarize
```

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
ralph run

# Use custom config directory
export RALPH_CONFIG_DIR=/custom/config
ralph run

# One-off theme override
RALPH_THEME="Monokai Extended" ralph run
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
