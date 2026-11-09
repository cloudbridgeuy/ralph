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

Run `ralph config check` to find unknown keys and invalid values in this file (see [CLI reference](cli-reference.md#ralph-config)).

## Configuration Precedence

Settings are resolved in this order (highest priority first):

1. **CLI flags** (e.g., `--theme "Solarized (dark)"`)
2. **Environment variables** (e.g., `RALPH_THEME=Solarized (dark)`)
3. **Store config file** (`<store>/config.toml`, chat commands only)
4. **Global config file** (`config.toml` in config directory)
5. **Default values**

This means CLI flags always win, allowing you to override config file settings for specific invocations.

The two files merge per key: a key in the store file replaces the same key in the global file, and every other global key stays. Tables merge recursively. Arrays replace, they never append. Either file may be missing.

Only the chat commands (`ralph chat` and its helper processes) have a store, so only they read the store file. `ralph ask`, `ralph wire` and every other command read only the global file.

Relative entries in `[chat] skills`, `prompt_file` and `append_prompt_files` resolve from the directory of the file that lists them; `~` and absolute entries stay as written, and `--skill` and `RALPH_CHAT_SKILLS` keep their own rules. `worktree_dir` stays relative to the repo root in both files.

What a store file can change: it can override any key. Ralph warns once per key when a store file sets one of these security-relevant keys: `chat.permission_mode`, `chat.mcp_servers`, `chat.user_settings`, `chat.prompt_file`, `chat.append_prompt_files`, `chat.skills`, `codex.permissions`, `cursor.permissions`, `claude.permissions`, `claude.config_dir`, `chat.editor`, `chat.user_claude_md`, `chat.empty_prompt` and `changes.diff_args`. If the store file fails to parse or validate, ralph warns and names the file, then uses the global file alone. `ralph config show` and `ralph config check` still report that failure as an error.

`ralph config show [--store DIR]` prints the merged config and the source of each value (default, global or store); a store value that hides a global one also shows the global value, as `[store; global = 80]`, and a value whose environment variable is set shows the variable's value, tagged `env NAME`, then every file value it hides. Flags do not apply to it. In a running chat, `/config` prints what that chat resolved at start: the start warnings, the config files, store and other paths in full (a file that does not exist is marked `(missing)`), the set variables, the start flags, and every value tagged default, global, store, env or flag, with each layer it overrides (see [ralph chat](context/ralph-chat.md#config)). `ralph config check --store DIR` checks both files and the merged result (see [CLI reference](cli-reference.md#ralph-config)).

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

Code fences use syntect's default syntaxes plus the extra set from `two-face` (bat's syntaxes, for example TypeScript, TSX, TOML, Dockerfile, Kotlin and Terraform). The fence tags `shell`, `jsx` and `jsonc` are read as `sh`, `tsx` and `json`. A fence with an unknown tag prints as plain text.

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
| `inline_code` | `yellow` | `dark_cyan` | Inline code text |
| `inline_code_bg` | ANSI `235` (dark gray) | ANSI `235` (dark gray) | Inline code background |
| `bullet` | `dark_grey` | `dark_green` | List bullets |
| `quote`, `table` | termimad default | termimad default | Quote mark, table borders |
| `link`, `url` | `blue`, `cyan` | `blue`, `cyan` | Link label (underlined), link URL |
| `memory_ref` | `magenta` | `magenta` | `#N` memory references |
| `border` | `dark_blue` | `dark_blue` | Worker header box border |
| `done`, `blocked`, `waiting` | `dark_green`, `dark_red`, `dark_yellow` | same | Worker status (`failed` uses `blocked`, `check` uses `waiting`) |

A color is a name (`black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `white`, `grey`, their `dark_` forms, `dark_grey`, `reset`), `#rrggbb`, or an ANSI 256 number from `0` to `255` (for example `236`). `ralph chat` warns at start for an unknown key or a bad color and keeps the default for it. Plain output stays plain when stdout is not a TTY or `NO_COLOR` is set.

#### Status line colors: `[theme.status]`

`ralph chat` paints its status line from this table. Every key is optional and takes the same color values as the role themes. Keys that are missing keep the defaults below, so an empty or missing table draws the same bar as before. The removed `memory_fg` key is an unknown key now: it is ignored with a warning. `ralph chat` warns at start for an unknown key or a bad color and keeps the default for it.

```toml
[theme.status]
mode_idle_fg = "black"    # IDLE block text
mode_idle_bg = "green"    # IDLE block background
mode_hold_fg = "black"    # HOLD block text
mode_hold_bg = "cyan"     # HOLD block background
mode_nap_fg = "black"     # NAP block text
mode_nap_bg = "magenta"   # NAP block background
mode_inbox_fg = "black"   # INBOX block text
mode_inbox_bg = "yellow"  # INBOX block background
mode_work_fg = "white"    # WORK block text
mode_work_bg = "blue"     # WORK block background
store_fg = "reset"        # session name text
workers_fg = "reset"      # workers text
watches_fg = "reset"      # watches and scheduled-task text
base_bg = "#303030"       # bar background: filler, name, workers, watches
```

The default `base_bg` is ANSI color 236, written `#303030` above. `reset` is the terminal's own text color. Powerline glyphs take their colors from the neighbouring blocks, so they have no keys.

#### Input colors: `[theme.input]`

`ralph chat` draws the `suggest_reply` pre-fill in its own color, so you can tell it from your own draft. The text goes back to the normal color on the first edit and stays normal, even if you retype the same words. Held and queued text keeps the normal color. The key is optional and takes the same color values as the role themes. `ralph chat` warns at start for an unknown key or a bad color and keeps the default for it.

```toml
[theme.input]
suggestion = "cyan"  # suggest_reply pre-fill text
```

#### Diff colors: `[theme.diff]`

ralph paints diffs and Edit, Write and NotebookEdit results with this table. Every key is optional and takes the same color values as the role themes. Keys that are missing keep the defaults below, so an empty or missing table paints the same bytes as before. File and diff headers stay bold and have no keys. `ralph chat` warns at start for an unknown key or a bad color and keeps the default for it. Plain output stays plain when stdout is not a TTY or `NO_COLOR` is set.

| Key | Default | Paints |
|-----|---------|--------|
| `add_fg` | `dark_green` | Added diff lines (`+`) |
| `del_fg` | `dark_red` | Removed diff lines (`-`) |
| `hunk_fg` | `dark_cyan` | Hunk headers (`@@`) |
| `add_bg` | `#2d822d` | Line-number background of added and "After" blocks |
| `del_bg` | `#8c2d2d` | Line-number background of "Before" blocks |

```toml
[theme.diff]
add_fg = "dark_green"
del_fg = "dark_red"
hunk_fg = "dark_cyan"
add_bg = "#2d822d"
del_bg = "#8c2d2d"
```

#### Tool colors: `[theme.tools]`

ralph paints tool calls and tool results with this table. It covers the invocation line (`▶ Name args`), the result marks, and the labels of Edit, Write and NotebookEdit results. Every key is optional and takes the same color values as the role themes. Keys that are missing keep the defaults below, so an empty or missing table paints the same bytes as before. Bold and dim text stay hard-coded and have no keys. `ralph chat` warns at start for an unknown key or a bad color and keeps the default for it. Plain output stays plain when stdout is not a TTY or `NO_COLOR` is set.

| Key | Default | Paints |
|-----|---------|--------|
| `name` | `dark_cyan` | Tool name and result headers |
| `args` | `dark_grey` | Invocation arguments |
| `label` | `dark_grey` | Secondary labels and dimmed result output |
| `ok` | `dark_green` | Success marks (`✓`) |
| `error` | `dark_red` | Error marks and messages (`✗`) |
| `accent` | `dark_yellow` | Pending marks (`⋯`) and inline indicators such as `(new file)` |
| `match_path` | `dark_grey` | Grep `file:line:` prefix |
| `match_text` | `yellow` | Grep match text |

```toml
[theme.tools]
name = "dark_cyan"
args = "dark_grey"
label = "dark_grey"
ok = "dark_green"
error = "dark_red"
accent = "dark_yellow"
match_path = "dark_grey"
match_text = "yellow"
```

#### UI colors: `[theme.ui]`

ralph paints the spinner, the `ralph ask` prompt and summary, warnings, success notices, and the rule between chat turns with this table. Every key is optional and takes the same color values as the role themes. Keys that are missing keep the defaults below, so an empty or missing table paints the same bytes as before. Bold, dim, and underline stay hard-coded and have no keys. `ralph chat` warns at start for an unknown key or a bad color and keeps the default for it. Warnings printed before the config loads use the default `warning` color. Plain output stays plain when stdout is not a TTY or `NO_COLOR` is set.

| Key | Default | Paints |
|-----|---------|--------|
| `spinner` | `dark_cyan` | Spinner frame |
| `ask_header` | `dark_magenta` | `▶ Prompt` header and rules of `ralph ask` |
| `ask_path` | `dark_cyan` | Attached file paths in the `ralph ask` prompt |
| `ask_ok` | `dark_green` | `✓ Ask Complete` line, and the `✓` symbol and box border of `ralph chat` success notices such as `codex session <id> resumed` |
| `ask_error` | `dark_red` | `✗ Ask Failed` line |
| `warning` | `dark_yellow` | Warning symbol and box border |
| `rule` | ANSI `238` | Rule printed between chat turns |

```toml
[theme.ui]
spinner = "dark_cyan"
ask_header = "dark_magenta"
ask_path = "dark_cyan"
ask_ok = "dark_green"
ask_error = "dark_red"
warning = "dark_yellow"
rule = "238"
```

`/view` mermaid pictures take their colors from this table too; see [`[mermaid.style]`](#mermaidstyle---diagram-pictures) `palette`.

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
| `mcp_servers` | list of strings | `["mcptools"]` | Servers from `~/.claude.json` to load in chat turns and in claude and claude-headless workers |
| `skills` | list of paths | `[]` | Skill directories exposed to chat |
| `prompt_file` | path | unset | Text file that replaces the behaviour rules of the chat system prompt: the reply format, ask-first, verify, cite-history, development-roles and similar rules. The tool contract (`suggest_reply`, `help`, `config`), the worker, watch and memory rules, and the forget and logging rules stay. Unset uses the built-in text, which is byte-identical to earlier versions. A file that is empty or only whitespace prints a warning and counts as unset |
| `append_prompt_files` | list of paths | `[]` | Text files appended, in order, after the system prompt and before the older chat lines |
| `project_claude_md` | bool | `true` | `false` adds `CLAUDE.md`, `CLAUDE.local.md` and `.claude/CLAUDE.md` of the directory where chat runs to `claudeMdExcludes`; ancestors are not excluded |
| `user_settings` | bool | `false` | `true` loads user settings too (`--setting-sources user,project,local`); it does not change `claudeMdExcludes` |
| `user_claude_md` | bool | `false` | `true` stops excluding `~/.claude/CLAUDE.md` |
| `recent_history_tokens` | integer | `8192` | Token budget for the newest chat messages shown verbatim in the memory view; `0` turns it off |
| `worker_check_secs` | integer | `60` | Seconds between worker checks while chat is open; `0` turns the check off |
| `keepalive_hours` | integer | `4` | Hours after the last real turn during which an idle chat pings claude every 55 min to keep the 1 h prompt cache warm; `0` turns it off ([Keepalive](context/ralph-chat.md#keepalive)) |
| `worker_quiet_secs` | integer | `300` | Seconds a running Cursor, opencode or Codex worker may send no activity (ACP traffic, session SSE events or Codex app-server frames) before one inbox alert; must be above 0, else a warning and `300`. The 300 s default is measured for Cursor and opencode, not for Codex: silences outside tool calls reached 132 s, and a tool call is silent for its whole run, so a tool call over 300 s alerts once ([Quiet periods](context/headless-workers.md#quiet-periods-2026-10-22)). Runs on the worker check, so `worker_check_secs = 0` turns it off too |
| `reset_turns` | integer | `30` | Turns one `claude` process serves before chat drops it and respawns from the store; must be above 0, else a warning and `30` |
| `reset_tokens` | integer | `140000` | Context tokens of the last assistant message at which chat drops the process and respawns from the store; must be above 0, else a warning and `140000` |
| `watch_timeout_secs` | integer | `3600` | Seconds before a watch is killed and reports that it timed out; applies to every watch without its own `timeout_secs`; `0` is an error |
| `tag_entries` | bool | `true` | `false` turns off the background pass that tags new chat messages `rule`, `decision` or `approval` with Haiku after each reply ([Tagging](context/ralph-chat.md#tagging)) |
| `wrap_width` | integer | unset | Column cap for chat replies and the prompt area; `0` or unset wraps at the terminal width |
| `cell_output_limit` | integer | `200` | Lines of cell output sent to Claude, per stream (stdout, stderr): first half and last half kept; `0` sends all |
| `cell_output_bytes` | integer | `65536` | Bytes of cell output sent to Claude, per stream (stdout, stderr): head and tail kept; `0` sends all |
| `worker_base` | string | unset | Base branch that `close_worker` checks for unmerged commits |
| `worktree_dir` | string | `.worktrees` | Directory that holds worker worktrees; a leading `~` expands to home, relative paths resolve against the repo root |
| `permission_mode` | string | `"auto"` | `claude --permission-mode` for chat turns and claude workers |
| `inbox_alert_sound` | bool | `true` | Play a sound when a worker report arrives and the chat is not focused. Skipped, with the notification, when Rex shows the report as a question through `terminal_status` |
| `inbox_alert_notification` | bool | `false` | Also send a system notification (`<id> <status>`) |
| `inbox_alert_unknown_focus` | `"alert"` or `"silent"` | `"alert"` | What to do when focus cannot be detected (outside tmux, no attached client, tmux query failed) |
| `status_powerline` | bool | `false` | Use powerline glyphs (`\ue0b0`, `\ue0b2`) between status line segments; needs a patched font. Off draws plain colour blocks |
| `auto_hold` | bool | `true` | When a turn ends with a suggested reply of exactly `/hold <target>` for a listed worker, watch, or schedule, enter that hold at once instead of pre-filling the prompt; `false` always pre-fills |
| `auto_continue` | bool | `true` | When a worker's turn ends on an API error (activity marker `failed`) and it has not reported since the last go-ahead, send it one continue message; if it is idle again with no report after that, post one `check` item. Runs on the worker check tick, so `worker_check_secs = 0` turns it off. See [ralph chat](context/ralph-chat.md#auto-continue) |
| `hold_progress` | bool | `true` | During `/hold`, show one dim progress line per held worker above the status bar (id, `held <time>`, activity, rebase step, commits, last pane line), refreshed every 30 seconds; `false` shows only the bar. See [/hold](context/ralph-chat.md#hold) |
| `resume_summary` | bool | `true` | Print a short recap of where the chat log ended before the first prompt (last message, Next step, live workers and unpushed commits); no model call. `false` turns it off; `chat --skip-summary` skips it for one run. See [startup recap](context/ralph-chat.md#startup-recap) |
| `send_it_max_turns` | integer | `15` | Default turn cap of a `/send-it` run plan that sets none; the run stops when it reaches the cap. `0` becomes `15` with a warning. See [ralph chat](context/ralph-chat.md#send-it) |
| `send_it_max_usd` | float | `5.0` | Default cost cap in USD of a `/send-it` run plan that sets none, summed from the cost of each run turn; the run stops when it reaches the cap. A value that is not finite or is `0` or less becomes `5.0` with a warning |
| `terminal_status` | bool | `true` | Report chat state to the terminal with OSC 7501 (Program Status Protocol): working, blocked, done, idle, cleared on exit. Sent only to Rex: stdout is a terminal and `REX_SESSION` is set; `NO_COLOR` does not affect it. See [ralph chat](context/ralph-chat.md#terminal-status) |
| `name` | string | unset | Session name shown in the status bar and the terminal title; `--name` overrides it. Unset uses the git root folder name, then the cwd folder name, then `ralph`. At most 32 characters, control characters removed. A label only: it does not separate stores. See [ralph chat](context/ralph-chat.md#session-name) |
| `terminal_title` | bool | `true` | Set the terminal title to `ralph · <name>` while the chat runs and restore the old title on exit. Sent only when stdout is a terminal; `NO_COLOR` does not affect it. tmux needs `set-titles on`. See [ralph chat](context/ralph-chat.md#terminal-title) |
| `editor` | list of strings | unset | Ctrl-G editor argv, draft path appended, e.g. `["nvim", "-c", "normal! Go", "-c", "startinsert"]`; unset uses `$VISUAL`, `$EDITOR`, then `vi`; empty or wrong type warns and falls back |
| `edit_mode` | `"emacs"` or `"vi"` | `"emacs"` | Key bindings of the chat prompt; another value prints a warning and uses `"emacs"`. See [ralph chat](context/ralph-chat.md) |
| `empty_submit` | `"inbox"`, `"inbox-first"` or `"phrase"` | `"inbox"` | What Enter on an empty prompt does. `inbox`: open the inbox when reports are unread, else nothing. `inbox-first`: open the inbox when reports are unread, else send `empty_prompt`. `phrase`: always send `empty_prompt`, even with unread reports; use `/inbox` to read them. An unknown value warns and uses `inbox` |
| `empty_prompt` | string | unset | Phrase sent by `inbox-first` and `phrase`, e.g. `"continue"`. Unset, empty or whitespace-only warns and the mode behaves like `inbox` |

`--skill` and `RALPH_CHAT_SKILLS` replace `skills`. `prompt_file` and `append_prompt_files` resolve a relative path against the directory of the config file that sets it (absolute paths and a leading `~` are kept; `~` expands to home), and a store value replaces the global one (a list replaces, it does not merge). Chat reads the files once at start, so a change needs a chat restart. A file that is missing or unreadable prints a warning and is skipped; if `prompt_file` fails, the built-in text applies. `ralph config check` reports each missing or unreadable file as an error. The five keys can be set in the global or the store file. For `permission_mode` the precedence is `chat --permission-mode`, then `RALPH_CHAT_PERMISSION_MODE`, then this key, then `auto`; chat passes the resolved mode to every claude worker it spawns, so chat and workers always match. `--recent-history-tokens <TOKENS>` overrides `recent_history_tokens`; see [recent history](context/ralph-chat.md#recent-history). Without `worker_base`, `close_worker` uses `origin/HEAD`, then `main`. For `worktree_dir` the precedence is `chat --worktree-dir`, then `RALPH_WORKTREE_DIR`, then this key, then `.worktrees`. A worker that already has a worktree keeps it: spawn and close look up `worker/<ID>` in `git worktree list` first, so changing the key does not lose running workers. A directory inside the repo is added to `.git/info/exclude`. A config that fails to read or parse skips `worktree_dir`, and the spawn or close result ends with a warning. For `wrap_width` the precedence is `chat --wrap-width <N>`, then `RALPH_WRAP_WIDTH`, then this key; a value that is not a whole number stops chat start with an error. The width used is the smaller of this cap and the terminal width, read at each render, so a narrower terminal still wins. The cap is read once at chat start: a change to this key or `RALPH_WRAP_WIDTH` needs a chat restart; a terminal resize does not. It applies to rendered prose (chat turns and drained worker reports) and to the reedline prompt area: the status line, the draft, and the footer. Code blocks are not wrapped. For `cell_output_limit` and `cell_output_bytes` the precedence is `RALPH_CELL_OUTPUT_LIMIT` / `RALPH_CELL_OUTPUT_BYTES`, then the key, then the default; a value that is not a whole number stops chat start with an error. The caps shape only what Claude receives; the terminal shows all output in `result` fences. See [executable cells](context/ralph-chat.md#executable-cells). `worker_check_secs` has no flag or env var; the check is described in [ralph chat](context/ralph-chat.md#worker-inbox). `chat --no-inbox-sound` and `--no-inbox-notification` turn off the matching alert for one run; focus rules are in [ralph chat](context/ralph-chat.md#worker-inbox). See [ralph chat](context/ralph-chat.md).

### `[cursor]` - Cursor Workers

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `permissions` | `"ask"` or `"yolo"` | `"ask"` | Both run `agent -p --force` per turn. `"ask"` asks the person before each shell command through the hook bridge (see [Headless workers](context/headless-workers.md#cursor-print-mode)); `"yolo"` asks nothing |
| `model` | string | unset (cursor's own default) | Model id for Cursor workers that `spawn_worker` starts without a `model`. Passed as `agent -p --model`; unset passes `auto` |

`ask` gates only shell commands. MCP tools run without asking in both modes (`--approve-mcps`). A shell ask with no answer in 120 s is denied. Cursor's own rules (deny lists, sandbox) still apply. Risk under `yolo`: shell commands run without asking. A wrong value or type, or a config that fails to read, warns and uses `"ask"`. The key is read when a Cursor worker starts.

A `model` passed to `spawn_worker` wins over this key. A value that is empty or has whitespace warns in the spawn result and uses cursor's default. A config that fails to load warns and uses cursor's default. The key is read at each spawn. A new worker runs `agent -p --model <value>`, so the user's Cursor default is not changed. A worker resumed from an old ACP session record (no `permissions`) still runs `agent acp`: after `session/load` ralph sets the session `model` option from the value Cursor advertises (`worker::select_cursor_model`), snapshots the user's model keys to `cursor-model-default.json` next to `config.toml` and restores them in `~/.cursor/cli-config.json` and `~/.cursor/acp-config.json` after the set. The next `ralph chat` start puts a leftover snapshot back.

### `[codex]` - Codex Workers

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `permissions` | `"ask"` or `"yolo"` | `"ask"` | `"yolo"` answers `accept` at once to Codex command, file-change and MCP tool-call approvals when `accept` is offered; `"ask"` waits for the person. Session or persistent grants, amendments and permission-grant requests are never auto-answered. Read when a worker starts; a wrong value warns and uses `"ask"` |
| `model` | string | unset (Codex's own default) | Model for Codex workers that `spawn_worker` starts without a `model`. A `model` passed to `spawn_worker` wins. Checked against `model/list` at spawn; an unknown model fails the spawn. A blank value or a value with whitespace warns in the spawn result and uses Codex's default. A config that fails to load adds a warning to the spawn result and codex uses its own defaults |
| `effort` | string | unset (Codex's own setting) | Reasoning effort for Codex workers. Checked against the effective model's `supportedReasoningEfforts` at spawn; an unsupported value fails the spawn and lists the supported ones. There is no `spawn_worker` effort parameter. Read when a worker starts |

Codex setup and limits:

- Version: codex CLI 0.162 or newer, probed with 0.164.0. Older fails the spawn. From 0.165 on, the spawn result warns `codex X is newer than the probed 0.164.0; the app-server protocol may differ.` and goes on.
- Login: ralph never manages Codex keys or env. Run `codex login` yourself. A spawn without login fails with `codex is not logged in; run `codex login``.
- Interrupt: `reply` with `interrupt` is partial. If chat gets no confirmation within 30 s, the outcome is unknown.
- Permissions: only requests that Codex raises are handled (`ask`), and `yolo` accepts one request at a time. Nothing is granted for a session.
- Resume: needs the saved Codex history, the worktree and a Codex login.
- Trust: ralph passes an in-memory `projects."<cwd>".trust_level = "trusted"` override on `thread/start` and `thread/resume`, so Codex writes no trust entry to `~/.codex/config.toml` (GUZ-847).
- Quiet: Codex frames count as activity, not progress. A tool that is silent for longer than `worker_quiet_secs` gives a false Quiet alert.
- Resume failure: a worker that cannot resume (for example, its history is gone) is not resumed. Chat warns `codex session <id> not resumed: ` followed by the start error, writes `codex-error/<id>` and keeps the session record.

### `[claude]` - Claude Workers

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `model` | string | unset (claude's own default) | Model name or alias passed as `--model` to Claude workers that `spawn_worker` starts without a `model` |
| `shell` | `zsh` or `bash` | `zsh` | Login shell that starts Claude workers (`<shell> -lic`) and the spawn preflight checks `claude` in; a wrong value warns and uses `zsh`, and `ralph config check` reports it as an error. A store config can set it per store |
| `config_dir` | path | unset (claude uses `~/.claude`) | Folder ralph passes as `CLAUDE_CONFIG_DIR` to every `claude` it starts: chat turns, the memory nap, `ralph ask` and Claude workers. Relative paths are read from the config file's folder; `~` expands to the home directory. A store config can set it per store; `ralph ask` has no `--store`, so only the global key applies to it. `ralph config check` notes a folder that does not exist |
| `permissions` | `"ask"` or `"yolo"` | `"ask"` | `claude-headless` workers only. `"ask"` spawns claude with `--permission-mode default`, `"yolo"` with `auto`; the worker's own `report` and `set_status` tools never ask. `"yolo"` allows every `can_use_tool` request at once, for that request only (no `updatedPermissions`); `"ask"` waits for the person, but allow rules in the project or user `.claude/settings.json` (under `config_dir` when set) still skip the prompt, because claude sends no `can_use_tool` for them. Read when a worker spawns; a resumed worker keeps the mode it spawned with; a wrong value warns and uses `"ask"` |

`config_dir` picks which Claude login and subscription ralph's `claude` processes use. It also moves the files ralph touches in that folder: the `CLAUDE.md` chat excludes, the `projects/` transcripts and `.claude.json` trust entry that closing a worker removes, and the `.claude.json` that `[chat] mcp_servers` imports from, for chat and for claude and claude-headless workers. With `config_dir` unset, a `CLAUDE_CONFIG_DIR` in ralph's own environment reaches the `claude` it starts, so claude-headless usage rows and worker close follow that folder instead of `~/.claude` (see [Worker leftovers](context/worker-leftovers.md#cleanup-by-worker-kind)). claude does not resolve the value or expand `~`, so ralph reads a relative value from the worktree, claude's cwd. An empty value puts the transcripts in the worktree and keeps `~/.claude.json`. Changing the key in the middle of a chat means `--resume` cannot find the old session. Cursor, codex and opencode workers are not affected.

A `model` passed to `spawn_worker` wins over this key. A value that is empty or has whitespace warns in the spawn result and uses claude's default. A config that fails to load warns and uses claude's default. The key is read at each spawn. A config that fails to load warns in the spawn result and uses `zsh`. A failed preflight error also carries that warning.

`claude-headless` workers: `reply` with `interrupt` is partial. It denies pending permission requests, then sends a `control_request` interrupt. The reply says the turn was cancelled, that nothing was running, or that the worker exited. Only when claude ignores the interrupt for 30 s is the outcome unknown. Resume is partial: a restarted chat resumes each worker with `claude --resume` in its worktree, but a turn running when the chat exited is lost and reported as `blocked`; a moved or deleted worktree is not resumed.

A bash login shell reads `~/.bash_profile` (or `~/.profile`), not `~/.bashrc`. Set the PATH for `claude` there, or make `~/.bash_profile` source `~/.bashrc`.

### `[opencode]` - opencode Workers

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `model` | string | `"openai/gpt-6.1-sol"` | `provider/model` for opencode workers that `spawn_worker` starts without a `model` |

A `model` passed to `spawn_worker` wins over this key. A value that is not `provider/model` warns in the spawn result and uses the built-in default. A config that fails to load warns in the spawn result and uses the built-in default. The key is read at each spawn.

### `[tickets]` - Ticket Keys

| Key | Default | Meaning |
| --- | --- | --- |
| `linear` | `[]` | Team keys of Linear tickets, such as `["GUZ"]` |
| `jira` | `[]` | Project keys of Jira tickets, such as `["VP20", "KODA"]` |

```toml
[tickets]
linear = ["GUZ"]
jira = ["VP20", "KODA"]
```

`spawn_worker` records a ticket for each worker. The `ticket` argument wins. Without it, a worker id that is exactly `<KEY>-<digits>` for a key above (case-sensitive) is the ticket, and any other id has none. The tracker is the list that holds the key, Linear first. A `ticket` argument that matches no key is kept with no tracker. `/workers <id>` shows it as `ticket  GUZ-918 (linear)`. The key is read at each spawn. A config that fails to load gives no keys, so only an explicit ticket is recorded, with no tracker.

### `[workers]` - `/workers` remote lookups

`/workers` shows each worker's ticket state and PR from background lookups. See [ralph chat](context/ralph-chat.md#workers).

| Key | Default | Meaning |
| --- | --- | --- |
| `view` | `"table"` | `table` shows a table when it fits and worker blocks otherwise. `list` always shows worker blocks |
| `pr_provider` | `"github"` | `github` looks up the PR with `gh`. `none` turns PR lookups off and the PR cell shows `-` |
| `lookup_timeout_ms` | `5000` | Kill one lookup, with its child processes, after this many milliseconds. The cell shows `?`. Minimum `1` |
| `cache_secs` | `60` | Show a result for this many seconds, failures too. After that the cell shows `…` until a new lookup ends. Minimum `1` |
| `verify_on_drift` | `true` | When a spawn sees an installed harness version that is newer than the probed one and not verified, run the free `ralph worker verify <harness>` in a background process (90 s verify budget, ended after 120 s at most, never `--live`). A pass is recorded quietly. A fail posts one inbox alert per harness and version. While `ralph chat` runs, the same run also starts from a once-per-24-h check of the installed claude, opencode and cursor versions (codex is skipped). `false` turns the background run and the daily check off. See [Version drift](context/headless-workers.md#version-drift) |

```toml
[workers]
pr_provider = "none"
lookup_timeout_ms = 3000
```

A `0` for `cache_secs` or `lookup_timeout_ms`, or an unknown `pr_provider` or `view`, fails config load. An unknown key is ignored. `/workers` reloads the file on each call, so an edit applies to the next `/workers`; a file that fails to load uses the defaults there. Ticket lookups need a tracker, so they follow `[tickets]`.

### `[mermaid]` - Diagram Rendering

In a terminal, a fenced `mermaid` code block renders as a Unicode text diagram for flowchart/graph, sequenceDiagram, erDiagram, classDiagram, stateDiagram, and stateDiagram-v2. Rendering runs in a child process (`ralph __mermaid`) that ralph kills on timeout or memory overrun. Any refusal, kill, error, or output wider than the terminal shows the highlighted source with no error text. Piped output keeps the raw source.

mermaid-text can drop a transition label on an edge into a composite state.

| Key | Default | Meaning |
| --- | --- | --- |
| `timeout_ms` | `500` | Kill the render child after this many milliseconds |
| `max_rss_mb` | `256` | Kill the child above this resident size (macOS and Linux only; elsewhere only the timeout applies) |
| `max_input_bytes` | `1048576` | Refuse a block with more source bytes |
| `max_flowchart_edges` | `50` | Refuse a flowchart/graph, stateDiagram, or stateDiagram-v2 with more arrow tokens |
| `max_class_relations` | `400` | Refuse a classDiagram with more relation tokens (provisional: dense graphs among few classes can exceed the timeout well below 400; the timeout still bounds them) |
| `picture_timeout_ms` | `3000` | Kill a `/view` picture child after this many milliseconds; the block then draws as text |

Fixed limits: flowchart, stateDiagram, and stateDiagram-v2 source up to 16 KB; sequenceDiagram up to 20 participants, 64 KB, and 2000 statements; erDiagram up to 64 KB and 2000 statements. A config that fails to load prints a warning and uses these defaults.

### `[mermaid.style]` - Diagram pictures

`/view` draws mermaid blocks as pictures (see [ralph chat](context/ralph-chat.md#view)). This table styles them. Every key is optional; the store `config.toml` overrides the global one per key. A bad value warns at chat start and in `ralph config check`, and uses the default.

| Key | Default | Meaning |
| --- | --- | --- |
| `palette` | `"ui"` | `ui` takes colors from `[theme.ui]` on a fixed dark ground (`#1e2127`, text `#abb2bf`), with node fills and borders tinted by `spinner`; `code` takes them from the agent code theme (background, foreground, scope colors; a theme with no colors uses `dark`); `light` and `dark` are fixed. Any other value warns and uses `ui` |
| `background` | `"theme"` | `theme` fills with the palette `bg`, `transparent` fills nothing, or a `#rrggbb` color |
| `series` | palette | Up to 8 `#rrggbb` colors for chart series and pie slices, in order; missing ones keep the palette |
| `font_family` | `["Arial", "Liberation Sans", "Helvetica Neue", "Helvetica"]` | Label fonts; the first installed one draws. Layout always uses Arial metrics, so Arial-metric fonts come first |
| `font_family_mono` | `["SF Mono", "Menlo", "DejaVu Sans Mono"]` | Monospace fonts |
| `font_size` | `16.0` | Layout font size in pixels; above 0 |
| `padding` | `24.0` | Padding around the diagram in pixels; 0 or more |
| `curve` | `"rounded"` | Flowchart edges: `rounded`, `linear` or `step` |
| `auto_direction` | `true` | Let a wide top-down flowchart switch to left-right |
| `min_text_ratio` | `0.6` | Draw text instead when picture text would be smaller than this share of the cell height; 0 to 1 |

`[mermaid.style.colors]` overrides single palette colors with `#rrggbb`: `bg`, `fg`, `muted`, `line`, `surface`, `surface_alt`, `border`, `accent`, `critical`. An unknown key warns.

`[mermaid.style.config]` passes a bezel-mermaid site config (mermaid config keys, such as `[mermaid.style.config.flowchart] nodeSpacing = 30`). It wins over `curve`; a diagram's own init block or frontmatter wins over both.

```toml
[mermaid.style]
palette = "code"
curve = "linear"

[mermaid.style.colors]
accent = "#d08770"
```

### `[changes]` - `/diff` pager and turn snapshots

`/diff` in chat runs `git diff`. See [ralph chat](context/ralph-chat.md#diff). Chat also snapshots the repo around each turn. See [ralph chat](context/ralph-chat.md#turn-snapshots).

| Key | Default | Meaning |
| --- | --- | --- |
| `pager` | `"git"` | `git`, `delta`, or `hunk`. An unknown string warns once at chat start and uses `git`; the config still loads |
| `diff_args` | `[]` | `git diff` arguments placed before the ones typed after `/diff` |
| `enabled` | `true` | Snapshot the chat's repo at the start and end of each turn. `false` turns it off |

`delta` and `hunk` are one-off `git -c core.pager=...` (`hunk` passes `core.pager=hunk pager` as one argument). ralph sets no environment variables and does not change `core.pager` in git config. With `pager = "git"`, git uses the user's pager.

### `[wire.prices]` - `ralph wire ls` price overrides

`ralph wire ls` prices calls from a built-in dated table. The same overrides also drive `/workers costs` in `ralph chat`, where `gpt` model keys are allowed and win over the built-in OpenAI rows. A partial `gpt` override fills its missing prices from Anthropic ratios. An override can set `cache_write_30m` and `cache_write_12h`. Without it, the 30m price comes from the provider multiplier when the provider has one. Worker costs use Anthropic ratios, which have no 30m multiplier, so a `gpt` override without `cache_write_30m` leaves codex cache writes out and the USD ends in `+`. See [/workers costs](context/ralph-chat.md#workers-costs). A `[wire.prices."<model prefix>"]` table overrides it for that exact model id, or the same id plus an 8-digit date (`-20261021`). Longest prefix wins. Any `[...]` suffix on the recorded model is ignored. An override applies at every prompt size. A negative price fails config load, so `ralph wire ls` fails. A config that fails to parse fails the same way. See [ralph wire](context/ralph-wire.md#prices).

```toml
[wire.prices."claude-haiku-5-5"]
input = 0.1
output = 0.5
```

| Key | Default | Meaning |
| --- | --- | --- |
| `input` | required | USD per million uncached input tokens |
| `cache_read` | `input` x provider multiplier | USD per million cache-read tokens |
| `cache_write_5m` | `input` x provider multiplier | USD per million 5-minute cache-write tokens |
| `cache_write_1h` | `input` x provider multiplier | USD per million 1-hour cache-write tokens |
| `cache_write_30m` | `input` x provider multiplier, when the provider has a 30-minute tier | USD per million 30-minute cache-write tokens |
| `cache_write_12h` | `input` x provider multiplier, when the provider has a 12-hour tier | USD per million 12-hour cache-write tokens |
| `output` | `input` x provider multiplier | USD per million output tokens |

Unknown keys are an error. Speed and geography factors from the recorded usage still apply on top.

## Chat Store

`ralph ask` stores only a usage row, not the prompt or reply. `ralph chat` keeps its store at `~/.ralph/chat`. For another path the precedence is `chat --store <DIR>`, then `RALPH_CHAT_STORE`, then `~/.ralph/chat`; a relative path resolves against the cwd at startup. Parallel chats need separate stores. See [ralph chat](context/ralph-chat.md).

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
