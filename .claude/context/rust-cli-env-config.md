# Rust CLI Environment Variable and Configuration Patterns

This document provides patterns for handling environment variables and configuration in Rust CLI applications. Use these patterns when implementing configuration loading, CLI argument parsing, or environment-based settings.

## Clap Environment Variable Integration

Clap can automatically read environment variables as fallbacks for CLI arguments using the `env` attribute:

```rust
use clap::Parser;

#[derive(Parser)]
pub struct Args {
    #[arg(long, env = "MY_APP_TOKEN")]
    token: Option<String>,

    #[arg(long, env = "MY_APP_BASE_URL", default_value = "https://api.example.com")]
    base_url: String,
}
```

The precedence is: CLI argument > environment variable > default value.

## Hiding Secrets from Help Output

Sensitive values should be hidden from `--help` output to avoid exposing them in logs or screenshots:

```rust
#[derive(Parser)]
pub struct Args {
    #[arg(long, env = "API_TOKEN", hide = true)]
    api_token: Option<String>,

    #[arg(long, env = "API_SECRET", hide_env_values = true)]
    api_secret: Option<String>,
}
```

Use `hide = true` to hide the entire argument. Use `hide_env_values = true` to show the argument but mask the environment variable value.

## Config Structs with from_env()

Create dedicated config structs with a `from_env()` constructor for clean separation of configuration loading:

```rust
use eyre::{eyre, Result};

pub struct Config {
    pub base_url: String,
    pub api_token: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            base_url: std::env::var("BASE_URL")
                .map_err(|_| eyre!("BASE_URL not set"))?,
            api_token: std::env::var("API_TOKEN")
                .map_err(|_| eyre!("API_TOKEN not set"))?,
        })
    }
}
```

This pattern keeps environment access in one place and provides clear error messages for missing configuration.

## Fallback Patterns

When multiple environment variables can provide the same value, use fallback chains:

```rust
let url = std::env::var("SERVICE_URL")
    .or_else(|_| std::env::var("SHARED_URL"))
    .map_err(|_| eyre!("Neither SERVICE_URL nor SHARED_URL set"))?;
```

For more complex fallbacks with different types or transformations:

```rust
let url = std::env::var("SERVICE_URL")
    .ok()
    .or_else(|| std::env::var("SHARED_URL").ok())
    .or_else(|| config_file.url.clone())
    .ok_or_else(|| eyre!("No URL configured"))?;
```

## Default Values

Provide sensible defaults when configuration is optional:

```rust
let timeout = std::env::var("TIMEOUT")
    .unwrap_or_else(|_| "30".to_string());

// With parsing
let timeout: u64 = std::env::var("TIMEOUT")
    .unwrap_or_else(|_| "30".to_string())
    .parse()
    .map_err(|_| eyre!("TIMEOUT must be a number"))?;
```

For boolean flags:

```rust
let debug = std::env::var("DEBUG")
    .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    .unwrap_or(false);
```

## Configuration Precedence

Standard precedence order from highest to lowest priority:

1. **CLI arguments** - Explicit user intent for this invocation
2. **Environment variables** - Session or deployment configuration
3. **Config files** - Persistent user preferences
4. **Hardcoded defaults** - Sensible fallbacks

Implement this pattern:

```rust
pub struct Config {
    pub base_url: String,
    pub timeout: u64,
}

impl Config {
    pub fn load(args: &Args) -> Result<Self> {
        // Start with defaults
        let mut config = Self::default();

        // Layer config file if present
        if let Some(file_config) = Self::load_config_file()? {
            config = config.merge(file_config);
        }

        // Layer environment variables
        config = config.with_env();

        // Layer CLI arguments (highest priority)
        config = config.with_args(args);

        Ok(config)
    }
}
```

## CLI Overrides Pattern

Allow CLI arguments to override configuration from other sources:

```rust
impl Config {
    pub fn with_overrides(
        mut self,
        url: Option<String>,
        timeout: Option<u64>,
    ) -> Self {
        if let Some(u) = url {
            self.base_url = u;
        }
        if let Some(t) = timeout {
            self.timeout = t;
        }
        self
    }
}

// Usage
let config = Config::from_env()?
    .with_overrides(args.url, args.timeout);
```

For many fields, use a builder pattern or derive macro.

## Naming Conventions

Follow these conventions for environment variable names:

- **Case**: SCREAMING_SNAKE_CASE
- **Prefix**: Use app or service name to avoid collisions
- **Grouping**: Group related variables with common prefixes

```rust
// Good: Prefixed and grouped
const ENV_MYAPP_BASE_URL: &str = "MYAPP_BASE_URL";
const ENV_MYAPP_API_TOKEN: &str = "MYAPP_API_TOKEN";
const ENV_MYAPP_TIMEOUT: &str = "MYAPP_TIMEOUT";

// Good: Database configuration grouped
const ENV_DB_HOST: &str = "MYAPP_DB_HOST";
const ENV_DB_PORT: &str = "MYAPP_DB_PORT";
const ENV_DB_USER: &str = "MYAPP_DB_USER";
const ENV_DB_PASSWORD: &str = "MYAPP_DB_PASSWORD";
```

Define constants to avoid typos and enable refactoring:

```rust
mod env_vars {
    pub const BASE_URL: &str = "MYAPP_BASE_URL";
    pub const API_TOKEN: &str = "MYAPP_API_TOKEN";
}

// Usage
let url = std::env::var(env_vars::BASE_URL)?;
```

## Home Directory for Config Files

Locate user config files in standard locations:

```rust
use std::path::PathBuf;

fn config_dir() -> Result<PathBuf> {
    // Cross-platform: HOME on Unix, USERPROFILE on Windows
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| eyre!("Could not determine home directory"))?;

    Ok(PathBuf::from(home).join(".config/myapp"))
}

fn config_file_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}
```

For XDG compliance on Linux:

```rust
fn config_dir() -> Result<PathBuf> {
    if let Ok(xdg_config) = std::env::var("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(xdg_config).join("myapp"));
    }

    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| eyre!("Could not determine home directory"))?;

    Ok(PathBuf::from(home).join(".config/myapp"))
}
```

## Complete Example

Combining all patterns:

```rust
use clap::Parser;
use eyre::{eyre, Result};
use std::path::PathBuf;

mod env_vars {
    pub const BASE_URL: &str = "MYAPP_BASE_URL";
    pub const API_TOKEN: &str = "MYAPP_API_TOKEN";
    pub const TIMEOUT: &str = "MYAPP_TIMEOUT";
}

#[derive(Parser)]
pub struct Args {
    #[arg(long, env = env_vars::BASE_URL)]
    base_url: Option<String>,

    #[arg(long, env = env_vars::API_TOKEN, hide = true)]
    api_token: Option<String>,

    #[arg(long, env = env_vars::TIMEOUT, default_value = "30")]
    timeout: u64,
}

pub struct Config {
    pub base_url: String,
    pub api_token: String,
    pub timeout: u64,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            base_url: std::env::var(env_vars::BASE_URL)
                .unwrap_or_else(|_| "https://api.example.com".to_string()),
            api_token: std::env::var(env_vars::API_TOKEN)
                .map_err(|_| eyre!("{} not set", env_vars::API_TOKEN))?,
            timeout: std::env::var(env_vars::TIMEOUT)
                .unwrap_or_else(|_| "30".to_string())
                .parse()
                .map_err(|_| eyre!("{} must be a number", env_vars::TIMEOUT))?,
        })
    }

    pub fn with_overrides(
        mut self,
        base_url: Option<String>,
        timeout: Option<u64>,
    ) -> Self {
        if let Some(u) = base_url {
            self.base_url = u;
        }
        if let Some(t) = timeout {
            self.timeout = t;
        }
        self
    }
}
```
