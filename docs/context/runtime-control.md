# Runtime Control

This document describes how to control ralph execution during runtime.

## Overview

Ralph provides signal-based interruption for graceful shutdown. Press `Ctrl+C` to interrupt execution and cleanly finalize the session.

## Signal Handling

| Signal | Action | Description |
|--------|--------|-------------|
| `SIGINT` (Ctrl+C) | Interrupt | Stop execution gracefully |
| `SIGTERM` | Interrupt | Stop execution gracefully (same behavior as SIGINT) |

## Interruption Behavior

When you press `Ctrl+C` during execution:

1. The current subprocess receives a termination signal
2. Any partial output is preserved in the iteration log
3. The session is finalized with "Interrupted" outcome
4. A message indicates the session was saved for reference

### Partial Results

If interruption occurs mid-iteration:
- Any output received before interruption is logged
- Tool calls and their results are preserved
- The iteration log records the incomplete state
- Git diff capture may be partial or skipped

### Session Finalization

After interruption:
- Session metadata is updated with final state
- Sessions list shows "Interrupted" outcome
- All iteration logs remain accessible
- Use `ralph replay <session-slug>` to review what was captured

## Why No Keyboard Controls

Earlier versions planned keyboard-based controls (pause with 's', stop with 'S', resume with 'p'). These were removed because:

1. **Raw terminal mode conflicts**: Enabling raw mode for keyboard input corrupts subprocess stdout, causing garbled output from claude
2. **Signal handling is sufficient**: Ctrl+C provides clean interruption that preserves session state
3. **Simplicity**: Signal-based control is standard Unix behavior and works reliably

If you need to pause and resume work, complete the current iteration and start a new session when ready.

## Non-Interactive Mode

When ralph runs in non-interactive contexts (piped output, CI environments):
- Spinner does not display
- Signal handling remains active
- Sessions finalize correctly on interruption

## Code Locations

| Component | File | Description |
|-----------|------|-------------|
| Signal handler | `crates/ralph/src/signal.rs` | SIGINT/SIGTERM handling |
| Subprocess | `crates/ralph/src/subprocess/spinner.rs` | Interrupt detection during execution |
| Run loop | `crates/ralph/src/run/mod.rs` | Interrupt handling in iteration loop |
