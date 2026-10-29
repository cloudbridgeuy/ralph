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

What a store file can change: it can override any key. Ralph warns once per key when a store file sets one of these security-relevant keys: `chat.permission_mode`, `chat.mcp_servers`, `chat.user_settings`, `chat.prompt_file`, `chat.append_prompt_files`, `chat.skills`, `codex.permissions` and `cursor.permissions`. If the store file fails to parse or validate, ralph warns and names the file, then uses the global file alone. `ralph config show` and `ralph config check` still report that failure as an error.

`ralph config show [--store DIR]` prints the merged config and the source of each value (default, global or store); a value whose environment variable is set is marked. Flags do not apply to it. `ralph config check --store DIR` checks both files and the merged result (see [CLI reference](cli-reference.md#ralph-config)).

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
store_fg = "reset"        # store path text
workers_fg = "reset"      # workers text
watches_fg = "reset"      # watches and scheduled-task text
base_bg = "#303030"       # bar background: filler, store, workers, watches
```

The default `base_bg` is ANSI color 236, written `#303030` above. `reset` is the terminal's own text color. Powerline glyphs take their colors from the neighbouring blocks, so they have no keys.

#### Input colors: `[theme.input]`

`ralph chat` draws the `suggest_reply` pre-fill in its own color, so you can tell it from your own draft. The text goes back to the normal color on the first edit and stays normal, even if you retype the same words. Held and queued text keeps the normal color. The key is optional and takes the same color values as the role themes. `ralph chat` warns at start for an unknown key or a bad color and keeps the default for it.

```toml
[theme.input]
suggestion = "cyan"  # suggest_reply pre-fill text
```

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
| `editor` | list of strings | unset | Ctrl-G editor argv, draft path appended, e.g. `["nvim", "-c", "normal! Go", "-c", "startinsert"]`; unset uses `$VISUAL`, `$EDITOR`, then `vi`; empty or wrong type warns and falls back |
| `edit_mode` | `"emacs"` or `"vi"` | `"emacs"` | Key bindings of the chat prompt; another value prints a warning and uses `"emacs"`. See [ralph chat](context/ralph-chat.md) |
| `empty_submit` | `"inbox"`, `"inbox-first"` or `"phrase"` | `"inbox"` | What Enter on an empty prompt does. `inbox`: open the inbox when reports are unread, else nothing. `inbox-first`: open the inbox when reports are unread, else send `empty_prompt`. `phrase`: always send `empty_prompt`, even with unread reports; use `/inbox` to read them. An unknown value warns and uses `inbox` |
| `empty_prompt` | string | unset | Phrase sent by `inbox-first` and `phrase`, e.g. `"continue"`. Unset, empty or whitespace-only warns and the mode behaves like `inbox` |

`--skill` and `RALPH_CHAT_SKILLS` replace `skills`. `prompt_file` and `append_prompt_files` resolve a relative path against the directory of the config file that sets it (absolute paths and a leading `~` are kept; `~` expands to home), and a store value replaces the global one (a list replaces, it does not merge). Chat reads the files once at start, so a change needs a chat restart. A file that is missing or unreadable prints a warning and is skipped; if `prompt_file` fails, the built-in text applies. `ralph config check` reports each missing or unreadable file as an error. The five keys can be set in the global or the store file. For `permission_mode` the precedence is `chat --permission-mode`, then `RALPH_CHAT_PERMISSION_MODE`, then this key, then `auto`; chat passes the resolved mode to every claude worker it spawns, so chat and workers always match. `--recent-history-tokens <TOKENS>` overrides `recent_history_tokens`; see [recent history](context/ralph-chat.md#recent-history). Without `worker_base`, `close_worker` uses `origin/HEAD`, then `main`. For `worktree_dir` the precedence is `chat --worktree-dir`, then `RALPH_WORKTREE_DIR`, then this key, then `.worktrees`. A worker that already has a worktree keeps it: spawn and close look up `worker/<ID>` in `git worktree list` first, so changing the key does not lose running workers. A directory inside the repo is added to `.git/info/exclude`. A config that fails to read or parse skips `worktree_dir`, and the spawn or close result ends with a warning. For `wrap_width` the precedence is `chat --wrap-width <N>`, then `RALPH_WRAP_WIDTH`, then this key; a value that is not a whole number stops chat start with an error. The width used is the smaller of this cap and the terminal width, read at each render, so a narrower terminal still wins. The cap is read once at chat start: a change to this key or `RALPH_WRAP_WIDTH` needs a chat restart; a terminal resize does not. It applies to rendered prose (chat turns and drained worker reports) and to the reedline prompt area: the status line, the draft, and the footer. Code blocks are not wrapped. For `cell_output_limit` and `cell_output_bytes` the precedence is `RALPH_CELL_OUTPUT_LIMIT` / `RALPH_CELL_OUTPUT_BYTES`, then the key, then the default; a value that is not a whole number stops chat start with an error. The caps shape only what Claude receives; the terminal shows all output in `result` fences. See [executable cells](context/ralph-chat.md#executable-cells). `worker_check_secs` has no flag or env var; the check is described in [ralph chat](context/ralph-chat.md#worker-inbox). `chat --no-inbox-sound` and `--no-inbox-notification` turn off the matching alert for one run; focus rules are in [ralph chat](context/ralph-chat.md#worker-inbox). See [ralph chat](context/ralph-chat.md).

### `[cursor]` - Cursor Workers

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `permissions` | `"ask"` or `"yolo"` | `"ask"` | `"yolo"` answers every Cursor `session/request_permission` with the allow-once option at once; `"ask"` waits for the person, except for the worker `report` tool |
| `model` | string | unset (cursor's own default) | Model id for Cursor workers that `spawn_worker` starts without a `model`. Applied with `session/set_config_option`, not `agent --model`. Cursor's ACP fixes the variant for that model |

`yolo` picks only the allow-once option, never allow-always, and writes nothing to `~/.cursor/cli-config.json`. A request with no allow-once option still waits for the person. Cursor's own rules (deny lists, sandbox) still apply. Risk: shell commands and MCP tools run without asking. A wrong value or type, or a config that fails to read, warns and uses `"ask"`. The key is read when a Cursor worker starts.

A `model` passed to `spawn_worker` wins over this key. A value that is empty or has whitespace warns in the spawn result and uses cursor's default. A config that fails to load warns and uses cursor's default. The key is read at each spawn. The child is `agent acp`. After `session/new`, and again after `session/load` when chat resumes the worker, ralph sets the session `model` option from the value Cursor advertises there (`worker::select_cursor_model`). An exact advertised value is sent as-is. A short name selects the advertised value for that name. Cursor's ACP fixes one variant per model name, so effort and fast mode cannot be chosen: there is no `[cursor] reasoning_effort` or `[cursor] fast`. If a name is listed more than once, the cheapest listed value is sent (`fast=false`, then no `fast`, then `fast=true`, then the lowest effort). Any other string is sent unchanged, and the agent's rejection is the spawn error. `agent --model` does not change the ACP session model. The set rewrites the user's Cursor default. Before the set, ralph writes `cursor-model-default.json` next to `config.toml` in the ralph config directory, holding only the model keys. It then restores `model`, `selectedModel`, `modelParameters`, `hasChangedDefaultModel`, `modelSelectionHistory` and `maxMode` in `~/.cursor/cli-config.json`, and `selectedModelVariantId` in `~/.cursor/acp-config.json`, and deletes the snapshot. A crash between the set and the restore leaves those files on the worker model. The next `ralph chat` start sees the snapshot and puts the saved keys back. With no snapshot, that start cannot tell the switched files from a change the user made. The resolved model is stored on the session record and applied again on resume; a record with no model leaves the agent's current model alone.

### `[codex]` - Codex Workers

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `permissions` | `"ask"` or `"yolo"` | `"ask"` | `"yolo"` answers `accept` at once to Codex command, file-change and MCP tool-call approvals when `accept` is offered; `"ask"` waits for the person. Session or persistent grants, amendments and permission-grant requests are never auto-answered. Read when a worker starts; a wrong value warns and uses `"ask"` |
| `model` | string | unset (Codex's own default) | Model for Codex workers that `spawn_worker` starts without a `model`. A `model` passed to `spawn_worker` wins. Checked against `model/list` at spawn; an unknown model fails the spawn |
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
| `shell` | `zsh` or `bash` | `zsh` | Login shell that starts Claude workers (`<shell> -lic`) and the spawn preflight checks `claude` in; other values fail `ralph config check`. A store config can set it per store |
| `config_dir` | path | unset (claude uses `~/.claude`) | Folder ralph passes as `CLAUDE_CONFIG_DIR` to every `claude` it starts: chat turns, the memory nap, `ralph ask` and Claude workers. Relative paths are read from the config file's folder; `~` expands to the home directory. A store config can set it per store; `ralph ask` has no `--store`, so only the global key applies to it. `ralph config check` notes a folder that does not exist |

`config_dir` picks which Claude login and subscription ralph's `claude` processes use. It also moves the files ralph touches in that folder: the `CLAUDE.md` chat excludes, the `projects/` transcripts and `.claude.json` trust entry that closing a worker removes, and the `.claude.json` that `[chat] mcp_servers` imports from. ralph never reads `CLAUDE_CONFIG_DIR` from its own environment. Changing the key in the middle of a chat means `--resume` cannot find the old session. Cursor, codex and opencode workers are not affected.

A `model` passed to `spawn_worker` wins over this key. A value that is empty or has whitespace warns in the spawn result and uses claude's default. A config that fails to load warns and uses claude's default. The key is read at each spawn. A config that fails to load warns in the spawn result and uses `zsh`. A failed preflight error also carries that warning.

A bash login shell reads `~/.bash_profile` (or `~/.profile`), not `~/.bashrc`. Set the PATH for `claude` there, or make `~/.bash_profile` source `~/.bashrc`.

### `[opencode]` - opencode Workers

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `model` | string | `"openai/gpt-6.1-sol"` | `provider/model` for opencode workers that `spawn_worker` starts without a `model` |

A `model` passed to `spawn_worker` wins over this key. A value that is not `provider/model` warns in the spawn result and uses the built-in default. A config that fails to load uses the built-in default. The key is read at each spawn.

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

Fixed limits: flowchart, stateDiagram, and stateDiagram-v2 source up to 16 KB; sequenceDiagram up to 20 participants, 64 KB, and 2000 statements; erDiagram up to 64 KB and 2000 statements. A config that fails to load prints a warning and uses these defaults.

### `[changes]` - `/diff` pager and turn snapshots

`/diff` in chat runs `git diff`. See [ralph chat](context/ralph-chat.md#diff). Chat also snapshots the repo around each turn. See [ralph chat](context/ralph-chat.md#turn-snapshots).

| Key | Default | Meaning |
| --- | --- | --- |
| `pager` | `"git"` | `git`, `delta`, or `hunk`. An unknown string warns once at chat start and uses `git`; the config still loads |
| `diff_args` | `[]` | `git diff` arguments placed before the ones typed after `/diff` |
| `enabled` | `true` | Snapshot the chat's repo at the start and end of each turn. `false` turns it off |

`delta` and `hunk` are one-off `git -c core.pager=...` (`hunk` passes `core.pager=hunk pager` as one argument). ralph sets no environment variables and does not change `core.pager` in git config. With `pager = "git"`, git uses the user's pager.

### `[wire.prices]` - `ralph wire ls` price overrides

`ralph wire ls` prices calls from a built-in dated table. The same overrides also drive `/workers costs` in `ralph chat`, where `gpt` model keys are allowed and win over the built-in OpenAI rows. A partial `gpt` override fills its missing prices from Anthropic ratios, and it has no 30m write price, so codex cache writes are left out and the USD ends in `+`. See [/workers costs](context/ralph-chat.md#workers-costs). A `[wire.prices."<model prefix>"]` table overrides it for that exact model id, or the same id plus an 8-digit date (`-20261021`). Longest prefix wins. Any `[...]` suffix on the recorded model is ignored. An override applies at every prompt size. A negative price fails config load, so `ralph wire ls` fails. A config that fails to parse fails the same way. See [ralph wire](context/ralph-wire.md#prices).

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
| `output` | `input` x provider multiplier | USD per million output tokens |

Unknown keys are an error. Speed and geography factors from the recorded usage still apply on top.

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
