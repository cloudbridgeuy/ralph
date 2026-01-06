# Run Command Architecture

## Context & Purpose

The `run` command implements an iterative LLM-driven development workflow. Instead of running a single long LLM session that accumulates context and requires compaction, work is divided into discrete user stories. Each iteration spawns a fresh LLM session that picks up one story, implements it, updates tracking files, and commits. This pattern reduces context bloat and creates natural checkpoints.

The command solves three problems:

1. **Session management** — Automates the loop of invoking the LLM, passing context files, and checking completion conditions. Without this, developers manually run the same command repeatedly.

2. **Progress tracking** — Detects when the LLM makes no progress (PRD unchanged) or completes all work (no pending stories). This prevents infinite loops and wasted compute.

3. **Auditability** — Captures every iteration's output with rich formatting, stores it for later review, and records git diffs showing what changed. This creates a reviewable history of how the codebase evolved.

The command is opinionated about workflow (PRD-driven, commit-per-feature) but flexible about tooling (prompt templates, command templates, configurable paths). It ships with sensible defaults for the `claude` CLI but accommodates other tools through configuration.

The target user is a developer using LLM assistants for coding who wants to automate the iteration loop while maintaining visibility into what each session accomplished.

## Core Concepts

**Session** — A complete `ralph run` invocation. Contains metadata (slug, project path, start time, iteration count) and references to iteration logs. Sessions are identified by slugs, either user-provided or auto-generated (e.g., `quiet-mountain`). The global `sessions.toml` indexes all sessions.

**Iteration** — A single LLM invocation within a session. Each iteration has a sequence number, captured output (as typed chunks), git diff of changes, and outcome (success, failure, or early-exit). Iterations are the atomic unit of work.

**Chunk** — A segment of LLM output with a detected content type: prose (markdown), code (fenced blocks), or diff (unified diff format). Chunks enable type-appropriate syntax highlighting during streaming and replay. The output parser buffers incomplete chunks until boundaries are detected.

**PRD (Product Requirements Document)** — A TOML file containing user stories with `passed` boolean status. Ralph reads this to count pending work and detect completion. The structure is minimal: `[[stories]]` array with `passed` field; all other fields are ignored.

**Context Files** — Files passed to the LLM for project context: design document, PRD, and progress notes. Each has a default path. The design doc and progress file are created (empty) if missing; the PRD must exist. The LLM reads these; ralph doesn't interpret their content beyond the PRD's `passed` fields.

**Prompt Template** — The instructions sent to the LLM, with placeholders (`{design_file}`, `{prd_file}`, `{progress_file}`) that ralph substitutes before invocation. Users can override the built-in template.

**Command Template** — The shell command pattern for invoking the LLM tool, with a `{prompt}` placeholder. Defaults to `claude --permission-mode acceptEdits -p {prompt}`.

## Invariants & Rules

**PRD must exist.** Unlike other context files, the PRD is required. Without it, there's no source of user stories. Ralph fails immediately with an actionable error if the PRD is missing.

**Design doc and progress file are touched if missing.** Ralph creates empty files at default paths and informs the user. This ensures consistent file references and helps users discover the expected project structure.

**PRD must change during each iteration.** Ralph snapshots the PRD content before invoking the LLM. If the PRD is byte-for-byte identical after the iteration completes, ralph exits with an error. This catches stuck LLMs that run but accomplish nothing.

**Iterations run sequentially, never in parallel.** Each iteration must complete before the next begins. The post-iteration PRD check depends on the prior iteration's changes being committed or at least written to disk.

**Pending story count determines loop continuation.** Before each iteration, ralph counts stories where `passed = false`. Zero pending stories means work is complete; the loop exits cleanly. This check also runs post-iteration to catch completion mid-run.

**Completion marker in output triggers early exit.** If the LLM emits the marker string (default `<promise>COMPLETE</promise>`), ralph exits the loop regardless of PRD state. This allows the LLM to signal completion explicitly.

**Chunks must be complete before output.** Code blocks and diffs are buffered until their closing boundary is detected. Partial chunks are not streamed to avoid broken syntax highlighting. Prose chunks stream more eagerly since they have no strict boundaries.

**Session slugs must be unique.** If a user-provided slug already exists in `sessions.toml`, ralph rejects it with an error rather than overwriting.

## Boundaries & Interfaces

### CLI Interface

The `run` subcommand accepts:

| Argument | Description |
|----------|-------------|
| `[iterations]` | Optional. Max iterations to run. Defaults to pending story count in PRD. |
| `--slug <name>` | Session identifier. Auto-generated if omitted (e.g., `fuzzy-walrus`). |
| `--prompt <file\|-\|string>` | Custom prompt template. Supports file path, `-` for stdin, or inline string. |
| `--command <template>` | Custom LLM invocation pattern. Default: `claude --permission-mode acceptEdits -p {prompt}` |
| `--design <path>` | Design document path. Default: `.claude/designs/design.md` |
| `--prd <path>` | PRD file path. Default: `.claude/plans/prd.toml` |
| `--progress <path>` | Progress notes path. Default: `.claude/plans/progress.txt` |
| `--retry <N>` | Auto-retry count on failure. Default: 3. |
| `--completion-marker <string>` | Custom completion marker. Default: `<promise>COMPLETE</promise>` |

### Filesystem Boundaries

- **Project files**: Context files and PRD in current working directory
- **Global config**: `~/.config/ralph/` for session storage and metadata
- **External tools**: `claude`, `delta`, `diff`, `git` invoked as subprocesses

### Subprocess Interface

Ralph spawns the LLM tool and:
- Captures stdout/stderr streams
- Monitors exit code for success/failure
- Applies timeout (configurable, with sensible default)

### Git Interface

After each iteration, ralph runs `git diff` to capture changes. Assumes git is available and the project is a git repository. Non-git projects skip diff capture with a warning.

### Output Interface

Streamed to stdout with syntax highlighting. Chunks are formatted using giallo for code/markdown. Diff highlighting uses delta if available, falls back to system `diff` with color, then basic inline coloring. No pager mode; output flows directly to terminal.

## Expected Behaviors

### Initialization

1. Parse arguments, validate iteration count is positive (if provided)
2. Resolve context file paths (use defaults or overrides)
3. Verify PRD exists; fail immediately if missing
4. Parse PRD to count pending stories; use as default iteration count if not specified
5. Touch missing design doc and progress file; print notice for each created
6. Generate or validate session slug (check uniqueness in `sessions.toml`)
7. Create session directory at `~/.config/ralph/sessions/<slug>/`
8. Register session in `sessions.toml` with project path and start timestamp

### Iteration Loop

1. **Pre-check**: Parse PRD, count pending stories; exit if zero
2. Snapshot PRD content in memory
3. Substitute placeholders in prompt template
4. Substitute `{prompt}` in command template
5. Spawn LLM subprocess, stream output
6. Parse output stream into chunks (prose/code/diff)
7. Buffer incomplete chunks; emit complete chunks with formatting
8. Store iteration output (chunks with types) to session directory
9. On subprocess completion: check exit code
10. On failure: print stderr/stdout, retry or prompt user
11. On success: capture `git diff`, store alongside iteration log
12. **Post-check**: Compare PRD to snapshot; error if unchanged
13. Re-count pending stories; exit if zero or completion marker found
14. Loop to next iteration

### Replay

1. Look up session by slug in `sessions.toml`
2. Read iteration logs in sequence
3. Re-render chunks with syntax highlighting to stdout

### Session Listing

`ralph sessions` lists all sessions with slug, project, date, and iteration count.

## Error States

### Initialization Errors

| Error | Cause | Recovery |
|-------|-------|----------|
| PRD not found | PRD file doesn't exist at default or specified path | Exit: "PRD file not found at {path}. Create it with user stories to begin." |
| PRD parse failure | TOML syntax error or missing `[[stories]]` structure | Exit with parse error location |
| PRD has no stories | File exists but contains zero `[[stories]]` entries | Exit: "PRD contains no stories. Add stories to begin." |
| Duplicate slug | User-provided slug already exists in `sessions.toml` | Exit: "Session '{slug}' already exists. Choose a different slug or omit for auto-generated." |
| Session directory creation failed | Permissions or disk space issue at `~/.config/ralph/` | Exit with system error details |

### Iteration Errors

| Error | Cause | Recovery |
|-------|-------|----------|
| LLM subprocess failed | Non-zero exit code from claude/tool | Print stdout/stderr, retry up to N times, then prompt user (retry/abort) |
| LLM timeout | Subprocess exceeded time limit | Treat as failure, apply retry logic |
| PRD unchanged | PRD byte-identical before and after iteration | Exit: "PRD unchanged after iteration. LLM may be stuck. Check progress.txt for notes." |
| Git diff failed | `git` not found or not a repository | Warn "Git diff unavailable, skipping change capture" and continue |
| Context file read error | Permission denied or file deleted mid-run | Exit with error; session is incomplete |

### Replay Errors

| Error | Cause | Recovery |
|-------|-------|----------|
| Session not found | Slug doesn't exist in `sessions.toml` | Exit: "Session '{slug}' not found. Run 'ralph sessions' to list available sessions." |
| Iteration log missing | Session exists but log file corrupted/deleted | Warn and skip that iteration, or exit if all logs missing |

## Configuration Files

### `~/.config/ralph/sessions.toml`

Index of all sessions across projects:

```toml
# Auto-managed by ralph. Manual edits are preserved but not recommended.

[[sessions]]
slug = "quiet-mountain"
project = "/Users/dev/myproject"
started_at = 2025-01-06T14:30:00Z
completed_at = 2025-01-06T15:45:00Z
iterations = 7
outcome = "completed"  # "completed" | "aborted" | "failed" | "in_progress"

[[sessions]]
slug = "fuzzy-walrus"
project = "/Users/dev/other-project"
started_at = 2025-01-06T16:00:00Z
iterations = 3
outcome = "aborted"
```

### Session Directory Structure

```
~/.config/ralph/sessions/<slug>/
├── session.toml          # Session metadata
├── iteration-1.toml      # Iteration log with typed chunks
├── iteration-1.diff      # Git diff after iteration 1
├── iteration-2.toml
├── iteration-2.diff
└── ...
```

### `iteration-N.toml`

Structured log for replay:

```toml
sequence = 1
started_at = 2025-01-06T14:30:00Z
completed_at = 2025-01-06T14:35:00Z
exit_code = 0
pending_before = 5
pending_after = 4

[[chunks]]
type = "prose"
content = "I'll implement the authentication feature..."

[[chunks]]
type = "code"
language = "rust"
content = """
fn authenticate() {
    // ...
}
"""

[[chunks]]
type = "diff"
content = """
diff --git a/src/auth.rs b/src/auth.rs
...
"""
```

### `iteration-N.diff`

Raw git diff output, stored separately for easy viewing with external tools.

## Success Criteria

### Functional Correctness

- Running `ralph run` without iteration count defaults to the number of pending stories in the PRD
- Running `ralph run 5` caps iterations at 5, stopping early if PRD completes sooner
- PRD file must exist; missing PRD produces immediate error with clear message
- Design doc and progress file are created (touched) if missing; user sees which files were created
- PRD unchanged after iteration produces a clear error message explaining the stuck state
- Custom `--prompt` and `--command` templates substitute placeholders correctly
- Session data persists to `~/.config/ralph/sessions/<slug>/` with iteration logs and git diffs
- `ralph replay <slug>` reproduces the session output with syntax highlighting

### Output Quality

- Code blocks render with syntax highlighting via giallo
- Diff output renders with delta when available, degrades gracefully to diff or basic coloring
- Streaming output appears progressively; code/diff chunks buffer until complete
- No broken or partial syntax highlighting artifacts

### Error Handling

- Missing PRD file fails immediately with actionable error (not touched/created)
- Subprocess failure prints captured stdout/stderr before retry prompt
- Retry logic respects `--retry N` count before prompting user
- Missing git repository warns but continues without diff capture
- Duplicate slug rejects with clear error, not silent overwrite

### Usability

- Default prompt and command work out of the box with `claude` CLI
- `ralph run` with no flags runs successfully if PRD exists with pending stories
- Generated slugs are memorable (adjective-noun) and unique
