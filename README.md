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
│   Chat and ask with a rich terminal experience              │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust: 1.82.0+](https://img.shields.io/badge/Rust-1.82.0+-orange.svg)](https://www.rust-lang.org/)

---

**ralph** is a CLI tool for working with an LLM from the terminal. It offers a persistent chat with compressed memory and single-shot `ask` prompts, records every session, and renders rich terminal output with syntax highlighting.

## Table of Contents

- [Features](#features)
- [Installation](#installation)
- [Quick Start](#quick-start)
- [Configuration](#configuration)
- [Documentation](#documentation)
- [Contributing](#contributing)
- [License](#license)

## Features

- **Chat** - Persistent chat with compressed memory, run by bare `ralph`
- **Session Management** - Track `ask` sessions with unique slugs and iteration history
- **Rich Terminal Output** - Syntax highlighting for code, diffs, and tool results
- **Replay Sessions** - Review past sessions with full syntax highlighting
- **Theme Support** - Multiple built-in themes or use custom `.tmTheme` files

## Installation

```bash
git clone https://github.com/yourusername/ralph.git
cd ralph
cargo install --path crates/ralph --root ~/.local --force  # Install to ~/.local/bin
cargo install --path crates/ralph --root /usr/local --force  # Custom prefix (binary in /usr/local/bin)
```

### Requirements

- Rust 1.82.0 or later
- An LLM CLI tool (default: [Claude CLI](https://github.com/anthropics/claude-code))

## Quick Start

1. **Chat**:

```bash
ralph
```

2. **Ask a single-shot question**:

```bash
ralph ask 'Explain this codebase'
```

3. **Continue the most recent session in the current project**:

```bash
ralph ask --continue 'Now add tests'
```

4. **Review sessions**:

```bash
# List all sessions
ralph sessions

# Replay a session's output
ralph replay quiet-mountain

# Replay a specific iteration
ralph replay quiet-mountain -i 3
```

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
- [Configuration](docs/configuration.md) - Config file format and options
- [Development Guide](docs/development.md) - Building and contributing

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
cargo install --path crates/ralph --root ~/.local --force
```

## License

MIT License - see [LICENSE](LICENSE) for details.
