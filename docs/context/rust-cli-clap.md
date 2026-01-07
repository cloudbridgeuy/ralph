# Rust CLI Patterns with Clap

This document provides structured guidance for AI agents implementing CLI applications using the `clap` crate in Rust. Follow these patterns for consistent, maintainable command-line interfaces.

## Main CLI Struct

The entry point for your CLI. Use `#[derive(clap::Parser)]` on the main struct.

```rust
use clap::Parser;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    /// Subcommand to execute
    #[command(subcommand)]
    pub command: Commands,

    /// Global arguments available to all subcommands
    #[clap(flatten)]
    pub global: GlobalArgs,
}
```

- `author`, `version`, `about` are pulled from `Cargo.toml` automatically
- `long_about` can provide extended help text (set to `None` to use `about`)
- `propagate_version = true` makes `--version` available on subcommands
- `#[command(subcommand)]` marks the field holding subcommands
- `#[clap(flatten)]` inlines fields from another struct

## Subcommand Enum

Define available commands using `#[derive(clap::Subcommand)]`.

```rust
use clap::Subcommand;

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize a new project
    Init(InitArgs),

    /// Run the application
    Run {
        /// Path to config file
        #[arg(short, long)]
        config: Option<PathBuf>,
    },

    /// Manage configuration
    #[command(subcommand)]
    Config(ConfigCommands),

    /// Print version information
    Version,
}
```

- **Tuple variants** `Init(InitArgs)`: Use when command has many arguments (cleaner)
- **Struct variants** `Run { ... }`: Use for simple commands with few arguments
- **Nested subcommands**: Add `#[command(subcommand)]` for sub-subcommands
- **Unit variants** `Version`: Use for commands with no arguments

### Nested Subcommands

```rust
#[derive(Subcommand)]
pub enum ConfigCommands {
    /// Show current configuration
    Show,

    /// Set a configuration value
    Set {
        #[arg(value_name = "KEY")]
        key: String,

        #[arg(value_name = "VALUE")]
        value: String,
    },

    /// Reset configuration to defaults
    Reset,
}
```

## Argument Groups

Use `#[derive(clap::Args)]` to group related arguments into reusable structs.

```rust
use clap::Args;

#[derive(Args)]
pub struct GlobalArgs {
    /// Enable verbose output
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Suppress all output
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Output format
    #[arg(long, global = true, default_value = "text")]
    pub format: OutputFormat,
}

#[derive(Args)]
pub struct InitArgs {
    /// Project name
    #[arg(value_name = "NAME")]
    pub name: String,

    /// Template to use
    #[arg(short, long)]
    pub template: Option<String>,

    /// Directory to create project in
    #[arg(short, long, default_value = ".")]
    pub directory: PathBuf,
}
```

- **Positional arguments**: No `short` or `long` attribute
- **Optional arguments**: Use `Option<T>` type
- **Required arguments**: Use non-Option type
- `global = true`: Argument available to all subcommands (must be on flattened struct in root)

## Value Enums

Use `#[derive(clap::ValueEnum)]` for arguments with constrained choices.

```rust
use clap::ValueEnum;

#[derive(ValueEnum, Clone, Copy, Debug, Default)]
pub enum OutputFormat {
    /// Plain text output
    #[default]
    Text,

    /// JSON output
    Json,

    /// YAML output
    Yaml,

    /// Table output
    #[value(alias = "tbl")]
    Table,
}
```

- Clap automatically generates help text from variant names
- Use `#[default]` with `Default` derive for default value
- Use `#[value(alias = "...")]` for alternative names
- Variants are matched case-insensitively by default

## Common Attributes

### Argument Attributes

```rust
#[derive(Args)]
pub struct ExampleArgs {
    /// Short and long flags: -n, --name
    #[arg(short, long)]
    pub name: String,

    /// Custom short flag: -N
    #[arg(short = 'N', long)]
    pub namespace: String,

    /// Environment variable fallback
    #[arg(long, env = "APP_TOKEN")]
    pub token: Option<String>,

    /// Default value (must be string literal)
    #[arg(long, default_value = "localhost")]
    pub host: String,

    /// Default value for non-string types
    #[arg(long, default_value_t = 8080)]
    pub port: u16,

    /// Hidden from help output
    #[arg(long, hide = true)]
    pub internal_flag: bool,

    /// Custom value name in help
    #[arg(long, value_name = "PATH")]
    pub config: PathBuf,

    /// Multiple values: --item foo --item bar
    #[arg(long)]
    pub item: Vec<String>,

    /// Required flag (unusual but possible)
    #[arg(long, required = true)]
    pub required_option: String,
}
```

### Command Attributes

```rust
#[derive(Parser)]
#[command(
    author,
    version,
    about = "Short description",
    long_about = "Longer description that appears in --help",
    after_help = "Examples:\n  app init my-project\n  app run --config app.toml",
    after_long_help = "Extended examples and documentation...",
)]
pub struct Cli {
    // ...
}

#[derive(Subcommand)]
pub enum Commands {
    /// Command description (becomes about)
    #[command(alias = "i", visible_alias = "initialize")]
    Init(InitArgs),

    /// Hidden command (not shown in help)
    #[command(hide = true)]
    Internal,
}
```

## Command Routing Pattern

### Main Entry Point (`main.rs`)

```rust
use clap::Parser;

mod commands;

fn main() -> anyhow::Result<()> {
    let cli = commands::Cli::parse();

    match cli.command {
        commands::Commands::Init(args) => commands::init::run(args, &cli.global),
        commands::Commands::Run { config } => commands::run::run(config, &cli.global),
        commands::Commands::Config(cmd) => commands::config::run(cmd, &cli.global),
        commands::Commands::Version => {
            println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}
```

### Module Structure

```
src/
  main.rs
  commands/
    mod.rs        # Cli, Commands, GlobalArgs definitions
    init.rs       # init command implementation
    run.rs        # run command implementation
    config/
      mod.rs      # ConfigCommands enum and routing
      show.rs     # config show implementation
      set.rs      # config set implementation
```

### Module-Level Pattern (`commands/mod.rs`)

```rust
use clap::{Parser, Subcommand, Args};

pub mod init;
pub mod run;
pub mod config;

#[derive(Parser)]
#[command(author, version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    #[clap(flatten)]
    pub global: GlobalArgs,
}

#[derive(Subcommand)]
pub enum Commands {
    Init(init::Args),
    Run(run::Args),
    #[command(subcommand)]
    Config(config::Commands),
}

#[derive(Args, Clone)]
pub struct GlobalArgs {
    #[arg(short, long, global = true)]
    pub verbose: bool,
}
```

### Command Implementation (`commands/init.rs`)

```rust
use clap::Args;
use crate::commands::GlobalArgs;

#[derive(Args)]
pub struct Args {
    /// Project name
    pub name: String,

    /// Template to use
    #[arg(short, long)]
    pub template: Option<String>,
}

pub fn run(args: Args, global: &GlobalArgs) -> anyhow::Result<()> {
    if global.verbose {
        println!("Initializing project: {}", args.name);
    }

    // Implementation...

    Ok(())
}
```

### Nested Subcommand Routing (`commands/config/mod.rs`)

```rust
use clap::Subcommand;
use crate::commands::GlobalArgs;

pub mod show;
pub mod set;

#[derive(Subcommand)]
pub enum Commands {
    Show,
    Set {
        key: String,
        value: String,
    },
}

pub fn run(cmd: Commands, global: &GlobalArgs) -> anyhow::Result<()> {
    match cmd {
        Commands::Show => show::run(global),
        Commands::Set { key, value } => set::run(&key, &value, global),
    }
}
```

## Combining with Serde

Reuse argument structs for both CLI parsing and configuration files.

```rust
use clap::Args;
use serde::{Deserialize, Serialize};

#[derive(Args, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct ServerConfig {
    /// Host to bind to
    #[arg(long, default_value = "127.0.0.1")]
    #[serde(default = "default_host")]
    pub host: String,

    /// Port to listen on
    #[arg(long, default_value_t = 8080)]
    #[serde(default = "default_port")]
    pub port: u16,

    /// Number of worker threads
    #[arg(long)]
    pub workers: Option<usize>,
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    8080
}

// Merge CLI args over config file values
impl ServerConfig {
    pub fn merge(self, other: ServerConfig) -> ServerConfig {
        ServerConfig {
            host: other.host,
            port: other.port,
            workers: other.workers.or(self.workers),
        }
    }
}
```

Usage pattern:

```rust
pub fn run(args: Args, global: &GlobalArgs) -> anyhow::Result<()> {
    // Load config file if provided
    let file_config = if let Some(path) = &args.config {
        let content = std::fs::read_to_string(path)?;
        toml::from_str(&content)?
    } else {
        ServerConfig::default()
    };

    // CLI args override file config
    let config = file_config.merge(args.server);

    // Use merged config...
    Ok(())
}
```

## Quick Reference

| Attribute | Purpose | Example |
|-----------|---------|---------|
| `#[derive(Parser)]` | Main CLI struct | `struct Cli { ... }` |
| `#[derive(Subcommand)]` | Command enum | `enum Commands { ... }` |
| `#[derive(Args)]` | Argument group | `struct GlobalArgs { ... }` |
| `#[derive(ValueEnum)]` | Constrained choices | `enum Format { Json, Yaml }` |
| `#[command(subcommand)]` | Mark subcommand field | On enum field in Parser struct |
| `#[clap(flatten)]` | Inline args from struct | On Args field |
| `#[arg(short, long)]` | Enable -x, --xxx flags | On field |
| `#[arg(env = "VAR")]` | Environment fallback | `env = "APP_TOKEN"` |
| `#[arg(default_value = "x")]` | String default | For String fields |
| `#[arg(default_value_t = x)]` | Typed default | For non-String fields |
| `#[arg(global = true)]` | Available to subcommands | On global args |
| `#[arg(hide = true)]` | Hide from help | Internal flags |
| `#[arg(value_name = "X")]` | Custom placeholder | `value_name = "PATH"` |
| `#[command(alias = "x")]` | Hidden alias | `alias = "i"` |
| `#[command(visible_alias = "x")]` | Shown alias | `visible_alias = "init"` |
| `#[command(after_help = "...")]` | Text after help | Examples section |
| `#[value(alias = "x")]` | ValueEnum alias | `alias = "tbl"` |

## Dependencies

Add to `Cargo.toml`:

```toml
[dependencies]
clap = { version = "4", features = ["derive", "env"] }

# Optional: for serde integration
serde = { version = "1", features = ["derive"] }
```

Features:
- `derive`: Enables derive macros (required for this pattern)
- `env`: Enables `#[arg(env = "...")]` support
- `string`: Enables `String` methods on `Command` for runtime building
