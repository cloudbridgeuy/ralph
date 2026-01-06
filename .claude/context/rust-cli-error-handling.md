# Rust CLI Error Handling Patterns

Error handling in Rust CLI applications follows a layered approach: rich, user-friendly errors at the CLI boundary, and clean, typed errors in core business logic.

## Error Architecture Overview

```
┌─────────────────────────────────────────┐
│  CLI Layer (main, commands)             │
│  → color_eyre::Result<()>               │
│  → Rich error messages with context     │
│  → Backtraces for debugging             │
├─────────────────────────────────────────┤
│  Core Layer (business logic)            │
│  → Custom error enums (thiserror)       │
│  → Or simple Result<T, String>          │
│  → No color_eyre dependency             │
└─────────────────────────────────────────┘
```

## Setting Up color_eyre

Initialize `color_eyre` at the start of `main()` to enable enhanced error reports with backtraces and span traces.

```rust
use color_eyre::eyre::Result;

fn main() -> Result<()> {
    color_eyre::install()?;
    
    // Application logic here
    run()
}
```

This must be called before any errors are created. It sets up the global error hook.

## Prelude Pattern for Imports

Create a prelude module to standardize error handling imports across the crate.

```rust
// src/prelude.rs
pub use color_eyre::eyre::{eyre, Context, OptionExt, Result};
```

Usage in other modules:

```rust
use crate::prelude::*;

fn do_something() -> Result<()> {
    let value = some_option.ok_or_eyre("value was None")?;
    Ok(())
}
```

Key exports:
- `eyre!` - Create ad-hoc errors with formatting
- `Context` - Adds `.context()` and `.with_context()` to `Result` and `Option`
- `OptionExt` - Adds `.ok_or_eyre()` to `Option`
- `Result` - The `color_eyre::Result<T>` type alias

## Custom Errors with thiserror

Use `thiserror` to define structured error types for core logic.

```rust
#[derive(thiserror::Error, Debug)]
pub enum ConfigError {
    #[error("Configuration file not found: {0}")]
    NotFound(PathBuf),
    
    #[error("Invalid configuration: {message}")]
    Invalid { message: String },
    
    #[error("Failed to parse configuration")]
    Parse(#[from] toml::de::Error),
    
    #[error("IO error reading config")]
    Io(#[from] std::io::Error),
}
```

The `#[from]` attribute enables automatic conversion via the `?` operator.

## Error Propagation Patterns

### Basic Propagation with `?`

```rust
fn read_config(path: &Path) -> Result<Config> {
    let content = std::fs::read_to_string(path)?;
    let config: Config = toml::from_str(&content)?;
    Ok(config)
}
```

### Adding Context to Errors

Use `.context()` to add information that helps users understand what operation failed.

```rust
fn read_config(path: &Path) -> Result<Config> {
    let content = std::fs::read_to_string(path)
        .context("Failed to read configuration file")?;
    
    let config: Config = toml::from_str(&content)
        .with_context(|| format!("Failed to parse config at {}", path.display()))?;
    
    Ok(config)
}
```

Use `.with_context()` when the context message requires computation (lazy evaluation).

### Converting Errors with map_err

For errors that don't implement `std::error::Error` or need custom conversion:

```rust
fn parse_port(s: &str) -> Result<u16> {
    s.parse::<u16>()
        .map_err(|e| eyre!("Invalid port '{}': {}", s, e))
}
```

### Converting Option to Result

```rust
fn get_user(users: &HashMap<String, User>, id: &str) -> Result<&User> {
    users.get(id)
        .ok_or_eyre("User not found")
}

// With dynamic message
fn get_user(users: &HashMap<String, User>, id: &str) -> Result<&User> {
    users.get(id)
        .ok_or_else(|| eyre!("User '{}' not found", id))
}
```

## HTTP Response Error Handling

When working with HTTP clients, check response status and include relevant details in errors.

```rust
async fn fetch_data(client: &Client, url: &str) -> Result<ApiResponse> {
    let response = client.get(url)
        .send()
        .await
        .context("Failed to send request")?;
    
    let status = response.status();
    
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(eyre!("API error [{}]: {}", status, body));
    }
    
    response.json::<ApiResponse>()
        .await
        .context("Failed to parse API response")
}
```

For more structured API error handling:

```rust
async fn fetch_data(client: &Client, url: &str) -> Result<ApiResponse> {
    let response = client.get(url)
        .send()
        .await
        .context("Failed to send request")?;
    
    let status = response.status();
    
    match status {
        s if s.is_success() => {
            response.json::<ApiResponse>()
                .await
                .context("Failed to parse API response")
        }
        StatusCode::NOT_FOUND => Err(eyre!("Resource not found")),
        StatusCode::UNAUTHORIZED => Err(eyre!("Authentication required")),
        StatusCode::FORBIDDEN => Err(eyre!("Access denied")),
        _ => {
            let body = response.text().await.unwrap_or_default();
            Err(eyre!("API error [{}]: {}", status, body))
        }
    }
}
```

## Core Crate Errors (Without color_eyre)

Core/library crates should avoid `color_eyre` to remain flexible. Use manual implementations or simple strings.

### Manual Display + Error Implementation

```rust
#[derive(Debug)]
pub enum CoreError {
    InvalidInput(String),
    NotFound { resource: String, id: String },
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(msg) => write!(f, "Invalid input: {}", msg),
            Self::NotFound { resource, id } => {
                write!(f, "{} not found: {}", resource, id)
            }
        }
    }
}

impl std::error::Error for CoreError {}
```

### Simple Result<T, String>

For internal utilities where structured errors aren't needed:

```rust
fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Name cannot be empty".to_string());
    }
    if name.len() > 100 {
        return Err(format!("Name too long: {} chars (max 100)", name.len()));
    }
    Ok(())
}
```

Convert to `color_eyre::Result` at the CLI boundary:

```rust
fn handle_command(name: &str) -> Result<()> {
    validate_name(name).map_err(|e| eyre!("{}", e))?;
    Ok(())
}
```

## Best Practices Summary

1. **Initialize early**: Call `color_eyre::install()` at the start of `main()`

2. **Use context liberally**: Add `.context()` to explain what operation failed

3. **Keep core logic clean**: Use `thiserror` or simple `Result<T, String>` in core crates

4. **Include relevant data**: Error messages should include values that help debugging (paths, IDs, status codes)

5. **Convert at boundaries**: Transform core errors to `color_eyre::Result` at command handlers

6. **Avoid `unwrap()` and `expect()`**: Use `?` with context instead, except in tests or truly impossible cases

7. **Use `eyre!` for ad-hoc errors**: When a structured error type isn't warranted

```rust
// Prefer this
let file = File::open(path)
    .with_context(|| format!("Failed to open {}", path.display()))?;

// Over this
let file = File::open(path).expect("Failed to open file");
```
