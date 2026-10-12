# Rust CLI Error Handling Patterns

Typed `thiserror` enums in modules, `Box<dyn std::error::Error>` in command handlers, `ExitCode` in `main`.

## Entry Point

`crates/ralph/src/main.rs` dispatches to `execute_*` handlers, prints the error to stderr, and returns an exit code.

```rust
fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Themes => execute_themes(),
        Commands::Ask(args) => execute_ask(args),
        // ...
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {}", e);
            ExitCode::FAILURE
        }
    }
}
```

## Command Handlers

Handlers return `Result<_, Box<dyn std::error::Error>>`. Module errors convert via `?`.

```rust
fn execute_ask(args: AskArgs) -> Result<(), Box<dyn std::error::Error>> {
```

## Module Errors with thiserror

Each module defines its own enum. `#[from]` enables `?` conversion.

```rust
// crates/ralph/src/subprocess/types.rs
#[derive(Debug, thiserror::Error)]
pub enum SubprocessError {
    #[error("Failed to spawn subprocess: {0}")]
    SpawnFailed(#[from] io::Error),
    #[error("Subprocess terminated by signal")]
    Signaled,
    // ...
}
```

## No unwrap or expect

`crates/ralph/src/main.rs` denies both outside tests:

```rust
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::expect_used))]
```

Use `?` or `Result`/`Option` combinators. Tests may use `.unwrap()`.
