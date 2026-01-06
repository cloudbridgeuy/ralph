# Rust CLI Project Structure Patterns

This document provides structured guidance for AI agents on organizing Rust CLI applications. These patterns promote maintainability, testability, and clear separation of concerns.

## Cargo Workspace Setup

Use a workspace to organize multi-crate projects. The workspace root `Cargo.toml` defines shared dependencies and member crates.

```toml
[workspace]
members = ["crates/*", "xtask/"]
resolver = "2"

[workspace.package]
edition = "2021"
license = "MIT"
repository = "https://github.com/org/project"

[workspace.dependencies]
serde = { version = "1.0", features = ["derive"] }
clap = { version = "4.5", features = ["derive", "env"] }
tokio = { version = "1.0", features = ["full"] }
color-eyre = "0.6"
reqwest = { version = "0.12", features = ["json"] }
```

Member crates reference workspace dependencies without version duplication:

```toml
# crates/myapp/Cargo.toml
[dependencies]
serde.workspace = true
clap.workspace = true
```

## Two-Crate Architecture (Functional Core - Imperative Shell)

Separate pure logic from I/O operations using two crates:

```
crates/
├── core/           # Functional Core - pure logic
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       └── transform.rs
└── myapp/          # Imperative Shell - CLI, I/O
    ├── Cargo.toml
    └── src/
        ├── main.rs
        └── api.rs
```

**Core crate** (`crates/core/Cargo.toml`):

```toml
[package]
name = "myapp-core"
version = "0.1.0"
edition.workspace = true

[dependencies]
serde.workspace = true
regex = "1.10"
# NO async runtime, NO I/O libraries
```

**CLI crate** (`crates/myapp/Cargo.toml`):

```toml
[package]
name = "myapp"
version = "0.1.0"
edition.workspace = true

[dependencies]
myapp-core = { path = "../core" }
clap.workspace = true
tokio.workspace = true
reqwest.workspace = true
color-eyre.workspace = true
```

The core crate contains pure functions that transform data. The CLI crate handles all I/O and uses core functions for business logic.

## The xtask Pattern

The xtask pattern provides project-specific automation commands without external tooling.

**Directory structure:**

```
project/
├── .cargo/
│   └── config.toml
├── xtask/
│   ├── Cargo.toml
│   └── src/
│       └── main.rs
└── Cargo.toml
```

**Cargo alias** (`.cargo/config.toml`):

```toml
[alias]
xtask = "run --package xtask --"
```

**xtask Cargo.toml:**

```toml
[package]
name = "xtask"
version = "0.1.0"
edition = "2021"
publish = false

[dependencies]
clap = { version = "4.5", features = ["derive"] }
```

**xtask implementation:**

```rust
// xtask/src/main.rs
use clap::{Parser, Subcommand};
use std::process::Command;

#[derive(Parser)]
struct App {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Install the binary to a target directory
    Install {
        #[arg(long, default_value = "~/.local/bin")]
        path: String,
    },
    /// Create a new release
    Release {
        #[arg(long)]
        version: String,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = App::parse();
    match app.command {
        Commands::Install { path } => install(&path),
        Commands::Release { version } => release(&version),
    }
}

fn install(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    Command::new("cargo")
        .args(["build", "--release"])
        .status()?;
    // Copy binary to path...
    Ok(())
}

fn release(version: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Tag and release logic...
    Ok(())
}
```

**Usage:**

```bash
cargo xtask install
cargo xtask install --path /usr/local/bin
cargo xtask release --version 1.0.0
```

## Prelude Module Pattern

A prelude module re-exports commonly used types to reduce import boilerplate across the crate.

```rust
// src/prelude.rs
pub use crate::error::Error;
pub use color_eyre::eyre::{eyre, Context, Result};
pub use anstream::{eprintln, println};
pub use tracing::{debug, error, info, warn};
```

**Usage in other modules:**

```rust
// src/feature/subcommand.rs
use crate::prelude::*;

pub async fn run(opts: Options) -> Result<()> {
    let data = fetch().await.context("failed to fetch data")?;
    println!("{}", data);
    Ok(())
}
```

Only include items that are used in most modules. Avoid polluting the prelude with rarely-used types.

## Feature Module Organization

Organize code by feature, with each feature in its own directory containing related subcommands.

```
src/
├── main.rs           # Entry point, top-level command dispatch
├── prelude.rs        # Common re-exports
├── error.rs          # Error types
├── global.rs         # Global options/state
├── auth/
│   ├── mod.rs        # Auth feature App, Commands, run()
│   ├── login.rs      # login subcommand
│   └── logout.rs     # logout subcommand
├── config/
│   ├── mod.rs
│   ├── get.rs
│   └── set.rs
└── api/
    ├── mod.rs
    └── client.rs
```

Each feature directory mirrors the CLI structure. A command like `myapp auth login` maps to `src/auth/login.rs`.

## Module Pattern (mod.rs)

Each feature's `mod.rs` defines the feature's CLI structure and dispatches to subcommands.

```rust
// src/auth/mod.rs
pub mod login;
pub mod logout;

use crate::prelude::*;

#[derive(Debug, clap::Parser)]
pub struct App {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, clap::Subcommand)]
pub enum Commands {
    /// Authenticate with the service
    Login(login::Options),
    /// Clear authentication credentials
    Logout(logout::Options),
}

pub async fn run(app: App, global: crate::Global) -> Result<()> {
    match app.command {
        Commands::Login(opts) => login::run(opts, global).await,
        Commands::Logout(opts) => logout::run(opts, global).await,
    }
}
```

**Subcommand module structure:**

```rust
// src/auth/login.rs
use crate::prelude::*;

#[derive(Debug, clap::Parser)]
pub struct Options {
    /// Username for authentication
    #[arg(long, env = "MYAPP_USERNAME")]
    pub username: String,

    /// Output format
    #[arg(long, default_value = "text")]
    pub format: OutputFormat,
}

pub async fn run(opts: Options, global: crate::Global) -> Result<()> {
    let token = authenticate(&opts.username).await?;
    store_token(&token)?;
    println!("Authenticated as {}", opts.username);
    Ok(())
}
```

## Data Function Pattern

Separate data fetching from presentation to enable reuse across CLI, MCP servers, tests, and other interfaces.

```rust
// src/users/list.rs
use crate::prelude::*;

#[derive(Debug, Clone, clap::Parser)]
pub struct Options {
    /// Filter by status
    #[arg(long)]
    pub status: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

/// CLI entry point - handles presentation
pub async fn run(opts: Options, global: crate::Global) -> Result<()> {
    let users = fetch_users(opts.clone(), &global).await?;

    if opts.json {
        println!("{}", serde_json::to_string_pretty(&users)?);
    } else {
        print_users_table(&users);
    }

    Ok(())
}

/// Data function - reusable for MCP, tests, library use
pub async fn fetch_users(opts: Options, global: &crate::Global) -> Result<Vec<User>> {
    let raw = global
        .client
        .get_users(opts.status.as_deref())
        .await
        .context("failed to fetch users")?;

    // Use pure core function for transformation
    Ok(myapp_core::transform_users(raw))
}

fn print_users_table(users: &[User]) {
    for user in users {
        println!("{:<20} {:<10}", user.name, user.status);
    }
}
```

This pattern enables:
- **Testing**: Call `fetch_users()` directly in tests without parsing CLI args
- **MCP integration**: Expose `fetch_users()` as an MCP tool
- **Library use**: Import the crate and call data functions programmatically

## Main Entry Point Structure

The main entry point parses arguments, initializes shared state, and dispatches to features.

```rust
// src/main.rs
mod auth;
mod config;
mod error;
mod global;
mod prelude;

use clap::Parser;
use prelude::*;

#[derive(Debug, Parser)]
#[command(name = "myapp", version, about)]
pub struct App {
    #[command(flatten)]
    pub global: Global,

    #[command(subcommand)]
    pub command: SubCommands,
}

#[derive(Debug, Parser)]
pub struct Global {
    /// Enable verbose output
    #[arg(long, short, global = true)]
    pub verbose: bool,

    /// API base URL
    #[arg(long, env = "MYAPP_API_URL", global = true)]
    pub api_url: Option<String>,
}

#[derive(Debug, clap::Subcommand)]
pub enum SubCommands {
    /// Manage authentication
    Auth(auth::App),
    /// Manage configuration
    Config(config::App),
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let app = App::parse();

    // Initialize tracing based on verbosity
    if app.global.verbose {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .init();
    }

    match app.command {
        SubCommands::Auth(sub) => auth::run(sub, app.global).await,
        SubCommands::Config(sub) => config::run(sub, app.global).await,
    }
}
```

## Complete Project Layout

A complete project following all patterns:

```
myapp/
├── .cargo/
│   └── config.toml           # Cargo aliases (xtask)
├── .claude/
│   └── context/              # AI agent documentation
├── crates/
│   ├── core/                 # Pure business logic
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       └── transform.rs
│   └── myapp/                # CLI application
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs
│           ├── prelude.rs
│           ├── error.rs
│           ├── global.rs
│           ├── auth/
│           │   ├── mod.rs
│           │   ├── login.rs
│           │   └── logout.rs
│           └── config/
│               ├── mod.rs
│               ├── get.rs
│               └── set.rs
├── xtask/
│   ├── Cargo.toml
│   └── src/
│       └── main.rs
├── Cargo.toml                # Workspace root
├── Cargo.lock
└── README.md
```

## Key Principles for AI Agents

1. **Follow existing patterns**: When adding features, mirror the structure of existing features in the codebase.

2. **Pure core, impure shell**: Business logic goes in the core crate. I/O operations stay in the CLI crate.

3. **Data functions enable reuse**: Always separate data fetching from presentation.

4. **One responsibility per file**: Each subcommand lives in its own file with `Options` and `run()`.

5. **Global options flow down**: Pass `Global` through the call chain rather than using global state.

6. **Use the prelude**: Import `crate::prelude::*` instead of individual items for common types.

7. **Workspace dependencies**: Define versions once in workspace root, reference with `.workspace = true`.
