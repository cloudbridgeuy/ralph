# Configuration

Ralph uses a TOML configuration file for persistent settings. This document covers file locations, available options, and configuration precedence.

## Configuration File Location

Ralph stores its configuration in platform-specific directories:

| Platform | Config Directory | Config File |
|----------|------------------|-------------|
| **Linux** | `~/.config/ralph/` | `~/.config/ralph/config.toml` |
| **macOS** | `~/Library/Application Support/ralph/` | `~/Library/Application Support/ralph/config.toml` |
| **Windows** | `%APPDATA%\ralph\` | `%APPDATA%\ralph\config.toml` |

### Environment Variable Overrides

You can override the config directory with an environment variable:

| Variable | Purpose |
|----------|---------|
| `RALPH_CONFIG_DIR` | Override config directory (where `config.toml` is stored) |

Example:
```bash
# Use a custom config directory
export RALPH_CONFIG_DIR=/path/to/custom/config
ralph ask 'your prompt'
```

## Configuration Precedence

Settings are resolved in this order (highest priority first):

1. **CLI flags** (e.g., `--theme "Solarized (dark)"`)
2. **Environment variables** (e.g., `RALPH_THEME=Solarized (dark)`)
3. **Config file** (`config.toml` in config directory)
4. **Default values**

This means CLI flags always win, allowing you to override config file settings for specific invocations.

## Configuration File Format

The config file uses TOML format. All sections and fields are optional - only include what you want to customize.

### Complete Example

```toml
# Ralph Configuration File
# Location: ~/.config/ralph/config.toml (Linux)
#           ~/Library/Application Support/ralph/config.toml (macOS)
#           %APPDATA%\ralph\config.toml (Windows)

[theme]
# Syntax highlighting theme for code blocks and diffs
# Can be a built-in theme name or path to a .tmTheme file
name = "base16-ocean.dark"

# When true, don't apply theme background colors
# Useful if you want your terminal's background to show through
no_background = false
```

### Minimal Example

```toml
# Just set the theme, everything else uses defaults
[theme]
name = "Solarized (dark)"
```

## Configuration Sections

### `[theme]` - Syntax Highlighting

Controls how code blocks and diffs are displayed in the terminal.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | string | `"base16-ocean.dark"` | Theme name or path to `.tmTheme` file |
| `no_background` | bool | `false` | Disable theme background colors |

#### Built-in Themes

Run `ralph themes` to see all available themes:

```
Available syntax highlighting themes:

  InspiredGitHub
  Solarized (dark)
  Solarized (light)
  base16-eighties.dark
  base16-mocha.dark
  base16-ocean.dark (default)
  base16-ocean.light
```

#### Using a Custom Theme

You can use any TextMate-compatible `.tmTheme` file:

```toml
[theme]
name = "/path/to/my-custom-theme.tmTheme"
```

#### Theme Environment Variable

The `RALPH_THEME` environment variable can be used instead of the config file:

```bash
# Set theme via environment variable
export RALPH_THEME="Solarized (dark)"
ralph ask 'your prompt'

# Or for a single command
RALPH_THEME="Monokai Extended" ralph ask 'your prompt'
```

#### Disabling Background Colors

If your terminal has a specific background color you want to preserve:

```toml
[theme]
name = "base16-ocean.dark"
no_background = true
```

Or via environment variable:
```bash
export RALPH_NO_BACKGROUND=1
```

Or via CLI flag:
```bash
ralph ask --no-background 'your prompt'
```

### `[ask]` - Single-Shot Prompts

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `permission_mode` | string | `"bypassPermissions"` | Tool permission mode for `ralph ask` |

Precedence: `ask --permission-mode`, then `RALPH_ASK_PERMISSION_MODE`, then this key, then `bypassPermissions`. `[chat] permission_mode` does not apply to ask. See [ralph ask](context/ralph-ask.md#permission-modes).

### `[chat]` - Persistent Chat

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `mcp_servers` | list of strings | `["mcptools"]` | Servers from `~/.claude.json` to load in chat turns |
| `skills` | list of paths | `[]` | Skill directories exposed to chat |
| `frozen_window` | integer | `32` | Entries per frozen memory-view window, from 1 to 199 |
| `worker_check_secs` | integer | `60` | Seconds between worker checks while chat is open; `0` turns the check off |
| `worker_base` | string | unset | Base branch that `close_worker` checks for unmerged commits |
| `worktree_dir` | string | `.worktrees` | Directory that holds worker worktrees; a leading `~` expands to home, relative paths resolve against the repo root |
| `permission_mode` | string | `"auto"` | `claude --permission-mode` for chat turns and claude workers |
| `inbox_alert_sound` | bool | `true` | Play a sound when a worker report arrives and the chat is not focused |
| `inbox_alert_notification` | bool | `false` | Also send a system notification (`<id> <status>`) |
| `inbox_alert_unknown_focus` | `"alert"` or `"silent"` | `"alert"` | What to do when focus cannot be detected (outside tmux, no attached client, tmux query failed) |

`--skill` and `RALPH_CHAT_SKILLS` replace `skills`. For `permission_mode` the precedence is `chat --permission-mode`, then `RALPH_CHAT_PERMISSION_MODE`, then this key, then `auto`; chat passes the resolved mode to every claude worker it spawns, so chat and workers always match. `--frozen-window <N>` overrides `frozen_window`; a value outside 1-199 stops chat start with an error. Without `worker_base`, `close_worker` uses the branch recorded at spawn (`branch.worker/<ID>.ralphBase`), then `origin/HEAD`, then `main`. For `worktree_dir` the precedence is `chat --worktree-dir`, then `RALPH_WORKTREE_DIR`, then this key, then `.worktrees`. A worker that already has a worktree keeps it: spawn and close look up `worker/<ID>` in `git worktree list` first, so changing the key does not lose running workers. A directory inside the repo is added to `.git/info/exclude`. A config that fails to read or parse skips `worktree_dir`, and the spawn or close result ends with a warning. `worker_check_secs` has no flag or env var; the check is described in [ralph chat](context/ralph-chat.md#worker-inbox). `chat --no-inbox-sound` and `--no-inbox-notification` turn off the matching alert for one run; focus rules are in [ralph chat](context/ralph-chat.md#worker-inbox). See [ralph chat](context/ralph-chat.md).

## Chat Store

`ralph ask` stores nothing. `ralph chat` keeps its store at `~/.ralph/chat`. See [ralph chat](context/ralph-chat.md).

## Verbose Tools Configuration

The `--verbose-tools` flag controls how much detail is shown for LLM tool invocations during `ralph ask`.

### Usage

```bash
# Enable verbose output for all tools
ralph ask --verbose-tools 'your prompt'

# Enable verbose for specific tools (comma-separated, case-insensitive)
ralph ask --verbose-tools=read,grep,bash 'your prompt'

# Single tool
ralph ask --verbose-tools=glob 'your prompt'
```

### Supported Tools

| Tool | Verbose Output Shows |
|------|---------------------|
| `read` | Full file content with syntax highlighting and line numbers |
| `edit` | Full diff with syntax highlighting |
| `write` | Full diff showing new/changed content |
| `glob` | All matched files grouped by directory |
| `grep` | Full regex pattern and matched content |
| `bash` | Full command and output with shell syntax highlighting |
| `todowrite` | Full todo list with status indicators |
| `notebookedit` | Full cell diff for Jupyter notebooks |

### Non-Verbose Mode (Default)

Without `--verbose-tools`, tool output is shown in a compact format:
- File paths shown without content
- Large outputs truncated
- Pattern matches summarized

## Environment Variables Reference

| Variable | Purpose | Example |
|----------|---------|---------|
| `RALPH_CONFIG_DIR` | Override config directory | `/custom/config` |
| `RALPH_THEME` | Set syntax highlighting theme | `Solarized (dark)` |
| `RALPH_NO_BACKGROUND` | Disable theme backgrounds | `1` |
| `RALPH_ASK_PERMISSION_MODE` | Permission mode for `ralph ask` | `acceptEdits` |
| `RALPH_CHAT_PERMISSION_MODE` | Permission mode for chat turns and claude workers | `auto` |
| `RALPH_WORKTREE_DIR` | Directory for worker worktrees; `~` expands, relative paths resolve against the repo root | `.worktrees` |
| `RALPH_CHAT_SKILLS` | `:`-separated skill directories for chat | `~/skills/a:~/skills/b` |

## Troubleshooting

### Finding Your Config Location

If you're unsure where ralph is looking for configuration:

```bash
# Check what paths ralph uses
ralph --version
# Then check the platform-specific path for your OS
```

### Config File Not Loading

1. Verify the file exists at the correct path
2. Check file permissions (must be readable)
3. Validate TOML syntax - even small errors prevent loading:
   ```bash
   # Test TOML validity
   cat ~/.config/ralph/config.toml | python3 -c "import sys, tomllib; tomllib.loads(sys.stdin.read())"
   ```

### Theme Not Applying

1. Verify the theme name matches exactly (run `ralph themes` for the list)
2. For custom themes, ensure the file path is absolute and the file exists
3. Check if a CLI flag or environment variable is overriding your config
