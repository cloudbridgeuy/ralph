# Runtime Control

This document describes the keyboard controls available during ralph execution for pausing, stopping, and resuming sessions.

## Overview

Ralph provides runtime control features that allow users to pause or stop execution without losing session state. These controls are available during subprocess execution and appear as hints in the spinner status line.

## Key Bindings

| Key | Action | Description |
|-----|--------|-------------|
| `s` | Soft Stop | Pause after current iteration completes |
| `S` | Hard Stop | Interrupt immediately, can resume later |
| `p` | Resume/Play | Continue from paused state |
| `q` | Quit | Exit from paused state |

## State Machine

```
                    ┌──────────────────┐
                    │     Running      │
                    │  [s: pause │ S:  │
                    │      stop]       │
                    └────────┬─────────┘
                             │
              ┌──────────────┼──────────────┐
              │ 's' pressed  │              │ 'S' pressed
              ▼              │              ▼
    ┌─────────────────┐      │    ┌─────────────────┐
    │ Soft-stopping   │      │    │   Hard-stopped  │
    │ [finishing...]  │      │    │ Interrupted.    │
    └────────┬────────┘      │    │ Press p/q       │
             │               │    └────────┬────────┘
             │ Iteration     │             │
             │ completes     │             │
             ▼               │             │
    ┌─────────────────┐      │             │
    │  Soft-stopped   │      │             │
    │ Paused. Press   │      │             │
    │ p to continue,  │      │             │
    │ q to quit       │      │             │
    └────────┬────────┘      │             │
             │               │             │
             │ 'p' pressed   │ 'p' pressed │
             │ (next iter)   │ (resume)    │
             └───────────────┼─────────────┘
                             │
                             ▼
                    ┌──────────────────┐
                    │     Running      │
                    │ (continues)      │
                    └──────────────────┘
```

## Soft Stop (`s`)

The soft stop allows the current iteration to complete fully before pausing:

1. Press `s` during execution
2. Spinner shows `[finishing...]` to indicate pending pause
3. Current iteration completes (including any commits)
4. Ralph enters **Soft-stopped** state
5. User can press `p` to continue with next iteration or `q` to quit

**Use case**: When you want to review work so far before continuing, or take a break without losing context.

### Soft Stop Resume Behavior

When resuming from soft-stopped state:
- A new iteration starts fresh
- Iteration numbering continues from where it left off
- No special continuation prompt is needed

## Hard Stop (`S`)

The hard stop immediately terminates the subprocess:

1. Press `S` (Shift+s) during execution
2. Claude subprocess receives SIGTERM and stops
3. Ralph enters **Hard-stopped** state
4. Claude session UUID is preserved for resumption
5. User can press `p` to resume or `q` to quit

**Use case**: When you need to stop immediately (e.g., claude is generating unwanted output).

### Hard Stop Resume Behavior

When resuming from hard-stopped state:
- The same Claude session is resumed using `--session-id <uuid>`
- A continuation prompt is sent: "Continue working from where you left off"
- Iteration numbering continues appropriately

**Note**: If the Claude session has expired or is unavailable, ralph will notify you and start a fresh iteration instead.

## Spinner Display

During execution, the spinner shows key binding hints:

```
⠋ Waiting for response... Session: brave-panda | Iteration: 2/5 | 12s [s: pause | S: stop]
```

When a soft stop is pending (after pressing `s`):

```
⠋ Waiting for response... Session: brave-panda | Iteration: 2/5 | 15s [finishing...]
```

The key hints use dim text styling to avoid distracting from the main status information.

## Non-Interactive Mode

When ralph is not running in an interactive terminal (e.g., piped output or running in CI), keyboard controls are automatically disabled:

- Spinner does not display
- Key presses are not monitored
- The subprocess runs to completion without control options

## Code Locations

| Component | File | Description |
|-----------|------|-------------|
| KeyboardMonitor | `crates/ralph/src/keyboard.rs` | Non-blocking key event detection |
| Spinner | `crates/ralph/src/spinner.rs` | Animated spinner with key hints |
| Subprocess | `crates/ralph/src/subprocess/spinner.rs` | Integration of spinner with keyboard |
| Run loop | `crates/ralph/src/run/mod.rs` | State machine for soft/hard stop |
