# ralph

```
┌─────────────────────────────────────────────────────────────┐
│                                                             │
│   ██████╗  █████╗ ██╗     ██████╗ ██╗  ██╗                  │
│   ██╔══██╗██╔══██╗██║     ██╔══██╗██║  ██║                  │
│   ██████╔╝███████║██║     ██████╔╝███████║                  │
│   ██╔══██╗██╔══██║██║     ██╔═══╝ ██╔══██║                  │
│   ██║  ██║██║  ██║███████╗██║     ██║  ██║                  │
│   ╚═╝  ╚═╝╚═╝  ╚═╝╚══════╝╚═╝     ╚═╝  ╚═╝                  │
│                                                             │
│   Claude chat with memory, workers, and rich rendering      │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust: 1.95.0+](https://img.shields.io/badge/Rust-1.95.0+-orange.svg)](https://www.rust-lang.org/)

---

<!-- ANCHOR: intro -->
**ralph** wraps `claude -p` with rich terminal rendering. Bare `ralph` opens one persistent chat whose memory is a compressed tree of summaries. From that chat, Claude can start background workers in their own git worktrees and watch long-running commands. `ralph ask` sends one prompt and stores only a usage row.
<!-- ANCHOR_END: intro -->

## Table of Contents

- [Features](#features)
- [Installation](#installation)
- [Quick Start](#quick-start)
- [Configuration](#configuration)
- [Documentation](#documentation)
- [Contributing](#contributing)
- [License](#license)

<!-- ANCHOR: start -->
## Features

- **Persistent chat** - Bare `ralph` runs `ralph chat`. One Claude process serves the chat, started with a compressed view of the chat so far, stored under `~/.ralph/chat`.
- **Compressed memory** - The chat log is summarized into a tree by a background nap pass. Claude opens details with the `zoom` and `recall` tools.
- **Background workers** - Claude starts, messages, and closes workers with `spawn_worker`, `reply`, and `close_worker`. Harnesses: `claude` (the default, a headless `claude -p` child), `claude-tmux` (claude in tmux), `cursor`, `opencode`, and `codex` (see [Codex workers](docs/configuration.md#codex---codex-workers)). Each worker gets its own git worktree. These tools come from `ralph chat-mcp`, which talks to the running chat over a Unix socket.
- **Inbox and watches** - Worker reports and finished `watch` commands land in the chat inbox. Read them with `/inbox`.
- **Threads** - `ralph chat --thread <name>` or `/fork <name>` opens a side chat forked from main; `ralph thread` lists, pulls, merges, parks, revives and drops threads ([guide](docs/context/threads.md)).
- **send-it** - `/send-it <instructions>` turns a request into a plan of steps with exit criteria; after you approve it with `/send-it go`, the chat runs it with no input ([guide](docs/context/send-it.md)).
- **Watch, schedule, hold** - `/watch` runs a command and reports when it ends, `/schedule` lists tasks the model armed, and `/hold` waits with no prompt for a matching report ([details](docs/context/ralph-chat.md#watches)).
- **Markdown and diagrams** - `/view <file.md>` renders a local file, and `/rerender [N]` reprints a stored reply at the current width; images and mermaid diagrams draw inline where the terminal shows images ([details](docs/context/ralph-chat.md#view)).
- **Usage and costs** - Every chat turn, nap, worker session and `ralph ask` call that reports usage writes a usage row; `/workers costs` totals them ([details](docs/context/ralph-chat.md#usage)).
- **Wire recording** - `ralph wire` is a local proxy that records harness-to-provider traffic; `ralph wire ls`, `show` and `diff` explain cost and cache misses ([details](docs/context/ralph-wire.md)).
- **Status line** - Shows the chat mode (`HOLD`, `NAP`, `INBOX`, `WORK`, `IDLE`); colors are set under `[theme.status]`.
- **Chat input** - Multi-line editor with history, `$EDITOR` drafts, clipboard images, `!` shell cells, skills, and an opt-in vi mode (`[chat] edit_mode = "vi"`).
- **Single-shot ask** - `ralph ask` sends one prompt from an argument or stdin and stores only a usage row, not the prompt or reply.
- **Rich terminal output** - Syntax highlighting for code, diffs, and tool results, with built-in themes or custom `.tmTheme` files (`ralph themes`).

## Installation

```bash
git clone https://github.com/cloudbridgeuy/ralph.git
cd ralph
cargo install --locked --path crates/ralph --root ~/.local --force  # Install to ~/.local/bin
cargo install --locked --path crates/ralph --root /usr/local --force  # Custom prefix (binary in /usr/local/bin)
```

`ralph upgrade` replaces an installed binary with the latest GitHub release ([details](docs/context/release-process.md#upgrade-an-install)). A source install is a dev build, so it needs `ralph upgrade --force`.

### Requirements

- Rust 1.95.0 or later
- [Claude Code](https://github.com/anthropics/claude-code) (`claude` on `PATH`)
- git, for worker worktrees
- Per worker harness: `tmux` for `claude-tmux` workers, the Cursor `agent` CLI for `cursor` workers, `opencode` for `opencode` workers

## Quick Start

1. **Chat**:

```bash
ralph
```

2. **Ask a single-shot question**:

```bash
ralph ask 'Explain this codebase'
```

3. **List syntax highlighting themes**:

```bash
ralph themes
```

<!-- ANCHOR_END: start -->

## Configuration

Ralph uses a TOML configuration file stored in platform-specific directories:

| Platform | Location                                          |
| -------- | ------------------------------------------------- |
| Linux    | `~/.config/ralph/config.toml`                     |
| macOS    | `~/Library/Application Support/ralph/config.toml` |
| Windows  | `%APPDATA%\ralph\config.toml`                     |

### Example Configuration

```toml
[theme]
name = "base16-ocean.dark"
no_background = false
```

### Configuration Precedence

1. CLI flags (e.g., `--theme "Solarized (dark)"`)
2. Environment variables (e.g., `RALPH_THEME`)
3. Config file (`config.toml`)
4. Default values

See [Configuration](docs/configuration.md) for complete documentation.

## Documentation

- [CLI Reference](docs/cli-reference.md) - All commands, flags, and options
- [ralph chat](docs/context/ralph-chat.md) - Persistent chat, memory tree and nap, workers, inbox, watches, chat-mcp tools
- [Worker leftovers](docs/context/worker-leftovers.md) - Things workers and smoke tests leave behind outside the repo
- [Threads](docs/context/threads.md) - Open, pull, merge, park, revive and drop side chats
- [send-it](docs/context/send-it.md) - Propose, approve and run an unattended plan
- [ralph ask](docs/context/ralph-ask.md) - Single-shot prompts, prompt sources, permission modes
- [ralph wire](docs/context/ralph-wire.md) - Record and inspect harness-to-provider traffic
- [Headless workers](docs/context/headless-workers.md) - Worker harnesses and `ralph worker verify`
- [Release process](docs/context/release-process.md) - Releases and `ralph upgrade`
- [Configuration](docs/configuration.md) - Config file format and options, including `[chat]` and `[theme.status]`
- [Development Guide](docs/development.md) - Building and contributing
- [Documentation site](https://cloudbridgeuy.github.io/ralph/) - These docs as a book

## Contributing

Contributions are welcome! Please see the [Development Guide](docs/development.md) for setup instructions.

```bash
# Build
cargo build

# Run tests
cargo test

# Run linter
cargo xtask lint

# Install locally for testing
cargo install --locked --path crates/ralph --root ~/.local --force
```

## License

MIT License - see [LICENSE](LICENSE) for details.

The bundled syntax grammars keep their own licences. Run `ralph licenses` to print their notices.
