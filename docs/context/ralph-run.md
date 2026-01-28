# ralph run Command

This document provides comprehensive documentation for the `ralph run` command, which drives PRD-driven iteration loops for automated development.

## Code Locations

| Component | File | Description |
|-----------|------|-------------|
| CLI args | `crates/ralph/src/cli/mod.rs` | RunArgs struct definition |
| Main loop | `crates/ralph/src/run/mod.rs` | run() function and iteration logic |
| Recovery | `crates/ralph/src/run/recovery.rs` | Retry logic for subprocess failures |
| Session | `crates/ralph/src/session.rs` | Session management and persistence |
| Startup display | `crates/ralph/src/startup/` | Display formatting modules |
| Signal handling | `crates/ralph/src/signal.rs` | SIGINT/SIGTERM handlers |
| Defaults | `crates/core/src/context.rs` | PROMPT_TEMPLATE, completion marker |

## CLI Flags

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `[ITERATIONS]` | positional | pending story count | Maximum iterations to run |
| `-s, --slug` | string | auto-generated | Session identifier (e.g., "quiet-mountain") |
| `-p, --prompt` | string | built-in template | Custom prompt (file path, `-` for stdin, or inline) |
| `-c, --command` | string | claude CLI | LLM invocation template with `{prompt}` placeholder |
| `--prd` | path | `.local/plans/prd.toml` | PRD file path |
| `--max-attempts` | usize | 3 | Recovery attempts after subprocess failure |
| `--completion-marker` | string | `<promise>COMPLETE</promise>` | Marker to detect completion |
| `--timeout` | u64 | 600 | Subprocess timeout in seconds |
| `--theme` | string | `Monokai Extended` | Syntax highlighting theme |
| `--no-background` | flag | false | Disable theme background colors |
| `-a, --additional-prompt` | string | none | Extra instructions appended to prompt |
| `--verbose-tools` | string | none | Verbose output for specific tools |
| `--no-prompt` | flag | false | Suppress prompt display before Iteration 1 |

### Verbose Tools Examples

```bash
ralph run --verbose-tools=grep,bash    # Enable for specific tools (or omit value for all)
```

## Iteration Loop Flow

The iteration loop follows this sequence:

```
┌─────────────────────────────────────────────────────────────┐
│ PRE-LOOP SETUP                                              │
│ 1. Verify PRD exists                                        │
│ 2. Parse PRD, count pending stories                         │
│ 3. Exit early if pending_count == 0                         │
│ 4. Determine max_iterations (from arg or pending count)     │
│ 5. Initialize session (or continue existing)                │
│ 6. Display startup info and prompt (if first run)           │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ FOR EACH ITERATION (1 to max_iterations):                   │
│                                                             │
│  1. Check interrupt signal → exit if interrupted            │
│  2. Re-read PRD, count pending stories (pre-check)          │
│  3. Exit early if pending_before == 0                       │
│  4. Display iteration header                                │
│  5. Snapshot PRD content (for change detection)             │
│  6. ┌─────────────────────────────────────────────────────┐ │
│     │ INVOKE SUBPROCESS with failure recovery              │ │
│     │ - Up to max_attempts + 1 total attempts              │ │
│     │ - Timeout after timeout_secs                         │ │
│     │ - On exhaustion: prompt user for action              │ │
│     └─────────────────────────────────────────────────────┘ │
│  7. Write iteration log (JSONL format)                      │
│  8. Check interrupt signal → exit if interrupted            │
│  9. Capture git diff to iteration-{N}.diff                  │
│ 10. Display iteration summary (cost, duration, tokens)      │
│ 11. Re-read PRD, count pending stories (post-check)         │
│ 12. Update iteration log with pending_after                 │
│ 13. ┌─────────────────────────────────────────────────────┐ │
│     │ CHECK COMPLETION CONDITIONS (in order):              │ │
│     │ a. Completion marker found → exit loop               │ │
│     │ b. All stories complete (pending == 0) → exit loop   │ │
│     │ c. PRD unchanged → error (stuck state)               │ │
│     │ d. Otherwise → continue to next iteration            │ │
│     └─────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ POST-LOOP FINALIZATION                                      │
│ 1. Finalize session as Completed                            │
│ 2. Return RunResult with aggregated metrics                 │
└─────────────────────────────────────────────────────────────┘
```

## Session Management

### Session Storage

Sessions are stored in platform-specific data directories:

| Platform | Path |
|----------|------|
| Linux | `~/.local/share/ralph/sessions/` |
| macOS | `~/Library/Application Support/ralph/sessions/` |
| Windows | `%APPDATA%\ralph\sessions\` |

Override with `RALPH_DATA_DIR` environment variable.

### Session Structure

Each session creates a directory with:

```
sessions/{slug}/
├── session.toml         # Session metadata
├── iteration-1.jsonl    # Iteration 1 log
├── iteration-1.diff     # Git diff after iteration 1
├── iteration-2.jsonl    # Iteration 2 log
├── iteration-2.diff     # Git diff after iteration 2
└── ...
```

### Session Metadata (session.toml)

```toml
slug = "quiet-mountain"
project_path = "/path/to/project"
started_at = "2024-01-15T10:30:00Z"
completed_at = "2024-01-15T11:45:00Z"
iterations = 3
outcome = "Completed"
prompt = "..."  # Full prompt for replay
```

### Global Sessions Index

All sessions are tracked in a global index (`sessions.toml` in data directory) for:
- Listing sessions by project
- Filtering by outcome
- Finding most recent session
- Quick session lookup

### Session Lifecycle

1. **Creation**: Auto-generated slug (adjective-noun) or user-provided
2. **Iterations**: Each iteration appends log and diff files
3. **Finalization**: Metadata updated with outcome and final timestamp

Possible outcomes: `Completed`, `Failed`, `Interrupted`, `Aborted`, `InProgress`

## Completion Conditions

Completion is checked after each iteration (see flow diagram step 13 for priority order). The loop also exits when `max_iterations` is reached.

### Completion Marker

Default: `<promise>COMPLETE</promise>`

The LLM should output this marker when it determines all PRD stories are implemented. Customize with `--completion-marker`.

## Failure Recovery

### Retry Logic

When the LLM subprocess fails (non-zero exit code or timeout):

1. **Automatic retry**: Up to `max_attempts` times (default 3, for 4 total attempts)
2. **Between attempts**: Error message displayed with captured output
3. **After exhaustion**: User prompted for action

### User Prompt Options

When all automatic retries are exhausted:

| Action | Behavior |
|--------|----------|
| **Retry** | Continue same session, reuse slug, append iterations |
| **Abort** | Finalize session as Aborted, exit |
| **EOF/non-interactive** | Finalize session as Failed |

### Error Context

Failed subprocess errors include:
- Exit code
- Attempt count
- Captured stdout/stderr
- Session slug (for later finalization)
- Iterations completed

## Signal Handling

### Supported Signals

| Signal | Behavior |
|--------|----------|
| `SIGINT` (Ctrl+C) | Graceful interrupt |
| `SIGTERM` | Graceful interrupt |

### Interruption Behavior

1. Interrupt flag set (atomic operation)
2. Current subprocess receives termination signal
3. Partial output preserved in iteration log
4. Session finalized as `Interrupted`
5. Message: `Interrupted. Session '{slug}' saved.`

Interrupt is checked:
- At the start of each iteration
- After writing iteration log

See [Runtime Control](runtime-control.md) for detailed interruption documentation.

## Output Patterns

### Startup Display

Shows before Iteration 1 (on first run, not on recovery):
- Session slug
- Story counts (total, pending, completed)
- Max iterations
- Custom configuration flags in use

### Iteration Header

```
╭─── Iteration 1/5 ─────────────────────────────────────────────╮
│ 3 stories pending                                             │
╰───────────────────────────────────────────────────────────────╯
```

In piped/non-terminal mode: plain ASCII.

### Spinner During Execution

```
⠋ Waiting for response... 5s [brave-panda 1/5]
```

Updates every 80ms with:
- Animated braille spinner
- Context message (Waiting, Thinking, Running tool, Buffering code)
- Elapsed time (current iteration + accumulated session time)
- Session slug and iteration progress

Stops automatically when first output arrives.

### Iteration Summary

```
  cost: $0.0234  duration: 12.5s  model: claude-sonnet-4-20250514
  tokens: 1,234 in / 567 out
```

Displayed after each iteration with aggregated metrics.

### Prompt Display

Shown before Iteration 1 (unless `--no-prompt`):
- Full prompt text
- Attached file references highlighted

## Configuration Precedence

### Prompt-Related Settings

No environment variable support (explicit CLI only):
1. CLI flags (`--prompt`, `--prd`, `--completion-marker`, `--additional-prompt`)
2. Hardcoded defaults

### Theme Settings

1. CLI flag (`--theme`, `--no-background`)
2. Environment variable (`RALPH_THEME`, `RALPH_NO_BACKGROUND`)
3. Config file (if present)
4. Default: `Monokai Extended`

### Subprocess Settings

All configured via CLI:
- `--timeout` (default: 600s)
- `--max-attempts` (default: 3)
- `--command` (default: claude CLI invocation)

## Examples

### Basic Usage

```bash
# Run with defaults (PRD at .local/plans/prd.toml)
ralph run

# Limit to 3 iterations
ralph run 3

# Custom session name
ralph run --slug my-feature

# Custom PRD location
ralph run --prd specs/feature.toml
```

### Custom Prompts

```bash
# Inline additional instructions
ralph run -a "Focus on error handling and edge cases"

# Additional instructions from file
ralph run -a /path/to/instructions.txt

# Completely custom prompt template
ralph run -p /path/to/custom-prompt.txt
```

### Subprocess Configuration

```bash
# Longer timeout for complex tasks
ralph run --timeout 1200

# More retry attempts
ralph run --max-attempts 5

# Custom completion marker
ralph run --completion-marker "<done/>"
```

### Display Options

```bash
# Hide prompt display
ralph run --no-prompt

# Verbose tool output
ralph run --verbose-tools=bash,grep

# Custom theme
ralph run --theme "Solarized (dark)"
```

## Related Documentation

- [Prompt Template System](prompt-template.md) - Placeholders and customization
- [Runtime Control](runtime-control.md) - Signal-based interruption
- [Git Workflow](git-workflow.md) - Semantic commits and Progressive Disclosure
