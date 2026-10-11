# Rust CLI Error Handling Patterns

Typed `thiserror` enums in modules, `Box<dyn std::error::Error>` in command handlers, `ExitCode` in `main`.

## Entry Point

`crates/ralph/src/main.rs` dispatches to `execute_*` handlers, prints the error to stderr, and returns an exit code.

```rust
fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Sessions(args) => execute_sessions(args),
        Commands::Themes => execute_themes(),
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
fn execute_sessions(args: SessionsArgs) -> Result<(), Box<dyn std::error::Error>> {
```

## Module Errors with thiserror

Each module defines its own enum. `#[from]` enables `?` conversion.

```rust
// crates/ralph/src/invoke.rs
#[derive(Debug, thiserror::Error)]
pub enum InvocationError {
    #[error("No prompt provided. Use: ralph ask 'your prompt here'")]
    NoPrompt,
    #[error("Subprocess failed: {0}")]
    Subprocess(#[from] SubprocessError),
    #[error("Session initialization failed: {0}")]
    Session(#[from] SessionError),
}
```

## No unwrap or expect

`crates/ralph/src/main.rs` denies both outside tests:

```rust
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::expect_used))]
```

Use `?` or `Result`/`Option` combinators. Tests may use `.unwrap()`.
