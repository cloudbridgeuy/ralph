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

#### Per-role themes: `[theme.assistant]` and `[theme.agent]`

Each role has its own markdown theme. `assistant` styles the replies of `ralph ask` and `ralph chat`. `agent` styles worker messages that `ralph chat` shows from `/inbox`, including the worker header box.

```toml
[theme.assistant]
code = "Monokai Extended"   # code-block theme; overrides [theme] name
heading1 = "cyan"

[theme.agent]
code = "Solarized (dark)"   # default: the assistant code theme
heading1 = "#ff6ac1"
inline_code = "dark_cyan"
border = "magenta"
```

| Key | Assistant default | Agent default | Styles |
|-----|-------------------|---------------|--------|
| `code` | `[theme] name` | assistant code theme | Syntax theme for fenced code blocks |
| `heading1`, `heading2`, `heading3` | `cyan`, `blue`, `magenta` | `green`, `dark_green`, `dark_yellow` | Headings (always bold) |
| `bold`, `italic` | terminal default | terminal default | Emphasis color |
| `inline_code` | `yellow` | `dark_cyan` | Inline code (on a dark gray background) |
| `bullet` | `dark_grey` | `dark_green` | List bullets |
| `quote`, `table` | termimad default | termimad default | Quote mark, table borders |
| `link`, `url` | `blue`, `cyan` | `blue`, `cyan` | Link label (underlined), link URL |
| `memory_ref` | `magenta` | `magenta` | `#N` memory references |
| `border` | `dark_blue` | `dark_blue` | Worker header box border |
| `done`, `blocked`, `waiting` | `dark_green`, `dark_red`, `dark_yellow` | same | Worker status (`failed` uses `blocked`, `check` uses `waiting`) |

A color is a name (`black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `white`, `grey`, their `dark_` forms, `dark_grey`, `reset`) or `#rrggbb`. `ralph chat` warns at start for an unknown key or a bad color and keeps the default for it. Plain output stays plain when stdout is not a TTY or `NO_COLOR` is set.

#### Status line colors: `[theme.status]`

`ralph chat` paints its status line from this table. Every key is optional and takes the same color values as the role themes. Keys that are missing keep the defaults below, so an empty or missing table draws the same bar as before. `ralph chat` warns at start for an unknown key or a bad color and keeps the default for it.

```toml
[theme.status]
mode_idle_fg = "black"    # IDLE block text
mode_idle_bg = "green"    # IDLE block background
mode_nap_fg = "black"     # NAP block text
mode_nap_bg = "magenta"   # NAP block background
mode_inbox_fg = "black"   # INBOX block text
mode_inbox_bg = "yellow"  # INBOX block background
mode_work_fg = "white"    # WORK block text
mode_work_bg = "blue"     # WORK block background
store_fg = "reset"        # store path text
workers_fg = "reset"      # workers text
watches_fg = "reset"      # watches text
memory_fg = "reset"       # memory size text
base_bg = "#303030"       # bar background: filler, store, workers, watches, memory
```

The default `base_bg` is ANSI color 236, written `#303030` above. `reset` is the terminal's own text color. Powerline glyphs take their colors from the neighbouring blocks, so they have no keys.

Code theme precedence for the assistant: `--theme` > `RALPH_THEME` > `[theme.assistant] code` > `[theme] name` > default.

The old `[theme] agent = "<theme>"` string still works as `[theme.agent] code = "<theme>"`; `ralph chat` prints a deprecation warning for it.

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
| `watch_timeout_secs` | integer | `3600` | Seconds before a watch is killed and reports that it timed out; applies to every watch without its own `timeout_secs`; `0` is an error |
| `wrap_width` | integer | unset | Column cap for chat replies and the prompt area; `0` or unset wraps at the terminal width |
| `cell_output_limit` | integer | `200` | Lines of cell output sent to Claude: first half and last half kept; `0` sends all |
| `cell_output_bytes` | integer | `65536` | Bytes of cell output sent to Claude: head and tail kept; `0` sends all |
| `worker_base` | string | unset | Base branch that `close_worker` checks for unmerged commits |
| `worktree_dir` | string | `.worktrees` | Directory that holds worker worktrees; a leading `~` expands to home, relative paths resolve against the repo root |
| `permission_mode` | string | `"auto"` | `claude --permission-mode` for chat turns and claude workers |
| `inbox_alert_sound` | bool | `true` | Play a sound when a worker report arrives and the chat is not focused |
| `inbox_alert_notification` | bool | `false` | Also send a system notification (`<id> <status>`) |
| `inbox_alert_unknown_focus` | `"alert"` or `"silent"` | `"alert"` | What to do when focus cannot be detected (outside tmux, no attached client, tmux query failed) |
| `status_powerline` | bool | `false` | Use powerline glyphs (`\ue0b0`, `\ue0b2`) between status line segments; needs a patched font. Off draws plain colour blocks |

`--skill` and `RALPH_CHAT_SKILLS` replace `skills`. For `permission_mode` the precedence is `chat --permission-mode`, then `RALPH_CHAT_PERMISSION_MODE`, then this key, then `auto`; chat passes the resolved mode to every claude worker it spawns, so chat and workers always match. `--frozen-window <N>` overrides `frozen_window`; a value outside 1-199 stops chat start with an error. Without `worker_base`, `close_worker` uses the branch recorded at spawn (`branch.worker/<ID>.ralphBase`), then `origin/HEAD`, then `main`. For `worktree_dir` the precedence is `chat --worktree-dir`, then `RALPH_WORKTREE_DIR`, then this key, then `.worktrees`. A worker that already has a worktree keeps it: spawn and close look up `worker/<ID>` in `git worktree list` first, so changing the key does not lose running workers. A directory inside the repo is added to `.git/info/exclude`. A config that fails to read or parse skips `worktree_dir`, and the spawn or close result ends with a warning. For `wrap_width` the precedence is `chat --wrap-width <N>`, then `RALPH_WRAP_WIDTH`, then this key; a value that is not a whole number stops chat start with an error. The width used is the smaller of this cap and the terminal width, read at each render, so a narrower terminal still wins. The cap is read once at chat start: a change to this key or `RALPH_WRAP_WIDTH` needs a chat restart; a terminal resize does not. It applies to rendered prose (chat turns and drained worker reports) and to the reedline prompt area: the status line, the draft, and the footer. Code blocks are not wrapped. For `cell_output_limit` and `cell_output_bytes` the precedence is `RALPH_CELL_OUTPUT_LIMIT` / `RALPH_CELL_OUTPUT_BYTES`, then the key, then the default; a value that is not a whole number stops chat start with an error. The caps shape only what Claude receives; the terminal shows all output in `result` fences. See [executable cells](context/ralph-chat.md#executable-cells). `worker_check_secs` has no flag or env var; the check is described in [ralph chat](context/ralph-chat.md#worker-inbox). `chat --no-inbox-sound` and `--no-inbox-notification` turn off the matching alert for one run; focus rules are in [ralph chat](context/ralph-chat.md#worker-inbox). See [ralph chat](context/ralph-chat.md).

### `[mermaid]` - Diagram Rendering

In a terminal, a fenced `mermaid` code block renders as a Unicode text diagram for flowchart/graph, sequenceDiagram, erDiagram, and classDiagram. Rendering runs in a child process (`ralph __mermaid`) that ralph kills on timeout or memory overrun. Any refusal, kill, error, or output wider than the terminal shows the highlighted source with no error text. Piped output keeps the raw source.

| Key | Default | Meaning |
| --- | --- | --- |
| `timeout_ms` | `500` | Kill the render child after this many milliseconds |
| `max_rss_mb` | `256` | Kill the child above this resident size (macOS and Linux only; elsewhere only the timeout applies) |
| `max_input_bytes` | `1048576` | Refuse a block with more source bytes |
| `max_flowchart_edges` | `50` | Refuse a flowchart/graph with more arrow tokens |
| `max_class_relations` | `400` | Refuse a classDiagram with more relation tokens (provisional: dense graphs among few classes can exceed the timeout well below 400; the timeout still bounds them) |

Fixed limits: flowchart source up to 16 KB; sequenceDiagram up to 20 participants, 64 KB, and 2000 statements; erDiagram up to 64 KB and 2000 statements. A config that fails to load prints a warning and uses these defaults.

## Chat Store

`ralph ask` stores nothing. `ralph chat` keeps its store at `~/.ralph/chat`. For another path the precedence is `chat --store <DIR>`, then `RALPH_CHAT_STORE`, then `~/.ralph/chat`; a relative path resolves against the cwd at startup. Parallel chats need separate stores. See [ralph chat](context/ralph-chat.md).

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
| `RALPH_CHAT_STORE` | Chat store directory; relative paths resolve against the cwd | `~/chats/a` |
| `RALPH_CHAT_SKILLS` | `:`-separated skill directories for chat | `~/skills/a:~/skills/b` |
| `RALPH_WRAP_WIDTH` | Column cap for chat replies and the prompt area; `0` = terminal width | `80` |

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
