# CLI/Main/Run Modules Simplification Plan

> **For Claude:** REQUIRED SUB-SKILL: Use executing-plans to implement this plan task-by-task.

**Goal:** Remove dead code from the subprocess module, consolidate duplicated error recovery logic, and simplify the prompt resolution functions following the "No Dead Code" rule.

**Architecture:** This refactoring follows Functional Core - Imperative Shell. Pure functions (error message formatting, prompt source resolution) go in functional core modules, while I/O operations (reading files, stdin) remain in the shell. We extract reusable helpers to reduce code duplication while maintaining testability.

**Tech Stack:** Rust, thiserror, chrono, std::io

---

## Phase 1: Remove Dead Code from Subprocess Module (High Priority)

### Task 1.1: Remove unused `basic.rs` file

**File:** `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/subprocess/basic.rs`

**Why:** The `invoke_subprocess` function is not called anywhere in the codebase. It's only re-exported but never used. The codebase uses the more advanced `invoke_subprocess_with_timeout` and `invoke_subprocess_with_spinner_config` functions.

**Steps:**

1. Delete the file `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/subprocess/basic.rs`

2. Update `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/subprocess/mod.rs`:
   - Remove line: `mod basic;`
   - Remove line: `pub use basic::invoke_subprocess;`

3. Run `cargo check --package ralph` to verify no compilation errors

**Expected output:** Compilation succeeds with no errors about missing `invoke_subprocess`.

---

### Task 1.2: Remove unused `streaming.rs` file

**File:** `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/subprocess/streaming.rs`

**Why:** The `invoke_subprocess_with_stream_processing` function is not called anywhere. The codebase uses `invoke_subprocess_with_timeout` which has the same stream processing capability plus timeout support.

**Steps:**

1. Delete the file `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/subprocess/streaming.rs`

2. Update `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/subprocess/mod.rs`:
   - Remove line: `mod streaming;`
   - Remove line: `pub use streaming::invoke_subprocess_with_stream_processing;`
   - Update the module-level doc comment to remove reference to `invoke_subprocess_with_stream_processing`

3. Run `cargo check --package ralph` to verify no compilation errors

---

### Task 1.3: Remove unused `themed.rs` file

**File:** `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/subprocess/themed.rs`

**Why:** The `invoke_subprocess_with_theme` function is not called anywhere. The codebase uses `invoke_subprocess_with_spinner_config` which includes theme support plus spinner functionality.

**Steps:**

1. Delete the file `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/subprocess/themed.rs`

2. Update `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/subprocess/mod.rs`:
   - Remove line: `mod themed;`
   - Remove line: `pub use themed::invoke_subprocess_with_theme;`
   - Update the module-level doc comment to remove reference to `invoke_subprocess_with_theme`

3. Run `cargo check --package ralph` to verify no compilation errors

---

### Task 1.4: Remove unused `SubprocessResult` type

**File:** `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/subprocess/types.rs`

**Why:** After removing `basic.rs`, the `SubprocessResult` type (lines 12-20) is no longer used. Only `StreamingSubprocessResult` is needed.

**Steps:**

1. Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/subprocess/types.rs`:
   - Remove the `SubprocessResult` struct (lines 11-20):
     ```rust
     /// Result of a subprocess invocation.
     #[derive(Debug, Clone)]
     pub struct SubprocessResult {
         /// The exit code from the subprocess.
         pub exit_code: i32,
         /// Captured stdout output.
         pub stdout: String,
         /// Captured stderr output.
         pub stderr: String,
     }
     ```

2. Update `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/subprocess/mod.rs`:
   - Change line: `pub use types::{StreamingSubprocessResult, SubprocessError, SubprocessResult};`
   - To: `pub use types::{StreamingSubprocessResult, SubprocessError};`

3. Run `cargo check --package ralph` to verify no compilation errors

---

### Task 1.5: Update subprocess module documentation

**File:** `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/subprocess/mod.rs`

**Why:** After removing dead code, the module documentation lists functions that no longer exist.

**Steps:**

1. Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/subprocess/mod.rs`:
   - Update the module-level doc comment from:
     ```rust
     //! # Progressive Composition
     //!
     //! The module provides multiple invocation variants with increasing functionality:
     //!
     //! 1. [`invoke_subprocess`] - Basic subprocess with line-by-line streaming
     //! 2. [`invoke_subprocess_with_stream_processing`] - Adds JSON parsing and highlighting
     //! 3. [`invoke_subprocess_with_timeout`] - Adds timeout enforcement
     //! 4. [`invoke_subprocess_with_theme`] - Adds custom theme configuration
     //! 5. [`invoke_subprocess_with_spinner_config`] - Adds spinner display with session info
     ```
   - To:
     ```rust
     //! # Available Functions
     //!
     //! The module provides subprocess invocation variants:
     //!
     //! 1. [`invoke_subprocess_with_timeout`] - Stream processing with timeout enforcement
     //! 2. [`invoke_subprocess_with_spinner_config`] - Adds spinner display with session info and theme support
     ```

2. Run `cargo doc --package ralph --no-deps` to verify documentation generates correctly

---

### Task 1.6: Run full test suite after Phase 1

**Why:** Verify all tests still pass after removing dead code.

**Steps:**

1. Run `cargo test --package ralph`

2. Run `cargo clippy --package ralph -- -D warnings`

3. Run `cargo fmt --package ralph --check`

**Expected output:** All tests pass, no clippy warnings, code is formatted.

---

## Phase 2: Consolidate Duplicated Error Recovery Logic (Medium Priority)

### Task 2.1: Create `FailureRecoveryContext` struct and helper function

**File:** `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/main.rs`

**Why:** Lines 203-318 contain duplicated logic for handling `SubprocessFailed` and `SubprocessTimedOut` errors. Both branches have nearly identical code for:
- Tracking session slug
- Prompting user
- Handling Retry/Abort/None cases
- Finalizing session with different outcomes

**Implementation:**

1. Add a new struct and function in `main.rs` after the existing imports (around line 44):

```rust
/// Context for handling subprocess failure recovery.
struct FailureRecoveryContext {
    /// Summary message to display to user.
    summary: String,
    /// Session slug for this run.
    session_slug: String,
    /// Iterations completed before failure.
    iterations_completed: usize,
    /// Total iterations from previous recovery attempts.
    total_iterations_completed: usize,
}

/// Result of failure recovery handling.
enum FailureRecoveryResult {
    /// User chose to retry - continue the loop.
    Retry {
        updated_total_iterations: usize,
    },
    /// Recovery was aborted (user chose or non-interactive).
    Aborted(Box<dyn std::error::Error>),
}

/// Handle subprocess failure with user prompting and session finalization.
///
/// This is a pure-ish function that handles the common failure recovery pattern:
/// 1. Update current_session_slug tracking
/// 2. Prompt user (if interactive)
/// 3. Handle Retry/Abort/None responses
/// 4. Finalize session appropriately
///
/// Returns `FailureRecoveryResult::Retry` if user wants to continue,
/// or `FailureRecoveryResult::Aborted` with the error to return.
fn handle_failure_recovery(
    ctx: &FailureRecoveryContext,
    current_session_slug: &mut Option<String>,
) -> FailureRecoveryResult {
    // Track the session slug for potential recovery
    if current_session_slug.is_none() {
        *current_session_slug = Some(ctx.session_slug.clone());
    }

    match prompt_on_failure(&ctx.summary) {
        Some(FailureAction::Retry) => {
            // Continue the same session - don't finalize, just accumulate iterations
            let updated = ctx.total_iterations_completed + ctx.iterations_completed;
            eprintln!("\nContinuing run (session '{}')...\n", ctx.session_slug);
            FailureRecoveryResult::Retry {
                updated_total_iterations: updated,
            }
        }
        Some(FailureAction::Abort) => {
            // User chose to abort - finalize session as aborted
            let final_iterations = ctx.total_iterations_completed + ctx.iterations_completed;
            if let Err(e) = session::finalize_session(
                &ctx.session_slug,
                final_iterations as u32,
                SessionOutcome::Aborted,
            ) {
                eprintln!("Warning: Failed to finalize session: {}", e);
            }
            FailureRecoveryResult::Aborted("Aborted by user".into())
        }
        None => {
            // Non-interactive mode or EOF - finalize as failed and abort
            let final_iterations = ctx.total_iterations_completed + ctx.iterations_completed;
            if let Err(e) = session::finalize_session(
                &ctx.session_slug,
                final_iterations as u32,
                SessionOutcome::Failed,
            ) {
                eprintln!("Warning: Failed to finalize session: {}", e);
            }
            eprintln!("Non-interactive mode - aborting.");
            FailureRecoveryResult::Aborted(ctx.summary.clone().into())
        }
    }
}
```

2. Run `cargo check --package ralph` to verify the new code compiles

---

### Task 2.2: Refactor SubprocessFailed handling to use helper

**File:** `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/main.rs`

**Steps:**

1. Replace the `RunError::SubprocessFailed` match arm (lines 203-260) with:

```rust
Err(RunError::SubprocessFailed {
    exit_code,
    attempts,
    raw_text: _,
    stderr: _,
    session_slug,
    iterations_completed,
}) => {
    let ctx = FailureRecoveryContext {
        summary: format!(
            "LLM subprocess failed with exit code {} after {} attempt(s).",
            exit_code, attempts
        ),
        session_slug,
        iterations_completed,
        total_iterations_completed,
    };

    match handle_failure_recovery(&ctx, &mut current_session_slug) {
        FailureRecoveryResult::Retry { updated_total_iterations } => {
            total_iterations_completed = updated_total_iterations;
            continue;
        }
        FailureRecoveryResult::Aborted(err) => return Err(err),
    }
}
```

2. Run `cargo check --package ralph` to verify compilation

---

### Task 2.3: Refactor SubprocessTimedOut handling to use helper

**File:** `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/main.rs`

**Steps:**

1. Replace the `RunError::SubprocessTimedOut` match arm (lines 261-318) with:

```rust
Err(RunError::SubprocessTimedOut {
    timeout_secs,
    attempts,
    raw_text: _,
    stderr: _,
    session_slug,
    iterations_completed,
}) => {
    let ctx = FailureRecoveryContext {
        summary: format!(
            "LLM subprocess timed out after {} seconds ({} attempt(s)).",
            timeout_secs, attempts
        ),
        session_slug,
        iterations_completed,
        total_iterations_completed,
    };

    match handle_failure_recovery(&ctx, &mut current_session_slug) {
        FailureRecoveryResult::Retry { updated_total_iterations } => {
            total_iterations_completed = updated_total_iterations;
            continue;
        }
        FailureRecoveryResult::Aborted(err) => return Err(err),
    }
}
```

2. Run `cargo test --package ralph` to verify all tests still pass

---

### Task 2.4: Write unit test for handle_failure_recovery

**File:** `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/main.rs`

**Why:** The new helper function needs test coverage.

**Note:** Testing `handle_failure_recovery` directly is challenging because it calls `prompt_on_failure` which reads from stdin. However, we can test the pure logic aspects.

**Steps:**

1. Add tests to the existing `#[cfg(test)]` module at the bottom of main.rs:

```rust
#[test]
fn test_failure_recovery_context_creation() {
    let ctx = FailureRecoveryContext {
        summary: "Test failure".to_string(),
        session_slug: "test-session".to_string(),
        iterations_completed: 3,
        total_iterations_completed: 5,
    };

    assert_eq!(ctx.summary, "Test failure");
    assert_eq!(ctx.session_slug, "test-session");
    assert_eq!(ctx.iterations_completed, 3);
    assert_eq!(ctx.total_iterations_completed, 5);
}
```

2. Run `cargo test --package ralph handle_failure_recovery` to verify the test passes

---

## Phase 3: Consolidate Prompt Resolution Functions (Medium Priority)

### Task 3.1: Create generic `resolve_from_source` function

**File:** `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/main.rs`

**Why:** `resolve_prompt` (lines 478-515) and `resolve_additional_prompt` (lines 524-549) have nearly identical logic for handling `-` (stdin), file paths, and inline strings. The same pattern appears in `build_summarize_config` (lines 574-596).

**Implementation:**

1. Add a new helper function before `resolve_prompt`:

```rust
/// Input source for prompt resolution.
#[derive(Debug, Clone, PartialEq)]
enum PromptSource<'a> {
    /// Read from stdin (when arg is "-")
    Stdin,
    /// Read from file at path
    File(&'a Path),
    /// Use inline string directly
    Inline(&'a str),
    /// No input provided
    None,
}

/// Classify the input argument into a source type.
///
/// This is a pure function that determines how to interpret the argument:
/// - "-" means stdin
/// - An existing file path means read from file
/// - Any other string is treated as inline content
/// - None means no input
fn classify_prompt_source(arg: Option<&str>) -> PromptSource<'_> {
    match arg {
        Some("-") => PromptSource::Stdin,
        Some(value) => {
            let path = Path::new(value);
            if path.exists() && path.is_file() {
                PromptSource::File(path)
            } else {
                PromptSource::Inline(value)
            }
        }
        None => PromptSource::None,
    }
}

/// Read content from a prompt source.
///
/// This is the imperative shell that performs actual I/O based on the source type.
fn read_from_source(
    source: PromptSource<'_>,
    default: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    match source {
        PromptSource::Stdin => {
            use std::io::Read;
            let mut content = String::new();
            std::io::stdin().read_to_string(&mut content)?;
            Ok(content)
        }
        PromptSource::File(path) => Ok(std::fs::read_to_string(path)?),
        PromptSource::Inline(value) => Ok(value.to_string()),
        PromptSource::None => Ok(default.unwrap_or("").to_string()),
    }
}
```

2. Run `cargo check --package ralph` to verify compilation

---

### Task 3.2: Refactor resolve_additional_prompt to use helpers

**File:** `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/main.rs`

**Steps:**

1. Replace `resolve_additional_prompt` with:

```rust
/// Resolve additional prompt from various sources.
///
/// Loads additional prompt instructions from:
/// - A file path (if the argument is a path to an existing file)
/// - Stdin (if the argument is "-")
/// - An inline string (if the argument doesn't match a file)
/// - Empty string (if no argument is provided)
fn resolve_additional_prompt(
    additional_arg: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let source = classify_prompt_source(additional_arg);
    read_from_source(source, None)
}
```

2. Run `cargo test --package ralph` to verify existing tests still pass

---

### Task 3.3: Refactor resolve_prompt to use helpers

**File:** `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/main.rs`

**Steps:**

1. Replace `resolve_prompt` with:

```rust
/// Resolve the prompt from various sources.
///
/// Loads the prompt template from one of three sources:
/// - A file path (if the argument is a path to an existing file)
/// - Stdin (if the argument is "-")
/// - An inline string (if the argument doesn't match a file)
/// - The default template (if no argument is provided)
///
/// After loading the template, placeholders are substituted with actual values:
/// - `{design_file}` - Path to the design document
/// - `{prd_file}` - Path to the PRD file
/// - `{progress_file}` - Path to the progress notes file
/// - `{completion_marker}` - The completion marker string
/// - `{additional_prompt}` - Additional instructions appended to the prompt
fn resolve_prompt(
    prompt_arg: Option<&str>,
    context_paths: &ContextPaths,
    completion_marker: &str,
    additional_prompt: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let source = classify_prompt_source(prompt_arg);
    let template = read_from_source(source, Some(defaults::PROMPT_TEMPLATE))?;

    // Substitute placeholders in the template
    Ok(substitute_template_placeholders(
        &template,
        context_paths,
        completion_marker,
        additional_prompt,
    ))
}
```

2. Run `cargo test --package ralph` to verify existing tests still pass

---

### Task 3.4: ~~Refactor build_summarize_config to use helpers~~ (COMPLETED)

**Status:** OBSOLETE - The `build_summarize_config` function and all summarization-related CLI flags were removed in the "Remove --progress and summarization CLI flags" refactoring. See Story 4 in the simplified context model PRD.

---

### Task 3.5: Write unit tests for prompt source classification

**File:** `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/main.rs`

**Steps:**

1. Add tests to the existing `#[cfg(test)]` module:

```rust
#[test]
fn test_classify_prompt_source_stdin() {
    assert_eq!(classify_prompt_source(Some("-")), PromptSource::Stdin);
}

#[test]
fn test_classify_prompt_source_none() {
    assert_eq!(classify_prompt_source(None), PromptSource::None);
}

#[test]
fn test_classify_prompt_source_inline() {
    // Non-existent path should be treated as inline
    assert_eq!(
        classify_prompt_source(Some("inline content")),
        PromptSource::Inline("inline content")
    );
}

#[test]
fn test_classify_prompt_source_file() {
    // Use Cargo.toml as a file that definitely exists
    let source = classify_prompt_source(Some("Cargo.toml"));
    assert!(matches!(source, PromptSource::File(_)));
}
```

2. Run `cargo test --package ralph classify_prompt_source` to verify tests pass

---

## Phase 4: Final Verification

### Task 4.1: Run complete test suite

**Steps:**

1. Run `cargo test --workspace`

2. Run `cargo clippy --workspace -- -D warnings`

3. Run `cargo fmt --all --check`

**Expected output:** All tests pass, no clippy warnings, code is formatted.

---

### Task 4.2: Verify documentation builds

**Steps:**

1. Run `cargo doc --workspace --no-deps`

2. Open `target/doc/ralph/index.html` and verify subprocess module documentation is correct

---

### Task 4.3: Manual smoke test

**Steps:**

1. Run `cargo build --release`

2. Run `./target/release/ralph --help` to verify CLI works

3. Run `./target/release/ralph themes` to verify a basic command works

---

## Summary of Changes

### Files to Delete
- `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/subprocess/basic.rs`
- `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/subprocess/streaming.rs`
- `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/subprocess/themed.rs`

### Files to Modify
- `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/subprocess/mod.rs` - Remove dead exports and update docs
- `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/subprocess/types.rs` - Remove `SubprocessResult`
- `/Users/guzmanmonne/Projects/Rust/ralph/crates/ralph/src/main.rs` - Add helpers and consolidate logic

### Estimated Lines Removed
- ~107 lines from `basic.rs`
- ~136 lines from `streaming.rs`
- ~203 lines from `themed.rs`
- ~10 lines from `types.rs` (SubprocessResult)
- ~58 lines duplication in error recovery (net reduction after helper)
- ~50 lines duplication in prompt resolution (net reduction after helper)

**Total estimated reduction:** ~500+ lines of code

### New Code Added
- `FailureRecoveryContext` struct (~10 lines)
- `FailureRecoveryResult` enum (~10 lines)
- `handle_failure_recovery` function (~35 lines)
- `PromptSource` enum (~10 lines)
- `classify_prompt_source` function (~15 lines)
- `read_from_source` function (~20 lines)
- Unit tests (~40 lines)

**Total new code:** ~140 lines

**Net reduction:** ~360+ lines

---

## Deferred Items (Low Priority)

The following items from the original findings are deferred for future work:

### RunConfig Field Grouping
The `RunConfig` struct has 18 fields. While this could be improved by grouping related fields (e.g., `CustomizationFlags`, `DisplayOptions`), this is a larger refactoring that should be done as a separate effort when there's a need to add more configuration.

### Session Finalization Pattern
There are 5 occurrences of session finalization with similar patterns. After Phase 2, the main.rs patterns are consolidated. The one in `run/mod.rs` (lines 562-567 and 575-581) could be further consolidated, but the context is different enough (error vs success paths) that the duplication is acceptable.

---

**Plan complete and saved to `docs/plans/cli-main-simplification.md`. Two execution options:**

**1. Subagent-Driven (this session)** - I dispatch fresh subagent per task, review between tasks, fast iteration

**2. Parallel Session (separate)** - Open new session with executing-plans, batch execution with checkpoints

**Which approach?**
