# Core Crate Simplification Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use executing-plans to implement this plan task-by-task.

**Goal:** Simplify and clean up the core crate by extracting helper functions, removing dead code, adding missing tests, and replacing magic numbers with named constants.

**Architecture:** This refactoring follows the Functional Core - Imperative Shell pattern. We will extract pure helper functions from large methods, maintain existing public APIs, and ensure all new pure functions have comprehensive unit tests.

**Tech Stack:** Rust, cargo test

---

## Task 1: Remove Empty Impl Block (Dead Code)

**Priority:** Low
**File:** `/Users/guzmanmonne/Projects/Rust/ralph/crates/core/src/stream/accumulation.rs`
**Lines:** 229-232

The empty impl block for `AssistantEvent` is dead code and should be removed per project rules.

### Step 1.1: Verify the dead code exists

Open the file and confirm the empty impl block:

```rust
// Re-export AssistantEvent methods that use ToolInvocation
impl AssistantEvent {
    // Note: extract_text and extract_tool_invocations are implemented in extraction.rs
}
```

**Expected:** Lines 229-232 contain an empty impl block with only a comment.

### Step 1.2: Remove the dead code

Delete lines 229-232 from `/Users/guzmanmonne/Projects/Rust/ralph/crates/core/src/stream/accumulation.rs`.

The file should end after line 227 (`accumulator.get_all_text()`).

### Step 1.3: Run tests to verify no regression

```bash
cargo test -p ralph-core
```

**Expected:** All tests pass. The empty impl block had no functionality.

---

## Task 2: Extract Magic Number as Named Constant

**Priority:** Low
**File:** `/Users/guzmanmonne/Projects/Rust/ralph/crates/core/src/chunk/heuristics.rs`
**Line:** 63

The magic number `2` represents the minimum number of diff lines required for heuristic detection.

### Step 2.1: Add named constant at top of file

Add after line 4 (after the imports):

```rust
/// Minimum number of diff-like lines required to classify text as an unfenced diff.
/// Lines starting with `+` or `-` (excluding `++`/`--` headers) are counted.
const MIN_DIFF_LINES_THRESHOLD: usize = 2;
```

### Step 2.2: Replace magic number with constant

Change line 63 from:

```rust
    diff_line_count >= 2
```

To:

```rust
    diff_line_count >= MIN_DIFF_LINES_THRESHOLD
```

### Step 2.3: Run tests to verify no regression

```bash
cargo test -p ralph-core
```

**Expected:** All tests pass. The behavior is unchanged.

---

## Task 3: Add Unit Tests for Fence Module

**Priority:** Low
**File to create:** `/Users/guzmanmonne/Projects/Rust/ralph/crates/core/src/chunk/tests/fence.rs`
**Module to update:** `/Users/guzmanmonne/Projects/Rust/ralph/crates/core/src/chunk/tests/mod.rs`

The fence module (`/Users/guzmanmonne/Projects/Rust/ralph/crates/core/src/chunk/fence.rs`) has two functions that need direct unit tests:
- `parse_fence_open(line: &str) -> Option<Option<String>>`
- `is_fence_close(line: &str) -> bool`

Note: These functions are `pub(crate)`, so tests must be in the same crate.

### Step 3.1: Create the fence test file

Create `/Users/guzmanmonne/Projects/Rust/ralph/crates/core/src/chunk/tests/fence.rs`:

```rust
//! Tests for fence detection utilities.

use crate::chunk::fence::{is_fence_close, parse_fence_open};

// =============================================================================
// parse_fence_open tests
// =============================================================================

#[test]
fn parse_fence_open_with_language() {
    let result = parse_fence_open("```rust");
    assert_eq!(result, Some(Some("rust".to_string())));
}

#[test]
fn parse_fence_open_with_language_and_whitespace() {
    let result = parse_fence_open("```python  ");
    assert_eq!(result, Some(Some("python".to_string())));
}

#[test]
fn parse_fence_open_bare_fence() {
    let result = parse_fence_open("```");
    assert_eq!(result, Some(None));
}

#[test]
fn parse_fence_open_bare_fence_with_trailing_whitespace() {
    let result = parse_fence_open("```   ");
    assert_eq!(result, Some(None));
}

#[test]
fn parse_fence_open_with_leading_whitespace() {
    let result = parse_fence_open("   ```javascript");
    assert_eq!(result, Some(Some("javascript".to_string())));
}

#[test]
fn parse_fence_open_not_a_fence() {
    let result = parse_fence_open("regular text");
    assert_eq!(result, None);
}

#[test]
fn parse_fence_open_partial_fence() {
    let result = parse_fence_open("``");
    assert_eq!(result, None);
}

#[test]
fn parse_fence_open_fence_in_text() {
    // Fence must be at the start (after optional whitespace)
    let result = parse_fence_open("text ```rust");
    assert_eq!(result, None);
}

#[test]
fn parse_fence_open_diff_language() {
    let result = parse_fence_open("```diff");
    assert_eq!(result, Some(Some("diff".to_string())));
}

#[test]
fn parse_fence_open_language_with_extra_words() {
    // Only first word is captured as language
    let result = parse_fence_open("```rust fn main");
    assert_eq!(result, Some(Some("rust".to_string())));
}

// =============================================================================
// is_fence_close tests
// =============================================================================

#[test]
fn is_fence_close_simple() {
    assert!(is_fence_close("```"));
}

#[test]
fn is_fence_close_with_surrounding_whitespace() {
    assert!(is_fence_close("  ```  "));
}

#[test]
fn is_fence_close_not_a_close() {
    assert!(!is_fence_close("```rust"));
}

#[test]
fn is_fence_close_partial() {
    assert!(!is_fence_close("``"));
}

#[test]
fn is_fence_close_empty_line() {
    assert!(!is_fence_close(""));
}

#[test]
fn is_fence_close_text_after_fence() {
    // Close fence must be exactly ``` (trimmed)
    assert!(!is_fence_close("``` text"));
}

#[test]
fn is_fence_close_fence_in_text() {
    assert!(!is_fence_close("text ```"));
}
```

### Step 3.2: Update the test module to include fence tests

Edit `/Users/guzmanmonne/Projects/Rust/ralph/crates/core/src/chunk/tests/mod.rs` to add `mod fence;`:

```rust
//! Tests for chunk parsing functionality.

mod batch;
mod fence;
mod split_lines;
mod streaming_core;
mod streaming_threshold;
mod streaming_whitespace;
```

### Step 3.3: Make fence module functions visible to tests

The fence module uses `pub(crate)` visibility, which should already be accessible from the tests module since they're in the same crate. Verify by running tests.

### Step 3.4: Run tests to verify fence tests pass

```bash
cargo test -p ralph-core fence
```

**Expected:** All new fence tests pass.

---

## Task 4: Refactor `parse_chunks()` with Helper Functions

**Priority:** Low
**File:** `/Users/guzmanmonne/Projects/Rust/ralph/crates/core/src/chunk/batch.rs`
**Lines:** 31-98 (~67 lines)

Extract the code block handling logic into pure helper functions.

### Step 4.1: Add helper function for flushing prose

Add after line 4 (after imports), before `parse_chunks`:

```rust
/// Flush accumulated prose into the chunks vector if non-empty.
///
/// This is a pure helper that takes ownership of the prose content.
fn flush_prose(chunks: &mut Vec<ParsedChunk>, prose: &mut String) {
    if !prose.is_empty() {
        chunks.push(ParsedChunk::prose(std::mem::take(prose)));
    }
}
```

### Step 4.2: Add helper function for emitting code block

Add after the `flush_prose` function:

```rust
/// Emit a completed code block chunk.
///
/// This is a pure helper that takes ownership of the code content.
fn emit_code_block(
    chunks: &mut Vec<ParsedChunk>,
    content: &mut String,
    language: &mut Option<String>,
    is_diff: bool,
) {
    let code_content = std::mem::take(content);
    if is_diff {
        chunks.push(ParsedChunk::diff(code_content));
    } else {
        chunks.push(ParsedChunk::code(code_content, language.take()));
    }
}
```

### Step 4.3: Add helper function for appending line to buffer

Add after the `emit_code_block` function:

```rust
/// Append a line to a buffer, adding newline separator if buffer is non-empty.
fn append_line(buffer: &mut String, line: &str) {
    if !buffer.is_empty() {
        buffer.push('\n');
    }
    buffer.push_str(line);
}
```

### Step 4.4: Refactor parse_chunks to use helpers

Replace the `parse_chunks` function body (lines 31-98) with:

```rust
pub fn parse_chunks(text: &str) -> Vec<ParsedChunk> {
    let mut chunks = Vec::new();
    let mut current_prose = String::new();
    let mut in_code_block = false;
    let mut code_block_content = String::new();
    let mut code_block_language: Option<String> = None;
    let mut is_diff_block = false;

    for line in text.lines() {
        if !in_code_block {
            if let Some(lang) = parse_fence_open(line) {
                flush_prose(&mut chunks, &mut current_prose);
                in_code_block = true;
                is_diff_block = lang.as_deref() == Some("diff");
                code_block_language = lang;
                code_block_content.clear();
            } else {
                append_line(&mut current_prose, line);
            }
        } else if is_fence_close(line) {
            emit_code_block(
                &mut chunks,
                &mut code_block_content,
                &mut code_block_language,
                is_diff_block,
            );
            in_code_block = false;
            is_diff_block = false;
        } else {
            append_line(&mut code_block_content, line);
        }
    }

    // Handle unterminated code block
    if in_code_block && !code_block_content.is_empty() {
        emit_code_block(
            &mut chunks,
            &mut code_block_content,
            &mut code_block_language,
            is_diff_block,
        );
    }

    // Flush remaining prose
    flush_prose(&mut chunks, &mut current_prose);

    chunks
}
```

### Step 4.5: Run tests to verify no regression

```bash
cargo test -p ralph-core batch
```

**Expected:** All batch tests pass. The refactoring preserves behavior.

### Step 4.6: Add unit tests for the new helper functions

Add to `/Users/guzmanmonne/Projects/Rust/ralph/crates/core/src/chunk/tests/batch.rs` at the end of the file:

```rust
// =============================================================================
// Helper function tests
// =============================================================================

mod helpers {
    use crate::chunk::{ChunkType, ParsedChunk};

    // We test the helpers indirectly through parse_chunks behavior
    // since they are private functions

    #[test]
    fn flush_prose_preserves_content() {
        // Verified through existing test_parse_simple_prose
        let chunks = super::parse_chunks("Hello\nWorld");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, "Hello\nWorld");
    }

    #[test]
    fn append_line_joins_with_newline() {
        // Verified through multi-line prose handling
        let chunks = super::parse_chunks("Line1\nLine2\nLine3");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, "Line1\nLine2\nLine3");
    }

    #[test]
    fn emit_code_block_handles_diff() {
        let chunks = super::parse_chunks("```diff\n-old\n+new\n```");
        assert_eq!(chunks.len(), 1);
        assert!(matches!(chunks[0].chunk_type, ChunkType::Diff));
    }

    #[test]
    fn emit_code_block_handles_language() {
        let chunks = super::parse_chunks("```typescript\nconst x = 1;\n```");
        assert_eq!(chunks.len(), 1);
        match &chunks[0].chunk_type {
            ChunkType::Code { language } => {
                assert_eq!(language.as_deref(), Some("typescript"));
            }
            _ => panic!("Expected code chunk"),
        }
    }
}
```

### Step 4.7: Run all tests

```bash
cargo test -p ralph-core
```

**Expected:** All tests pass.

---

## Task 5: Refactor `StreamingChunkBuffer::process_line()` with Helper Functions

**Priority:** Medium
**File:** `/Users/guzmanmonne/Projects/Rust/ralph/crates/core/src/chunk/streaming.rs`
**Lines:** 307-377 (~70 lines)

Extract helper functions for each match arm following the Functional Core pattern.

### Step 5.1: Add helper for handling prose line in eager mode

Add as a private method in the `impl StreamingChunkBuffer` block, after `flush_prose_buffer` (around line 392):

```rust
    /// Handle a prose line in eager mode (threshold = 0).
    ///
    /// Returns a single-element vector with the prose chunk.
    fn emit_eager_prose(&mut self, line: &str) -> Vec<ParsedChunk> {
        let chunk = ParsedChunk::prose(line);
        self.emitted_count += 1;
        vec![chunk]
    }
```

### Step 5.2: Add helper for handling prose line in buffered mode

Add after `emit_eager_prose`:

```rust
    /// Handle a prose line in buffered mode (threshold > 0).
    ///
    /// Accumulates the line and returns chunks if threshold is reached.
    fn accumulate_prose_line(&mut self, line: &str) -> Vec<ParsedChunk> {
        if !self.buffer.is_empty() {
            self.buffer.push('\n');
        }
        self.buffer.push_str(line);
        self.buffered_prose_lines += 1;

        if self.buffered_prose_lines >= self.prose_buffer_threshold {
            self.flush_prose_buffer().into_iter().collect()
        } else {
            Vec::new()
        }
    }
```

### Step 5.3: Add helper for starting a code block

Add after `accumulate_prose_line`:

```rust
    /// Start a new code block, returning any flushed prose.
    fn start_code_block(&mut self, language: Option<String>) -> Vec<ParsedChunk> {
        let mut result = Vec::new();

        // Flush any buffered prose before starting a code block
        if let Some(prose_chunk) = self.flush_prose_buffer() {
            result.push(prose_chunk);
        }

        let is_diff = language.as_deref() == Some("diff");
        self.state = BufferState::Code { language, is_diff };
        self.buffer.clear();

        result
    }
```

### Step 5.4: Add helper for closing a code block

Add after `start_code_block`:

```rust
    /// Close the current code block and emit the chunk.
    fn close_code_block(&mut self) -> Vec<ParsedChunk> {
        let (language, is_diff) = match &self.state {
            BufferState::Code { language, is_diff } => (language.clone(), *is_diff),
            BufferState::Prose => return Vec::new(),
        };

        let content = std::mem::take(&mut self.buffer);
        let chunk = if is_diff {
            ParsedChunk::diff(content)
        } else {
            ParsedChunk::code(content, language)
        };
        self.emitted_count += 1;
        self.state = BufferState::Prose;

        vec![chunk]
    }
```

### Step 5.5: Add helper for accumulating code content

Add after `close_code_block`:

```rust
    /// Accumulate a line of code content.
    fn accumulate_code_line(&mut self, line: &str) {
        if !self.buffer.is_empty() {
            self.buffer.push('\n');
        }
        self.buffer.push_str(line);
    }
```

### Step 5.6: Refactor process_line to use helpers

Replace the `process_line` method body (keeping the signature and docs):

```rust
    pub fn process_line(&mut self, line: &str) -> Vec<ParsedChunk> {
        match &self.state {
            BufferState::Prose => {
                if let Some(lang) = parse_fence_open(line) {
                    self.start_code_block(lang)
                } else if self.prose_buffer_threshold == 0 {
                    self.emit_eager_prose(line)
                } else {
                    self.accumulate_prose_line(line)
                }
            }
            BufferState::Code { .. } => {
                if is_fence_close(line) {
                    self.close_code_block()
                } else {
                    self.accumulate_code_line(line);
                    Vec::new()
                }
            }
        }
    }
```

### Step 5.7: Run tests to verify no regression

```bash
cargo test -p ralph-core streaming
```

**Expected:** All streaming tests pass.

### Step 5.8: Update finish() to use close_code_block helper

The `finish()` method has similar code for handling unterminated code blocks. Update it to reuse the helper:

```rust
    pub fn finish(&mut self) -> Vec<ParsedChunk> {
        let mut result = Vec::new();

        match &self.state {
            BufferState::Prose => {
                // Flush any buffered prose (when using threshold-based buffering)
                if let Some(prose_chunk) = self.flush_prose_buffer() {
                    result.push(prose_chunk);
                }
            }
            BufferState::Code { .. } => {
                // Emit unterminated code block using existing helper
                if !self.buffer.is_empty() {
                    result.extend(self.close_code_block());
                }
            }
        }

        // Reset state (close_code_block already resets to Prose, but ensure clean state)
        self.state = BufferState::Prose;
        self.buffer.clear();
        self.buffered_prose_lines = 0;

        result
    }
```

### Step 5.9: Run all tests

```bash
cargo test -p ralph-core
```

**Expected:** All tests pass.

---

## Task 6: Final Verification

### Step 6.1: Run clippy

```bash
cargo clippy -p ralph-core -- -D warnings
```

**Expected:** No warnings or errors.

### Step 6.2: Run all tests

```bash
cargo test -p ralph-core
```

**Expected:** All tests pass.

### Step 6.3: Run formatter

```bash
cargo fmt --check
```

**Expected:** No formatting issues.

---

## Summary of Changes

| File | Change Type | Description |
|------|-------------|-------------|
| `crates/core/src/stream/accumulation.rs` | Delete | Remove empty impl block (lines 229-232) |
| `crates/core/src/chunk/heuristics.rs` | Add | Named constant `MIN_DIFF_LINES_THRESHOLD` |
| `crates/core/src/chunk/tests/fence.rs` | Create | Unit tests for fence module |
| `crates/core/src/chunk/tests/mod.rs` | Edit | Add `mod fence;` |
| `crates/core/src/chunk/batch.rs` | Refactor | Extract `flush_prose`, `emit_code_block`, `append_line` helpers |
| `crates/core/src/chunk/tests/batch.rs` | Edit | Add helper function tests |
| `crates/core/src/chunk/streaming.rs` | Refactor | Extract `emit_eager_prose`, `accumulate_prose_line`, `start_code_block`, `close_code_block`, `accumulate_code_line` helpers |

---

**Plan complete and saved to `/Users/guzmanmonne/Projects/Rust/ralph/docs/plans/core-simplification.md`. Two execution options:**

**1. Subagent-Driven (this session)** - I dispatch fresh subagent per task, review between tasks, fast iteration

**2. Parallel Session (separate)** - Open new session with executing-plans, batch execution with checkpoints

**Which approach?**
