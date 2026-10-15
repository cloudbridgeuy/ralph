# Worker leftovers

A running list of things workers and smoke tests leave behind outside the repo. `close_worker` now removes both the transcript folder and the trust entry of the worker it closes, so closed Claude workers add nothing here. Cursor workers run `agent acp`, never `claude`, so they create neither and `close_worker` skips the cleanup for them. Workers append what they leave. Nobody deletes entries by hand.

Ralph itself never reads these transcript folders.

## Kinds

1. Trust entries in `~/.claude.json` for paths that no longer exist.
2. Transcript folders in `~/.claude/projects/<path>`, where every character of the path that is not an ASCII letter or digit becomes `-`.

## Known items

- 14 `/tmp` trust entries and 81 transcript folders from earlier closed workers and smoke tests (not itemized).
- From the GUZ-508 smoke test: trust entry `/private/tmp/guz508smoke.VvrX/repo` and folder `~/.claude/projects/-private-tmp-guz508smoke-VvrX-repo/`.
