# Worker leftovers

A running list of things workers and smoke tests leave behind outside the repo. Workers append what they leave. Nobody deletes entries by hand.

## Cleanup by worker kind

`close_worker` scopes home-directory cleanup by `WorkerKind`, and runs it only after a successful close whose plan includes `RemoveWorktree` (`cleans_claude_leftovers` and `cleans_cursor_leftovers` in `crates/core/src/`). A markers-only close does not clean home directories. Cleanup failures add warnings to the result rather than failing the close.

- **`Tmux` (currently Claude):** removes the worktree's transcript folder `~/.claude/projects/<mangled path>` and its trust entry under `projects` in `~/.claude.json`. The trust file is rewritten through a temp file; a concurrent change skips the rewrite with a warning. Successful cleanup leaves neither item; a missing worktree or a cleanup warning can leave them behind.
- **`Cursor`:** runs `agent acp`, never `claude`. It leaves `~/.cursor/projects/<slug>` (plus a hashed socket dir there when the path is long) and `~/.cursor/acp-sessions/<id>/`. Close removes project paths derived from the worktree's real path and sessions whose `meta.json` `cwd` exactly matches the worktree or its real path, and lists each removed path in its result. It keeps the project folders, with a warning, when another known path (a `git worktree list` entry or an `acp-sessions` `cwd`) maps to the same slug or the git listing fails. This does not restore `~/.cursor/cli-config.json` changes made by `agent acp`.
- **`Opencode`:** closes only that worker's session, evicts its worktree location and removes its worker files plus worktree/branch. It never scans or deletes `~/.cursor` or Claude home-directory data, and does not stop the shared `opencode serve`, even for the last worker. No general cleanup of opencode's own home-directory data is implemented.

See [ralph chat](ralph-chat.md#mcp-tools) for close steps and safety checks, and [Headless workers](headless-workers.md) for hosting kinds, compatibility and chat-owned host shutdown.

Ralph itself never reads these transcript folders.

## Kinds

1. Trust entries in `~/.claude.json` for paths that no longer exist.
2. Transcript folders in `~/.claude/projects/<path>`, where every character of the path that is not an ASCII letter or digit becomes `-`.

## Known items

- 14 `/tmp` trust entries and 81 transcript folders from earlier closed workers and smoke tests (not itemized).
- From the GUZ-508 smoke test: trust entry `/private/tmp/guz508smoke.VvrX/repo` and folder `~/.claude/projects/-private-tmp-guz508smoke-VvrX-repo/`.
