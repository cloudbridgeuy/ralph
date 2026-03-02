# Runtime Control

This document describes how to control ralph execution during runtime.

## Overview

Ralph provides interactive keyboard controls and signal-based interruption for flexible execution management. Press keys to control execution flow, or use `Ctrl+C` to interrupt and cleanly finalize the session.

## Keyboard Controls

When running in an interactive terminal, ralph provides keyboard controls during subprocess execution:

| Key | Action | Description |
|-----|--------|-------------|
| `s` | Soft Stop | Finish current iteration, then exit |
| `S` | Hard Stop | Immediately halt and save paused state |
| `p` | Pause | Toggle pause/resume output display |

### Soft Stop (`s`)

Pressing `s` requests a graceful exit after the current iteration completes:

1. The spinner hints change to `[finishing...]`
2. The current subprocess runs to completion
3. The iteration log and git diff are captured normally
4. The session finalizes with "Completed" outcome
5. The run loop exits without starting the next iteration

Use soft stop when you want to review progress or take a break without interrupting work in progress.

### Hard Stop (`S`)

Pressing `S` immediately halts execution and saves state for later resumption:

1. The subprocess receives SIGINT and terminates
2. Paused state is written to `{data_dir}/paused.toml`
3. The session can be resumed later with `ralph run --resume`
4. Partial iteration output is discarded (not written to progress file)

Use hard stop when you need to stop immediately but want to resume from where you left off.

### Pause (`p`)

Pressing `p` toggles pause/resume for output display:

1. **When pausing**: Subprocess continues running, but output is buffered internally
2. The spinner shows `[paused - p: resume]` to indicate pause state
3. **When resuming**: Buffered output is immediately displayed
4. Pause state does not affect iteration timing or final results

Use pause when you need to examine output without the stream continuing to scroll.

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

## Raw Mode Lifecycle

Keyboard controls require terminal raw mode to detect individual keypresses. Ralph carefully manages raw mode to avoid corrupting subprocess output:

### When Raw Mode is Enabled

- During spinner display (waiting for LLM response)
- During output gaps when the LLM is thinking

### When Raw Mode is Disabled

- Before any subprocess stdout/stderr is written
- On all exit paths (completion, timeout, interrupt, error)

### Panic Safety

Raw mode uses an RAII guard (`RawModeGuard`) that automatically disables raw mode when dropped. This ensures cleanup even if the code panics, preventing terminal corruption.

### Pattern Reference

The raw mode lifecycle follows the same pattern as `replay_countdown.rs`:
- Enable raw mode only when actively polling for keyboard input
- Disable before any output operations
- Use `event::poll(Duration::ZERO)` for non-blocking input detection

## Non-Interactive Mode

When ralph runs in non-interactive contexts (piped output, CI environments):
- Keyboard controls are disabled (raw mode is never enabled)
- Spinner does not display
- Signal handling remains active
- Sessions finalize correctly on interruption

## Code Locations

| Component | File | Description |
|-----------|------|-------------|
| Keyboard handling | `crates/ralph/src/keyboard.rs` | RawModeGuard, key action detection |
| Signal handler | `crates/ralph/src/signal.rs` | SIGINT/SIGTERM handling |
| Subprocess spinner | `crates/ralph/src/subprocess/spinner.rs` | Raw mode lifecycle, keyboard polling |
| Spinner display | `crates/ralph/src/spinner.rs` | Key hints, pause state display |
| Run loop | `crates/ralph/src/run/mod.rs` | Soft stop handling at iteration boundaries |
