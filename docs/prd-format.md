# PRD File Format

This document describes the format and workflow for PRD (Product Requirements Document) files used by ralph.

## Overview

PRD files are TOML documents that define work items (stories) for ralph to track. Ralph monitors these files during iteration loops to detect when work is complete and to limit the number of iterations.

## File Location

PRD files are stored in `.local/plans/prd.toml` by default. You can specify a custom path using the `--prd` flag:

```bash
ralph run --prd ./my-project/requirements.toml
```

## Story Schema

Each story is defined as a `[[stories]]` table array entry. Stories have the following fields:

### Required Fields

| Field | Type | Description |
|-------|------|-------------|
| `description` | string | Brief description of what the story accomplishes |
| `passes` | boolean | Whether the story has been completed (`true`) or is pending (`false`) |

### Optional Fields

| Field | Type | Description |
|-------|------|-------------|
| `category` | string | Categorizes the type of work (see Categories below) |
| `steps` | array of strings | Implementation steps to complete the story |
| `acceptance` | array of strings | Criteria that must be met for the story to pass |
| `actor` | string | Who or what performs the story (e.g., "user", "system") |

**Note:** The `passes` field defaults to `false` if omitted. Ralph also accepts `passed` as an alias for `passes`.

## Categories

Categories help organize stories by type of work. Common categories include:

| Category | Description |
|----------|-------------|
| `ui` | User interface changes, display formatting, visual feedback |
| `internal` | Documentation, tests, refactoring, infrastructure |
| `feature` | New functionality or capabilities |
| `bugfix` | Fixes for existing functionality |

Categories are informational only - ralph does not filter or process stories differently based on category.

## Example PRD File

```toml
# Project PRD - Feature Development

[[stories]]
category = "feature"
description = "Add user authentication"
steps = [
  "Create login form component",
  "Implement JWT token validation",
  "Add protected route middleware",
  "Store session in secure cookie",
]
passes = false
acceptance = [
  "Users can log in with email/password",
  "Invalid credentials show error message",
  "Session persists across page reloads",
  "Logout clears session completely",
]

[[stories]]
category = "ui"
description = "Improve error message display"
steps = [
  "Add red border to invalid form fields",
  "Show inline validation messages",
  "Add shake animation on submit failure",
]
passes = true
acceptance = [
  "Error messages appear below invalid fields",
  "Messages disappear when field is corrected",
]

[[stories]]
category = "internal"
description = "Add unit tests for auth module"
steps = [
  "Test token generation",
  "Test token validation",
  "Test session expiry",
]
passes = false
acceptance = [
  "All auth functions have test coverage",
  "Tests pass in CI pipeline",
]
```

## Workflow

Ralph uses PRD files in its iteration loop as follows:

### 1. Initial Analysis

When `ralph run` starts, it:
- Reads the PRD file
- Counts total stories, completed stories, and pending stories
- Exits early if no pending stories exist (`passes = false`)
- Sets max iterations to pending count (unless overridden with `--max-iterations`)

### 2. Per-Iteration Monitoring

Before each iteration, ralph:
- Re-reads the PRD file
- Checks if pending count has dropped to zero
- Snapshots the PRD content for change detection

After each iteration, ralph:
- Re-reads the PRD file
- Compares to the snapshot (byte-for-byte)
- If unchanged: returns an error (stuck state detected)
- If changed: counts new pending stories

### 3. Completion Detection

The iteration loop ends when either:
- **All stories complete**: Pending count reaches zero
- **Completion marker found**: LLM outputs `<promise>COMPLETE</promise>`
- **Max iterations reached**: Configured iteration limit hit

### 4. Story State Transitions

Stories transition through states based on the `passes` field:

```
pending (passes = false)
    │
    ▼  [LLM completes the work]
    │
completed (passes = true)
```

Ralph expects the LLM to update `passes = true` after completing each story. If an iteration completes without any PRD changes, ralph treats this as a stuck state and returns an error.

## Best Practices

1. **Keep stories atomic**: Each story should represent a single, testable unit of work
2. **Write clear acceptance criteria**: These help the LLM know when a story is truly complete
3. **Order by priority**: Place highest-priority stories first in the file
4. **Use comments**: Add `#` comments to group related stories or add context
5. **Update incrementally**: Mark stories as `passes = true` as soon as they're complete

## Related Documentation

- [CLI Reference](cli-reference.md) - Command-line options including `--prd` and `--max-iterations`
- [Configuration](configuration.md) - Config file options for default paths
