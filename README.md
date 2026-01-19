# ralph

Ralph is a CLI tool for LLM-powered vibe coding. It orchestrates iterative LLM sessions to implement user stories from a PRD (Product Requirements Document), tracking progress, managing sessions, and providing rich terminal output with syntax highlighting.

## Installation

```bash
cargo xtask install              # Install to ~/.local/bin
cargo xtask install --path /usr/local/bin  # Custom path
```

## Quick Start

```bash
# Run the iteration loop with default settings
ralph run

# Run exactly 5 iterations
ralph run 5

# List sessions
ralph sessions

# Replay a session's output
ralph replay quiet-mountain
```

## Configuration

Ralph uses a TOML configuration file for persistent settings. Configuration is stored in platform-specific directories:

| Platform | Config File Location |
|----------|---------------------|
| **Linux** | `~/.config/ralph/config.toml` |
| **macOS** | `~/Library/Application Support/ralph/config.toml` |
| **Windows** | `%APPDATA%\ralph\config.toml` |

### Example Configuration

```toml
[theme]
# Syntax highlighting theme (run `ralph themes` to see options)
name = "base16-ocean.dark"

# Disable theme background colors to use terminal's default
no_background = false
```

### Configuration Precedence

Settings are resolved in order (highest priority first):

1. CLI flags (e.g., `--theme "Solarized (dark)"`)
2. Environment variables (e.g., `RALPH_THEME`)
3. Config file (`config.toml`)
4. Default values

For full configuration documentation, see [docs/configuration.md](docs/configuration.md).

## Documentation

- [CLI Reference](docs/cli-reference.md) - Complete reference for all commands and options
- [Configuration](docs/configuration.md) - Detailed configuration file documentation
- [Development Guide](docs/development.md) - Local development and build instructions
