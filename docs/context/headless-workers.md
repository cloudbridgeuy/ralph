# Headless workers

Worker hosting and chat-owned lifetimes are separate from the selected agent. For spawn, reply, reporting and close behavior, see [ralph chat](ralph-chat.md#mcp-tools); for home-directory cleanup, see [Worker leftovers](worker-leftovers.md).

## WorkerKind and compatibility

`WorkerKind` (`crates/core/src/worker/kind.rs`) describes how a worker is hosted:

| Kind | Current harness | Hosting |
| --- | --- | --- |
| `Tmux` | `claude` (default) | One tmux session per worker |
| `Cursor` | `cursor` | One `agent acp` child per worker, owned by `CursorHost` |
| `Opencode` | `opencode` | One session per worker on the chat's shared `opencode serve`, owned by `OpencodeHost` |

`Harness` still selects the agent and its spawn options; `WorkerKind` replaces the old Cursor boolean in `RunningWorker`, `WorkerObservation` and `CloseState`. The separate `opencode` boolean remains as agent/session metadata, not the hosting discriminator. For example, `close_plan` can remove an old opencode session before closing a tmux worker when that metadata is present.

`RunningWorker.kind` serializes as `"kind": "tmux"`, `"cursor"` or `"opencode"`. Its serde `alias = "cursor"` and `WorkerKind`'s custom deserializer let old records with `"cursor": true` load as `Cursor`, and `"cursor": false` as `Tmux`. If neither field is present, the kind defaults to `Tmux`; the `opencode` boolean does not infer a kind. Saving emits `kind`, not the old `cursor` key. Unknown kind strings are rejected. These are serialization compatibility rules, not a migration of the harness-specific owner/session files.

The shell discovers running kinds from tagged tmux sessions, this chat's Cursor owner files and opencode owner files (`workers::running_workers`). Close separately derives the kind from a Cursor owner file first, then an opencode owner or session file, otherwise `Tmux`; a worktree's `opencode.json` supplies cleanup metadata but does not by itself select `Opencode`. The monitor captures panes only for `Tmux`; checks for both headless kinds use markers and omit tmux attach hints (`chat/monitor.rs`, `core/src/worker_check.rs`).

## One host per harness, owned by the chat

`HeadlessHost: Send + Sync` (`crates/ralph/src/workers/headless.rs`) is the shared lifecycle interface, not a per-worker interface:

| Method | Contract | Current implementations |
| --- | --- | --- |
| `resume(&self)` | Resume persisted workers or supervise shared resources on a dedicated thread | `CursorHost` calls `resume_all` to load eligible ACP session records; `OpencodeHost` runs its server/event supervision loop (its constructor loads session records) |
| `poll(&self) -> bool` | Process queued worker requests; return false after host shutdown to end the poller | Each host consumes its own handoff directories; opencode also handles preflight requests |
| `shutdown(&self)` | Stop this host's resources at chat exit, never on an individual close or report | Cursor terminates its owned worker groups, including starts in progress; opencode stops the shared server process group and sweeps tracked descendants |

`chat::monitor::watch_headless` constructs `HeadlessShutdown` with an `Arc<dyn HeadlessHost>` for each of `CursorHost` and `OpencodeHost`. Each host gets a separate resume thread and a separate poll thread, sleeping `HANDOFF_POLL` (250 ms) between polls. Blocking ACP or HTTP handoffs therefore do not hold up the other host or the status/check loop.

The chat keeps the `HeadlessShutdown` guard alive across its input loop. Dropping it, including on an early error return, shuts down hosts in reverse construction order. `ExitCleanup` holds clones of the same hosts: on input loss it kills watches, calls each host's shutdown, then the input watcher exits the process. Shutdown must tolerate repeat calls. It is per host and chat-owned, not per worker: `close_worker` uses worker-specific close handoffs and never calls `HeadlessHost::shutdown`. Closing even the last opencode worker leaves the server available for the next spawn; a report does not stop either host.

## Adding the next headless harness

The shared trait unifies lifecycle orchestration only; it is not a plugin registry or a generic spawn/reply protocol. A future headless Claude or Codex adapter needs the following wiring, following the existing Cursor and opencode implementations:

1. Add the harness choice, argument validation and MCP schema in `crates/core/src/worker.rs`, and its preflight/spawn handoff in `crates/ralph/src/workers/spawn.rs`. Today `claude` still means tmux and `codex` is not accepted; neither has a headless host.
2. Add a hosting kind if it uses a new hosting mechanism. Update both `WorkerKind`'s serialized variants and its compatible deserializer; preserve the old boolean mapping and default. Extend running-worker discovery, ownership, reply/nudge/permission routing and close-kind detection in `workers.rs`, plus pane/check handling in `chat/monitor.rs` and `core/src/worker_check.rs`. These paths still have explicit harness/kind branches.
3. Implement `HeadlessHost` for the adapter and register one host in `HeadlessShutdown::new`. Keep resume/supervision and request polling independent, honor the closed state and make shutdown safe to repeat. No new harness-specific lifecycle loop or exit guard belongs in chat orchestration.
4. Keep worker close separate from host shutdown. Extend the pure `close_plan`/`CloseStep` dispatch in `core/src/worker/ownership.rs` and the shell's step executor only as needed. Shared resources must survive another worker's close. Keep home-directory cleanup explicitly scoped by kind, rather than treating every non-tmux worker as Cursor.
5. Reuse the worktree, marker and `ralph-worker` report contracts. Add pure/unit coverage for serialization compatibility, close isolation, discovery/check behavior and lifecycle dispatch, following the existing tests; update the harness details and cleanup docs alongside the adapter.
