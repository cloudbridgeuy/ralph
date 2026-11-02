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
| `worker` | Verify an installed worker agent against ralph's local spec ([details](context/headless-workers.md#verify-an-agent-version)) |
| `upgrade` | Install the latest GitHub release over the running binary ([details](context/release-process.md#upgrade-an-install)) |

`ralph chat --name <NAME>` sets the chat session name shown in the status bar and the terminal title. It overrides `[chat] name`; without both, the name is the git root or cwd folder name. The name is only a label: chats that share a store still share memory and workers, so use `--store` to separate them ([details](context/ralph-chat.md#session-name)).

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
ralph wire [--provider anthropic|openai] [--port N] [--upstream URL] [--dir PATH] [--responses]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--provider` | `anthropic` | Provider to proxy: `anthropic` or `openai` |
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

`config show` prints the merged config, one `key = value  [source]` line per value, where source is `default`, `global` or `store`. A store value that hides a global one adds it, as `key = value  [store; global = file value]`. When an environment variable for that key is set and not empty, the line shows its value instead, then every file value it hides, as `key = "value"  [env NAME; store = file value, global = file value]`, and such a key with no file value gets its own `[env NAME]` line. Flags do not apply to `config show`; in a chat, `/config` also shows the start flags and every source path (see [ralph chat](context/ralph-chat.md#config)).

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

## ralph worker

```bash
ralph worker verify <cursor|codex|claude-headless|opencode|claude> [--live]
```

Runs the installed worker agent in a fresh temporary directory and prints `PASS`, `FAIL` or `UNKNOWN` per check, then a summary line that says whether `--live` ran. For `claude-headless`, `claude` and `opencode` the summary also names the model, chosen from `[claude] model` or `[opencode] model` as `ralph worker spawn` does (claude's default model, or the opencode default, when unset). Exit code is 0 when every blocking check passes and 1 otherwise. A full pass records the agent version in `verified-agents.json` in the config directory (honours `RALPH_CONFIG_DIR`), which stops the "differs from the tested" warning for that version.

Without `--live` no prompt is sent and no model is called. `--live` adds one reply, one permission round-trip and one cancel, which use model calls. A failed or unknown cancel is printed but never fails the spec. See [Headless workers](context/headless-workers.md#verify-an-agent-version) for the checks.

---

## ralph upgrade

```bash
ralph upgrade [--check] [--force]
```

Installs the latest published GitHub release of ralph over the running binary. It calls the GitHub API without authentication.

| Flag | Effect |
|------|--------|
| `--check` | Print the installed and latest versions and what `ralph upgrade` would do; download nothing |
| `--force` | Replace a dev build (`-dev` in `ralph --version`), even with an older release |

Exit code is 1 when GitHub is unreachable, the directory of the binary is not writable, the checksum does not match, or a dev build is refused. Already on the latest release exits 0. See [Upgrade an install](context/release-process.md#upgrade-an-install) for the steps.

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
