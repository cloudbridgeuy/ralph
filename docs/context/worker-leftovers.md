# Worker leftovers

A running list of things workers and smoke tests leave behind outside the repo. Workers append what they leave. Nobody deletes entries by hand. Automatic cleanup on `close_worker` is planned as a follow-up to the workers epic (GUZ-503).

Ralph itself never reads these transcript folders.

## Kinds

1. Trust entries in `~/.claude.json` for paths that no longer exist.
2. Transcript folders in `~/.claude/projects/<path with / turned into ->`.

## Known items

- 14 `/tmp` trust entries and 81 transcript folders from earlier closed workers and smoke tests (not itemized).
- From the GUZ-508 smoke test: trust entry `/private/tmp/guz508smoke.VvrX/repo` and folder `~/.claude/projects/-private-tmp-guz508smoke-VvrX-repo/`.
