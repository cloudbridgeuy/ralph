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

## Exit Codes

| Code | Meaning |
|------|---------|
| `0` | Success - all operations completed successfully |
| `1` | Error - operation failed (see error message for details) |

---

## See Also

- [Configuration](configuration.md) - Detailed configuration file documentation
- [ralph themes](#ralph-themes) - List available syntax highlighting themes
