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

Run a local recording proxy, or inspect recorded sessions with `ls`, `show`, and `sessions`. Details: [ralph wire](context/ralph-wire.md).

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
ralph wire sessions [--dir PATH] [--json]
```

| Flag | Applies to | Description |
|------|------------|-------------|
| `--session` | `ls`, `show` | Session id (default: the latest) |
| `--dir` | `ls`, `show`, `sessions` | Directory holding session folders (default `<tmp>/ralph-wire`) |
| `--all` | `ls` | Also show non-inference calls |
| `N` | `show` | Call number (`seq`) from `ls` |
| `--json` | `ls`, `show`, `sessions` | Print JSON instead of a table |

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
