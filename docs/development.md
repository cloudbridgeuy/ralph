# Development Guide

This guide covers local development setup and build instructions for ralph.

## Prerequisites

- **Rust toolchain**: 1.95.0 or later (MSRV)
  The root `Cargo.toml` sets `rust-version = "1.95.0"` under `[workspace.package]`, and CI and release workflows pin toolchain 1.95.0. Member manifests do not inherit `rust-version`, so Cargo does not enforce it for workspace crates.
- **Cargo**: Included with Rust
- **Git**: For version control and hooks

Install Rust via [rustup](https://rustup.rs/):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Verify your installation:

```bash
rustc --version  # Should be 1.95.0 or later
cargo --version
```

## Building

Clone the repository and build:

```bash
git clone https://github.com/guzmanmonne/ralph.git
cd ralph

# Debug build
cargo build

# Release build (optimized)
cargo build --release
```

## Running

Run ralph directly from cargo:

```bash
# Start chat (bare ralph)
cargo run

# Run a subcommand
cargo run -- ask 'What is Rust?'
cargo run -- themes
```

For release builds:

```bash
cargo run --release -- ask 'What is Rust?'
```

## Testing

Run the test suite:

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run a specific test
cargo test test_name

# Run tests for a specific crate
cargo test -p ralph
cargo test -p ralph_core
```

## Documentation Book

The docs build as an mdBook site from `book.toml` and `docs/SUMMARY.md`.

```bash
cargo install mdbook --locked
mdbook build        # Output in target/book
mdbook serve --open # Live preview in the browser
```

## Code Quality

Use the xtask lint command for comprehensive code quality checks:

```bash
# Run all checks (format, check, clippy, test)
cargo xtask lint

# Auto-fix formatting and clippy issues
cargo xtask lint --fix

# Check all files (not just staged)
cargo xtask lint --force
```

The lint command runs these checks in order:

1. `cargo fmt` - Code formatting
2. `cargo check` - Compilation verification
3. `cargo clippy` - Lint checks (warnings as errors)
4. `cargo test` - Full test suite
5. `cargo rail unify --check` - Dependency unification, unused deps, dead features

### Git Hooks

Install pre-commit hooks to run lint checks automatically:

```bash
# Install hooks
cargo xtask lint --install-hooks

# Check hooks status
cargo xtask lint --hooks-status

# Test hooks
cargo xtask lint --test-hooks

# Uninstall hooks
cargo xtask lint --uninstall-hooks
```

## Installing Locally

Install ralph to your local bin directory:

```bash
# Install to ~/.local/bin (default)
cargo install --path crates/ralph --root ~/.local --force

# Install to custom prefix (binary in <prefix>/bin)
cargo install --path crates/ralph --root /usr/local --force
```

Ensure the installation directory is in your `PATH`.

## Workspace Structure

Ralph uses a Cargo workspace with multiple crates:

```
ralph/
├── Cargo.toml          # Workspace root with shared dependencies
├── clippy.toml         # Clippy configuration (max 5 args)
├── .cargo/
│   └── config.toml     # Cargo aliases (xtask shortcut)
├── crates/
│   ├── core/           # ralph_core - Pure business logic
│   │   └── src/
│   │       ├── chat*.rs    # Chat turn, status, skills logic
│   │       ├── chat_memory/ # Compressed memory tree
│   │       ├── chunk/      # Content chunking
│   │       └── stream/     # LLM stream processing
│   └── ralph/          # ralph CLI - I/O shell
│       └── src/
│           ├── main.rs         # Entry point and command dispatch
│           ├── cli/            # CLI definitions (clap)
│           ├── chat*.rs        # Chat shell, memory store, MCP server
│           ├── invoke.rs       # Shared claude invocation
│           ├── render/         # Terminal rendering
│           ├── stream_processor/ # Stream handling
│           │   ├── tool_display/  # Tool output rendering
│           │   └── tool_results/  # Tool result processing
│           └── subprocess/     # Process management
├── xtask/              # Build automation tasks
│   └── src/
│       └── main.rs     # xtask commands (lint, install)
└── docs/               # Documentation
    └── context/        # Development pattern docs
```

### Crate Responsibilities

| Crate | Purpose |
|-------|---------|
| `ralph_core` | Pure business logic (no I/O). Handles parsing, state, and transformations. |
| `ralph` | CLI application. Handles I/O, subprocess management, terminal rendering. |
| `xtask` | Build automation. Provides lint checks and installation commands. |

This follows the **Functional Core - Imperative Shell** pattern:

- **Core crate**: Pure functions that are easy to test in isolation
- **Ralph crate**: Orchestrates I/O using the core's business logic

## Code Standards

### Clippy Rules

The project enforces strict clippy rules:

- **No `.unwrap()` or `.expect()`** in production code (test code exempt)
- **Maximum 5 function arguments** - use config structs for more
- **Warnings treated as errors**

These are configured in each crate's `lib.rs` or `main.rs`:

```rust
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::expect_used))]
#![cfg_attr(not(test), deny(clippy::too_many_arguments))]
```

### Formatting

All code must be formatted with `cargo fmt`. Run `cargo xtask lint` before committing.

## Environment Variables

For development, these environment variables can override default paths:

| Variable | Description |
|----------|-------------|
| `RALPH_CONFIG_DIR` | Config directory override |
| `RALPH_THEME` | Theme selection |
| `RALPH_NO_BACKGROUND` | Disable theme backgrounds |

## Troubleshooting

### Rust Version Too Old

If you see errors about unsupported Rust features:

```bash
rustup update stable
rustup default stable
```

### Clippy Failures

If clippy fails on `.unwrap()` usage, use proper error handling:

```rust
// Instead of:
let value = some_option.unwrap();

// Use:
let value = some_option.ok_or_else(|| Error::msg("value missing"))?;
```

### Test Failures

Tests must not use `#[serial]` or modify global state. If tests interfere with each other, refactor to use pure functions that don't depend on environment variables or the filesystem.
