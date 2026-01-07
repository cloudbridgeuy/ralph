# Rust CLI I/O Patterns

This document provides structured guidance for AI agents implementing stdin/stdout/stderr patterns in Rust CLI applications. These patterns are generic and applicable to any Rust CLI project.

## Async Stdin Reading

Use tokio's async I/O for non-blocking line-by-line input processing. This is essential for CLIs that need to process piped input or handle concurrent operations.

```rust
use tokio::io::{AsyncBufReadExt, BufReader};

async fn read_stdin_lines() -> Result<Vec<String>, std::io::Error> {
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();
    let mut lines = Vec::new();

    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line).await?;
        if bytes_read == 0 {
            break; // EOF reached
        }
        lines.push(line.trim_end().to_string());
    }

    Ok(lines)
}
```

Key points:
- Always `clear()` the line buffer before each read to avoid accumulation
- Check for `bytes_read == 0` to detect EOF
- Use `trim_end()` to remove trailing newlines if needed

## Async Stdout Writing

Use tokio's async stdout for non-blocking output. Always flush after writes to ensure data is visible immediately.

```rust
use tokio::io::{AsyncWriteExt, stdout};

async fn write_output(data: &str) -> Result<(), std::io::Error> {
    let mut stdout = stdout();
    stdout.write_all(data.as_bytes()).await?;
    stdout.write_all(b"\n").await?;
    stdout.flush().await?; // Critical: ensures output is visible
    Ok(())
}
```

Key points:
- `flush()` is mandatory after writes, especially before program exit or when output is piped
- Batch multiple small writes when possible to reduce syscall overhead
- Use `write_all()` instead of `write()` to ensure complete data transfer

## Terminal Detection (TTY vs Piped)

Detect whether stdout is a terminal to adjust output behavior. Use interactive features (colors, spinners) only when connected to a terminal.

```rust
use std::io::IsTerminal;

fn configure_output() {
    if std::io::stdout().is_terminal() {
        // Interactive mode:
        // - Enable colors and formatting
        // - Show progress indicators
        // - Display rich metadata
    } else {
        // Piped mode:
        // - Plain text output only
        // - Machine-parseable format
        // - No ANSI escape codes
    }
}
```

Check each stream independently when needed:

```rust
use std::io::IsTerminal;

let stdin_is_tty = std::io::stdin().is_terminal();
let stdout_is_tty = std::io::stdout().is_terminal();
let stderr_is_tty = std::io::stderr().is_terminal();
```

Key points:
- `IsTerminal` trait is available in std since Rust 1.70
- Check stdin to detect if input is piped vs interactive
- Check stdout to decide on colors/formatting
- Check stderr independently if it has different formatting needs

## Stderr for Diagnostics

Use stderr for all non-data output: logs, progress, errors, and metadata. This keeps stdout clean for actual program output that can be piped.

```rust
// Logs and diagnostics go to stderr
eprintln!("Processing {} files...", count);
eprintln!("[INFO] Connection established");
eprintln!("[ERROR] Failed to parse config: {}", err);

// Only actual data goes to stdout
println!("{}", result);
```

When to use stderr:
- Error messages and warnings
- Progress updates and status information
- Debug and verbose output
- Metadata about the operation
- Any output that is not the "result" of the command

## Synchronous Prompts

Use synchronous I/O for interactive prompts. This blocks the thread but provides predictable behavior for user input.

```rust
use std::io::{self, Write};

fn prompt(message: &str) -> Option<String> {
    print!("{}", message);
    io::stdout().flush().ok(); // Flush before blocking read
    
    let mut input = String::new();
    match io::stdin().read_line(&mut input) {
        Ok(0) => None, // EOF
        Ok(_) => Some(input.trim().to_string()),
        Err(_) => None,
    }
}

// Usage
if let Some(answer) = prompt("Continue? [y/N]: ") {
    if answer.to_lowercase() == "y" {
        // proceed
    }
}
```

Key points:
- Always flush stdout before reading stdin (print! doesn't auto-flush)
- Handle EOF (Ctrl+D) gracefully by checking for 0 bytes read
- Trim input to remove trailing newline

## Progress Indicators

Use carriage return (`\r`) for in-place updates. Always flush after partial line writes.

```rust
use std::io::{self, Write};

fn show_progress(current: usize, total: usize) {
    let percentage = (current * 100) / total;
    print!("\rProcessing: {}% ({}/{})", percentage, current, total);
    io::stdout().flush().ok();
}

fn clear_progress() {
    print!("\r\x1b[K"); // Move to start, clear line
    io::stdout().flush().ok();
}
```

For longer operations with async code:

```rust
use tokio::io::{AsyncWriteExt, stdout};

async fn async_progress(message: &str) {
    let mut stdout = stdout();
    stdout.write_all(format!("\r{}", message).as_bytes()).await.ok();
    stdout.flush().await.ok();
}
```

Key points:
- `\r` returns cursor to line start without newline
- `\x1b[K` clears from cursor to end of line (ANSI escape)
- Always flush immediately after progress updates
- Clear progress line before printing final output
- Only use progress indicators when stdout is a terminal

## Prelude Re-exports for Consistent Output

Re-export print macros from `anstream` for automatic color/formatting handling based on terminal detection.

```rust
// In src/prelude.rs or similar
pub use anstream::{println, eprintln, print, eprint};
```

Usage in other modules:

```rust
use crate::prelude::*;

fn output_result(data: &str) {
    // Automatically handles color stripping when piped
    println!("{}", data);
}
```

This approach:
- Centralizes output behavior configuration
- Automatically strips ANSI codes when output is piped
- Provides consistent behavior across the codebase
- Makes it easy to swap output implementations

## Combined Example

A complete pattern showing all concepts together:

```rust
use std::io::IsTerminal;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

async fn process_input() -> Result<(), Box<dyn std::error::Error>> {
    let interactive = std::io::stdin().is_terminal();
    
    if interactive {
        // Interactive mode: prompt for input
        eprintln!("Enter lines (Ctrl+D to finish):");
    }
    
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut stdout = tokio::io::stdout();
    let mut line = String::new();
    let mut count = 0;
    
    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line).await?;
        if bytes_read == 0 {
            break;
        }
        
        count += 1;
        
        // Progress to stderr (only if terminal)
        if interactive {
            eprintln!("Processed line {}", count);
        }
        
        // Actual output to stdout
        let processed = line.to_uppercase();
        stdout.write_all(processed.as_bytes()).await?;
    }
    
    stdout.flush().await?;
    eprintln!("Done: {} lines processed", count);
    
    Ok(())
}
```

## Summary Table

| Pattern | When to Use | Key Considerations |
|---------|-------------|-------------------|
| Async stdin reading | Processing piped input, concurrent operations | Clear buffer each iteration, check for EOF |
| Async stdout writing | Non-blocking output, high-throughput data | Always flush, use write_all |
| Terminal detection | Conditional formatting, colors | Check each stream independently |
| Stderr diagnostics | Logs, errors, progress, metadata | Keeps stdout clean for piping |
| Sync prompts | Interactive user input | Flush before read, handle EOF |
| Progress indicators | Long operations in terminals | Use `\r`, always flush, clear before final output |
| Prelude re-exports | Consistent output across codebase | Centralized configuration, auto color handling |

## Decision Guide for AI Agents

1. **Reading input?**
   - Piped/file input -> Async stdin with tokio
   - Interactive prompt -> Sync stdin with flush

2. **Writing output?**
   - Program results/data -> stdout
   - Everything else -> stderr

3. **Formatting output?**
   - Check `is_terminal()` first
   - Terminal -> colors, progress, rich formatting
   - Piped -> plain text, machine-parseable

4. **Long operation?**
   - Show progress on stderr
   - Use `\r` for in-place updates
   - Only when stderr is terminal
