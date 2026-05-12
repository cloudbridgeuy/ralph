# Runtime Control

This document describes how to control ralph execution during runtime.

## Overview

Ralph is controlled at runtime through POSIX signals. `Ctrl+C` (SIGINT) is the only supported interactive interruption mechanism: it terminates the current subprocess, finalizes the session cleanly, and exits.

Mid-iteration keyboard polling (the historical `s` / `S` / `p` shortcuts and the raw-mode lifecycle that backed them) was removed in the `dismantle-keybindings` shape. The subprocess loop no longer reads stdin; key presses during a run are typed into the shell after the child exits.

## Signal Handling

| Signal | Action | Description |
|--------|--------|-------------|
| `SIGINT` (Ctrl+C) | Interrupt | Stop execution gracefully |
| `SIGTERM` | Interrupt | Stop execution gracefully (same behavior as SIGINT) |

The signal handler is registered once at startup; both signals set a shared atomic flag that the run loop and the subprocess loop poll. See [`crate::signal`](../../crates/ralph/src/signal.rs).

## Interruption Behavior

When you press `Ctrl+C` during execution:

1. The current subprocess and its process group receive a kill signal via `kill_process_group`
2. Any output already received is drained from the stdout/stderr channels and printed
3. The stream processor finishes, emitting any final formatted output
4. The session is finalized with an "Interrupted" outcome
5. Ralph exits with a non-zero exit code

The check fires from two places:

- **Inside the subprocess loop** (`crates/ralph/src/subprocess/spinner.rs`): polled every iteration of the read loop so that mid-iteration interrupts kill the child within milliseconds
- **Between iterations** (e.g. `crates/ralph/src/strategy/prd_loop.rs`, `crates/ralph/src/strategy/conversation_loop.rs`): checked before invoking the next subprocess so that a Ctrl+C received during a brief gap is still honored

### Partial Results

If interruption occurs mid-iteration:

- Output received before the signal is preserved in the iteration log
- Tool calls and their results captured so far are preserved
- The iteration log records the incomplete state
- Git diff capture may be partial or skipped

### Session Finalization

After interruption:

- Session metadata is updated with the final state
- The sessions list shows an "Interrupted" outcome
- All iteration logs remain accessible
- Use `ralph replay <session-slug>` to review what was captured

## Timeout-Based Termination

Independent of any user input, each subprocess invocation enforces a configurable timeout (`timeout_secs` on `SpinnerSubprocessConfig`). When the timeout elapses:

1. The subprocess and its process group are killed via `kill_process_group`
2. Buffered output is drained
3. `SubprocessError::Timeout` is returned with a partial `StreamingSubprocessResult`

Recovery and strategy loops decide whether to retry the iteration or surface the timeout as a failure.

## Non-Interactive Mode

When ralph runs in non-interactive contexts (piped output, CI environments):

- The spinner does not render
- Signal handling remains active — `Ctrl+C` and `SIGTERM` still kill the child cleanly
- Sessions finalize correctly on interruption

## Code Locations

| Component | File | Description |
|-----------|------|-------------|
| Signal handler | `crates/ralph/src/signal.rs` | SIGINT/SIGTERM handler, `is_interrupted()` atomic flag |
| Subprocess spinner | `crates/ralph/src/subprocess/spinner.rs` | Signal & timeout kill paths, spinner, stream draining |
| Process-group kill | `crates/ralph/src/subprocess/mod.rs` | `kill_process_group` helper |
| Spinner display | `crates/ralph/src/spinner/mod.rs` | Slug, iteration, elapsed-time display |
| PRD loop | `crates/ralph/src/strategy/prd_loop.rs` | Inter-iteration signal check, finalization |
| Conversation loop | `crates/ralph/src/strategy/conversation_loop.rs` | Inter-iteration signal check, finalization |
