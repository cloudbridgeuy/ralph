# CLI Reference

Complete reference for all ralph commands and options.

## Global Options

```
ralph [COMMAND]
```

The command is optional: bare `ralph` runs `ralph chat`.

| Option | Description |
|--------|-------------|
| `-V`, `--version` | Print version information (see [version scheme](context/release-process.md#version-scheme)) |
| `-h`, `--help` | Print help (`-h` shows a summary) |

## Commands

| Command | Description |
|---------|-------------|
| `themes` | List available syntax highlighting themes |
| `ask` | Send a single-shot prompt to the LLM ([details](context/ralph-ask.md)) |
| `chat` | Chat with a persistent, compressed memory; default when no command is given ([details](context/ralph-chat.md)) |
| `wire` | Record harness-to-provider traffic through a local proxy ([details](context/ralph-wire.md)) |
| `config` | Check `config.toml` for errors and unknown keys |

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
| `RALPH_THEME` | `--theme` | Set syntax highlighting theme |
| `RALPH_NO_BACKGROUND` | `--no-background` | Disable theme backgrounds (set to `1`) |
| `RALPH_ASK_PERMISSION_MODE` | `ask --permission-mode` | Permission mode for `ask` |
| `RALPH_CHAT_PERMISSION_MODE` | `chat --permission-mode` | Permission mode for chat turns and claude workers |
| `RALPH_WORKTREE_DIR` | `chat --worktree-dir` | Directory for worker worktrees |
| `RALPH_CHAT_SKILLS` | `chat --skill` | `:`-separated skill directories for chat |
| `RALPH_WRAP_WIDTH` | `chat --wrap-width` | Column cap for chat replies and the prompt area (also `[chat] wrap_width`); `0` = terminal width |

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

## ralph wire

Run a local recording proxy, or inspect recorded sessions with `ls`, `show`, `diff`, and `sessions`. Details: [ralph wire](context/ralph-wire.md).

```bash
ralph wire [--provider anthropic] [--port N] [--upstream URL] [--dir PATH] [--responses]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--provider` | `anthropic` | Provider to proxy |
| `--port` | `0` | Port on 127.0.0.1; `0` picks a free port |
| `--upstream` | provider default | Upstream base URL |
| `--dir` | `<tmp>/ralph-wire` | Directory holding session folders |
| `--responses` | off | Also store decoded SSE events per call |

```bash
ralph wire ls [--session ID] [--dir PATH] [--all] [--json]
ralph wire show N [--session ID] [--dir PATH] [--json]
ralph wire diff A [B] [--session ID] [--dir PATH] [--json]
ralph wire sessions [--dir PATH] [--json]
```

| Flag | Applies to | Description |
|------|------------|-------------|
| `--session` | `ls`, `show`, `diff` | Session id (default: the latest) |
| `--dir` | `ls`, `show`, `diff`, `sessions` | Directory holding session folders (default `<tmp>/ralph-wire`) |
| `--all` | `ls` | Also show non-inference calls |
| `N` | `show` | Call number (`seq`) from `ls` |
| `A`, `B` | `diff` | Call numbers (`seq`); with only `A`, diff `A` against the earlier call sharing its longest prompt prefix |
| `--json` | `ls`, `show`, `diff`, `sessions` | Print JSON instead of a table |

---

## ralph config

```bash
ralph config check [--path FILE] [--store DIR]
ralph config show [--store DIR]
```

Parses the config file and reports every problem on stderr. Without `--path` it checks the default `config.toml` (honours `RALPH_CONFIG_DIR`); a missing default file prints `no config at <path>; defaults apply` and exits 0. A missing `--path` file is an error.

With `--store DIR` it also checks `DIR/config.toml` on its own, then the merged result of both files for errors only the combination causes. Each line from a file is prefixed with that file's path. A missing store file is fine.

`config show` prints the merged config, one `key = value  [source]` line per value, where source is `default`, `global` or `store`; `env NAME is set` is added when an environment variable for that key is set. Flags do not apply to `config show`.

| Line | Meaning |
|------|---------|
| `unknown key: chat.wrap_widh` | Key that no setting uses; chat ignores it |
| `[chat] edit_mode = "nano" is not "emacs" or "vi"; using "emacs"` | Invalid value that chat warns about and replaces with a default |
| `watch_timeout_secs must be above 0.` | Value chat rejects at startup |
| `cannot read prompt file <path>: <error>` | A `[chat] prompt_file` or `append_prompt_files` path (after relative paths resolve against the file) that is missing or unreadable; chat skips it with a warning |
| `note: ...` | Deprecation; does not fail the check |
| `ok: <path>` | No errors |

Parse failures and invalid `[workers]` or `[wire.prices]` values also fail the check. Exit code is 1 when any error is found. Environment variables and `$EDITOR` are not consulted.

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
