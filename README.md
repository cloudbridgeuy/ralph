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
│   Orchestrate LLM-powered development from user stories     │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust: 1.82.0+](https://img.shields.io/badge/Rust-1.82.0+-orange.svg)](https://www.rust-lang.org/)

---

**ralph** is a CLI tool that orchestrates iterative LLM sessions to implement features from a PRD (Product Requirements Document). It manages the feedback loop between you and your LLM, tracking progress, handling failures with automatic retries, and rendering rich terminal output with syntax highlighting.

## Table of Contents

- [Features](#features)
- [Installation](#installation)
- [Quick Start](#quick-start)
- [Configuration](#configuration)
- [Documentation](#documentation)
- [Contributing](#contributing)
- [License](#license)

## Features

- **Iterative Development Loop** - Automatically runs multiple LLM iterations until all stories are complete
- **Session Management** - Track progress across sessions with unique slugs and iteration history
- **Rich Terminal Output** - Syntax highlighting for code, diffs, and tool results
- **Failure Recovery** - Automatic retry logic with configurable attempts and timeouts
- **Replay Sessions** - Review past sessions with full syntax highlighting
- **Customizable Prompts** - Use custom prompt templates or add additional instructions
- **Theme Support** - Multiple built-in themes or use custom `.tmTheme` files

## Installation

```bash
git clone https://github.com/yourusername/ralph.git
cd ralph
cargo xtask install              # Install to ~/.local/bin
cargo xtask install --path /usr/local/bin  # Custom path
```

### Requirements

- Rust 1.82.0 or later
- An LLM CLI tool (default: [Claude CLI](https://github.com/anthropics/claude-code))

## Quick Start

1. **Create a PRD file** at `.local/plans/prd.toml`:

```toml
[[stories]]
category = "feature"
description = "Add user authentication"
steps = [
  "Create login form component",
  "Add authentication API endpoint",
  "Store session tokens securely",
]
passes = false
acceptance = [
  "Users can log in with email and password",
  "Session persists across page refreshes",
]
```

2. **Run ralph**:

```bash
ralph run
```

Ralph will iterate through your stories, invoking the LLM to implement each one until all stories pass or you interrupt the session.

3. **Monitor progress**:

```bash
# List all sessions
ralph sessions

# Replay a session's output
ralph replay quiet-mountain

# Replay a specific iteration
ralph replay quiet-mountain -i 3
```

### Example Output

```
┌────────────────────────────────────────────────────────────────┐
│  ralph run                                                     │
├────────────────────────────────────────────────────────────────┤
│  Session: quiet-mountain                                       │
│  Stories: 3 pending                                            │
└────────────────────────────────────────────────────────────────┘

Iteration 1/3...

[Tool: Edit] src/auth/login.rs
────────────────────────────────────────
  Before:
    1 │ fn login() {
    2 │     todo!()
    3 │ }
────────────────────────────────────────
  After:
    1 │ fn login(email: &str, password: &str) -> Result<Session> {
    2 │     let user = db.find_user_by_email(email)?;
    3 │     verify_password(password, &user.password_hash)?;
    4 │     Ok(Session::create(user.id))
    5 │ }
────────────────────────────────────────
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
- [PRD Format](docs/prd-format.md) - How to structure your PRD files
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
cargo xtask install
```

## License

MIT License - see [LICENSE](LICENSE) for details.
