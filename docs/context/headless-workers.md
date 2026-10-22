# Headless workers

Worker hosting and chat-owned lifetimes are separate from the selected agent. For spawn, reply, reporting and close behavior, see [ralph chat](ralph-chat.md#mcp-tools); for home-directory cleanup, see [Worker leftovers](worker-leftovers.md).

## HarnessId and compatibility

`HarnessId` (`crates/core/src/worker/kind.rs`) selects the agent and describes how its worker is hosted:

| Harness | Name | Hosting |
| --- | --- | --- |
| `Claude` | `claude` (default) | One tmux session per worker |
| `Cursor` | `cursor` | One `agent acp` child per worker, owned by `CursorHost` |
| `Opencode` | `opencode` | One session per worker on the chat's shared `opencode serve`, owned by `OpencodeHost` |
| `Codex` | `codex` | One `codex app-server --listen stdio://` child per worker |

`HarnessId` replaces the old Cursor boolean in `RunningWorker`, `WorkerObservation` and `CloseState`, and spawn commands carry it. The separate `opencode` boolean remains as agent/session metadata, not the hosting discriminator. For example, `close_plan` can remove an old opencode session before closing a claude worker when that metadata is present.

`RunningWorker.kind` serializes as `"kind": "claude"`, `"cursor"`, `"opencode"` or `"codex"`. Its serde `alias = "cursor"` and `HarnessId`'s custom deserializer let old records with `"cursor": true` load as `Cursor`, `"cursor": false` as `Claude`, and the old `"tmux"` kind string as `Claude`. If neither field is present, the kind defaults to `Claude`; the `opencode` boolean does not infer a kind. Saving emits `kind`, not the old `cursor` key. Unknown kind strings are rejected. These are serialization compatibility rules, not a migration of the harness-specific owner/session files.

The shell discovers running kinds from tagged tmux sessions, this chat's Cursor owner files and opencode owner files (`workers::running_workers`). Close separately derives the kind from a Cursor owner file first, then an opencode owner or session file, otherwise `Claude`; a worktree's `opencode.json` supplies cleanup metadata but does not by itself select `Opencode`. The monitor captures panes only for `Claude`; checks for both headless harnesses use markers and omit tmux attach hints (`chat/monitor.rs`, `core/src/worker_check.rs`).

## One host per harness, owned by the chat

`HeadlessHost: Send + Sync` (`crates/ralph/src/workers/headless.rs`) is the shared lifecycle interface, not a per-worker interface:

| Method | Contract | Current implementations |
| --- | --- | --- |
| `resume(&self)` | Resume persisted workers or supervise shared resources on a dedicated thread | `CursorHost` calls `resume_all` to load eligible ACP session records; `OpencodeHost` runs its server/event supervision loop (its constructor loads session records) |
| `poll(&self) -> bool` | Process queued worker requests; return false after host shutdown to end the poller | Each host consumes its own handoff directories; opencode also handles preflight requests |
| `shutdown(&self)` | Stop this host's resources at chat exit, never on an individual close or report | Cursor terminates its owned worker groups, including starts in progress; opencode stops the shared server process group and sweeps tracked descendants |

`chat::monitor::watch_headless` constructs `HeadlessShutdown` with an `Arc<dyn HeadlessHost>` for each of `CursorHost` and `OpencodeHost`. Each host gets a separate resume thread and a separate poll thread, sleeping `HANDOFF_POLL` (250 ms) between polls. Blocking ACP or HTTP handoffs therefore do not hold up the other host or the status/check loop.

Cursor's `AcpWorker` and opencode's per-session `OpencodeSession` implement the core `HeadlessWorker` port (`capabilities`, `send`, `poll`). `send` runs `decide` first, then maps `Prompt`, `AnswerPermission` and `Close`; `Spawn`, `Resume` and `Interrupt` return an error because the host handles them (interrupt keeps the inherent `AcpWorker::interrupt`, which needs the result entry id). Turn start, turn end, permission changes and close push `WorkerEvent`s to a per-worker sink. Each 250 ms `CursorHost::poll` drains every worker, runs `reduce` and applies the effects: `blocked` and `waiting` markers are set and cleared there, and close clears `ready`, `blocked` and `waiting`. `unreported` and its `waiting` write stay direct; the `ready` write at spawn stays direct; `ReplyText` effects are ignored. `OpencodeSession` is a short-lived view built per request by `OpencodeHost::worker` (the API handle, the front pending permission and the session's shared event sink). Its `send` maps `Prompt`, `Interrupt` (full: a synchronous `POST /api/session/{id}/interrupt`, whose `{"interrupted":bool}` body becomes `WorkerEvent::Interrupted`; a failed call warns and yields outcome `None`), `AnswerPermission` and `Close` (`DELETE /api/session/{id}`, then `Closed`); `Spawn` and `Resume` return the host-handles-it error. `OpencodeHost::run` turns each `Effect` into events (`Ready`, `TurnStarted`, `PermissionAsked`, `PermissionSettled`) and `apply_events` polls them, runs `reduce` and applies the marker effects at once; `reported` clear, `unreported` and its `waiting` write, and the ready write at spawn stay direct. The interrupt outcome reaches chat through `<store>/opencode-interrupt-result/<id>/<entry>` (`cancelled` or `sent`), awaited by `Workers::reply` like Cursor's; a failed interrupt writes nothing, so chat reports the outcome unknown after 30 s. ACP `decode_update` emits `WorkerEvent::Activity { at }` for every `session/update` frame (plus `Reported` for the report tool call), and the notification handler writes it to the activity marker through `Heartbeat::touch_at`. opencode's `session_activity` does the same for every session-tagged SSE event; `OpencodeWorker::route` writes it through `touch_at` (turn start and end still call `set_running`).

The chat keeps the `HeadlessShutdown` guard alive across its input loop. Dropping it, including on an early error return, shuts down hosts in reverse construction order. `ExitCleanup` holds clones of the same hosts: on input loss it kills watches, calls each host's shutdown, then the input watcher exits the process. Shutdown must tolerate repeat calls. It is per host and chat-owned, not per worker: `close_worker` uses worker-specific close handoffs and never calls `HeadlessHost::shutdown`. Closing even the last opencode worker leaves the server available for the next spawn; a report does not stop either host.

## Adding the next headless harness

The shared trait unifies lifecycle orchestration only; it is not a plugin registry or a generic spawn/reply protocol. A future headless Claude or Codex adapter needs the following wiring, following the existing Cursor and opencode implementations:

1. Add the harness choice, argument validation and MCP schema in `crates/core/src/worker.rs`, and its preflight/spawn handoff in `crates/ralph/src/workers/spawn.rs`. Today `claude` still means tmux. `codex` is accepted and has a capability row, but its spawn arms in `workers/spawn.rs` return "codex harness not implemented yet" until the adapter exists; no headless host serves it.
2. Add a `HarnessId` variant if it uses a new hosting mechanism. Update both `HarnessId`'s serialized variants and its compatible deserializer; preserve the old boolean mapping and default. Extend running-worker discovery, ownership, reply/nudge/permission routing and close-kind detection in `workers.rs`, plus pane/check handling in `chat/monitor.rs` and `core/src/worker_check.rs`. These paths still have explicit harness/kind branches.
3. Implement `HeadlessHost` for the adapter and register one host in `HeadlessShutdown::new`. Keep resume/supervision and request polling independent, honor the closed state and make shutdown safe to repeat. No new harness-specific lifecycle loop or exit guard belongs in chat orchestration.
4. Keep worker close separate from host shutdown. Extend the pure `close_plan`/`CloseStep` dispatch in `core/src/worker/ownership.rs` and the shell's step executor only as needed. Shared resources must survive another worker's close. Keep home-directory cleanup explicitly scoped by kind, rather than treating every non-tmux worker as Cursor.
5. Reuse the worktree, marker and `ralph-worker` report contracts. Add pure/unit coverage for serialization compatibility, close isolation, discovery/check behavior and lifecycle dispatch, following the existing tests; update the harness details and cleanup docs alongside the adapter.

## Capability probes

Every Cursor and opencode cell of the capability matrix (`Capabilities::support` in `crates/core/src/worker/headless.rs`) is tagged `[probed]`, with its probe note, `[corrected]`, meaning a probe found the matrix text wrong, or `[unverified]`, meaning no probe backs it yet. The 2026-10-22 probes ran the installed ralph (`f45be36`) in a throwaway repo and store with `RALPH_CONFIG_DIR` pointing at a test config.

### Cursor probe (cursor-agent 2026.10.14, 2026-10-22)

| Feature | Matrix value | Tag | Probe note |
| --- | --- | --- | --- |
| `spawn` | Full | [probed] | `spawn_worker` started `agent acp` in a new worktree and wrote `ready/<id>` |
| `prompt` | Full | [probed] | A `reply` after a chat restart ran as the next `session/prompt` |
| `interrupt` | Partial: outcome unknown after 30 s | [probed] | Mid-tool `session/cancel` returned `stopReason: cancelled` within 1 s and chat reported the turn cancelled. The 30 s unknown path was not hit |
| `perm-ask` | Full | [probed] | With `[cursor] permissions = "ask"` a shell call raised `session/request_permission`, `blocked/<id>` and one `blocked` inbox report; the `allow-once` reply resumed the turn |
| `perm-yolo` | Partial: global config, not per spawn | [probed] | `[cursor] permissions = "yolo"` is read at chat start and needed a restart; every request was then answered `allow-once` in the same second with no `blocked` marker |
| `resume` | Partial: only if agent supports session/load | [probed] | After a chat restart the host sent `session/load` and the worker recalled its earlier command |
| `model` | Full | [probed] | `composer-2.5` became `session/set_config_option` `composer-2.5[fast=true]`; the model keys of `~/.cursor/cli-config.json` were back to the user's default afterwards |
| `report` | Full | [probed] | `report` reached the inbox and wrote `reported/<id>` |
| `close` | Partial: cli-config.json not restored | [corrected] | Stale since the model-default restore: close removed the sessions, worktree, branch, `~/.cursor/projects/<worktree>` and `~/.cursor/acp-sessions/<session>`, and `cli-config.json` matched its backup except Cursor's own `privacyCache.updatedAt` |
| `heartbeat` | Partial: ACP traffic only; exit seen by polling the child every 250 ms | [probed] | `kill -9` of `agent acp` wrote `gone/<id>` within 1 s and posted one Dead message. Its `worker-server` child stayed alive with parent pid 1 |

### opencode probe (opencode 2.0.29, 2026-10-19 and 2026-10-22)

| Feature | Matrix value | Tag | Probe note |
| --- | --- | --- | --- |
| `spawn` | Full | [probed] | `spawn_worker` created `ses_ralph<id>` on the chat's shared server. A model whose provider auth fails (`anthropic/...` with an invalid OAuth token) ends the first turn with a `blocked` report naming the error |
| `prompt` | Full | [probed] | Mid-turn prompt: HTTP 200. The server serializes; the second prompt runs after the first ends (SSE labels it `session.inbox.*` with `"delivery":"steer"`). A `reply` after a chat restart also ran |
| `interrupt` | Full | [probed] | 2026-10-19 only. `POST /api/session/{id}/interrupt` is synchronous. It returns 200 `{"interrupted":true}` mid-turn, `{"interrupted":false}` when idle, and 404 `SessionNotFoundError` for an unknown session. SSE then sends `session.execution.interrupted` |
| `perm-ask` | Partial: only prompts the server raises despite `allow-*` | [probed] | Under `allow-*` session permissions a shell command and a file write outside the project ran, with 0 `permission.*` events (seen again on 2026-10-22) |
| `perm-yolo` | Partial: always on | [probed] | A worker wrote a file outside its worktree with no `blocked` marker; ralph has no opencode permission setting |
| `resume` | Full | [probed] | After a chat restart a new `opencode serve` started and a `reply` to the same `ses_ralph<id>` recalled the earlier command |
| `model` | Full | [probed] | The session record keeps the spawn model; `opencode/claude-haiku-5-5` reported itself as `claude-haiku-5-5`, and `anthropic/...` failed on Anthropic auth, so the provider follows the spawn model |
| `report` | Full | [probed] | `report` reached the inbox and wrote `reported/<id>` |
| `close` | Partial: no home cleanup; shared server kept | [probed] | Close removed the session, worktree and branch; the server kept its pid after the last opencode worker closed |
| `heartbeat` | Partial: per-session SSE activity; one alert per shared-server death | [probed] | The SSE comment `: heartbeat` arrives every 15 s with no session id. Server death (`kill -9`, 2026-10-19): SSE truncates with no close event; HTTP is refused |

### Codex probe (codex-cli 0.162.1, 2026-10-22)

The Codex matrix row in `Capabilities::support` is derived from these probes, and there is no adapter yet (spawn returns "codex harness not implemented yet"). They checked the GUZ-642 proposal. A Python JSON-RPC driver ran `codex app-server --listen stdio://` in a throwaway git repo, on `gpt-5.6-luna` at `low` effort with `approvalsReviewer: "user"` on every thread. The real `ralph worker-mcp --id --inbox --owner --worktree` server was attached through the thread's runtime `config.mcp_servers`. Grades: VERIFIED means the proposal held, CORRECTED means a probe contradicted it, and NOT-TESTED means no probe reached it.

| Feature | Proposed | Grade | Probe note |
| --- | --- | --- | --- |
| `spawn` | Full | VERIFIED | `initialize` answered in 0.4 s and `thread/start` in 0.1-0.7 s. `mcpServer/startupStatus/updated` showed `ralph-worker` going from `starting` to `ready` 0.65 s after the `thread/start` response. `thread/start` accepted the unknown model `no-such-model-xyz`. With an empty `CODEX_HOME`, `thread/start` also succeeded. The first turn then sent `error` frames (401, `Reconnecting... n/5`, `willRetry: true`), and `account/read` returned `account: null, requiresOpenaiAuth: true` |
| `prompt` | Full: a reply during a turn runs once as the next turn | CORRECTED | `turn/start` during a running turn returned the running turn's id and steered its text into that turn. `UserPromptSubmit` fired again, only one `turn/completed` followed, and the final answer covered only the second prompt. The adapter must hold replies until `turn/completed` |
| `interrupt` | Partial: timeout means Unknown | CORRECTED | `turn/interrupt` returned `{}`, then `turn/completed` arrived with `status: interrupted` within 0.02 s, both running and blocked. A blocked turn's pending approval was settled by Codex (`serverRequest/resolved`), and a later answer was ignored. A running `sleep 61` kept running after the interrupt (in its own process group), and its `item/completed` arrived 60 s later with the old `turnId`. When idle, the server returned `-32600 no active turn to interrupt`. One interrupt sent while an interrupted command still ran got no reply for 232 s, until the next turn ended |
| `perm-ask` | Partial: protocol-raised requests only | VERIFIED | With `approvalPolicy: "untrusted"`, a shell call raised `item/commandExecution/requestApproval` with `availableDecisions: [accept, acceptWithExecpolicyAmendment, cancel]` (no `acceptForSession` offered). A file write raised `item/fileChange/requestApproval` with no decision list. Under `on-request` with `workspace-write`, workspace commands ran unasked, and only the network `curl` asked. Each MCP tool call raised `mcpServer/elicitation/request` (`_meta.codex_approval_kind: mcp_tool_call`, `persist: [session, always]`). `item/permissions/requestApproval` never fired, even under a `granular` policy with `request_permissions: true` |
| `perm-yolo` | Partial: single-request accepts only | VERIFIED | `accept` (no persist) on a command, a file change and an elicitation applied to that request only; the next command asked again. A JSON-RPC error reply rejected the command (`Rejected("approval request failed")`) and the turn went on, so unknown requests fail closed. Turn or session grants were NOT-TESTED, because no `item/permissions/requestApproval` was raised |
| `resume` | Partial: needs history, worktree and login | VERIFIED | After `kill -9`, a new child's `thread/resume` re-attached the MCP servers. The next turn recalled a code word from before the kill. The report showed up only inside the resume response's `turns` and was not posted again, so the inbox was unchanged. An unknown id gave `-32600 no rollout found for thread id …`. With no `model`, resume restored the thread's model and effort; on a thread killed before its first turn ended, it fell back to the global default. With `model` but no `effort`, effort became the global `high`. Every resume sent `deprecationNotice` asking for `excludeTurns: true` |
| `model` | Full | VERIFIED | `model/list` returned 7 models with `supportedReasoningEfforts` and `isDefault`. `thread/start` does not validate the model, so the adapter must check against `model/list`. `turn/start` takes `effort` |
| `report` | Full | VERIFIED | Each `report` call first raised an elicitation. An error reply declined it: the inbox stayed empty while the agent still replied "REPORTED". With the runtime thread config `mcp_servers.ralph-worker.default_tools_approval_mode = "approve"`, there was no elicitation, the report reached the inbox and `reported/<id>` was written. Delivery shows as `item/completed` with `mcpToolCall`, `server: ralph-worker`, `tool: report`, `status: completed`, result `Delivered. …`. A missing owner file gave the tool text `Could not deliver: worker closed.` |
| `close` | Partial: global config not restored | CORRECTED | Closing stdin ended the app-server (exit 0 in 0.02 s), and its MCP children exited. Every child (MCP servers, `codex-code-mode-host`, each tool command) runs in its own process group. After `kill -9` of the app-server, stdout hit EOF in 0.01 s and the MCP children exited, but a running `sleep 299` was left with parent pid 1. Killing the app-server's group therefore misses tool processes. The first `thread/start` wrote `[projects."/private/tmp/guz643/repo"] trust_level = "trusted"` to `~/.codex/config.toml` (GUZ-585 class). History stays in `~/.codex/sessions` rollouts and the sqlite state |
| `heartbeat` | Partial: silent tools can cause false Quiet | CORRECTED | Codex runs long shell calls as background sessions and polls them. During `sleep 200`, 35 frames arrived with a longest gap of 30.0 s (`item/commandExecution/terminalInteraction` plus reasoning items, which use model tokens). A `sleep 25` sent no frames. An idle thread sent 0 frames in 45 s and in 120 s. Child exit shows as stdout EOF |

Workers inherit the global Codex setup. Every `~/.codex/hooks.json` hook fired: `SessionStart` (the tmux notifier and `rex __internal session-start --agent codex`), `UserPromptSubmit`, `PermissionRequest` (also for MCP elicitations) and `Stop`. Each hook run arrives as `hook/started` and `hook/completed` frames. `mcp_servers.mcptools` and the built-in `codex_apps` start on every thread, and `~/.codex/AGENTS.md` is listed in `instructionSources`. The probe unset `REX_*` and `TMUX` only to keep the dispatching chat's session out of the hooks; a real worker inherits the chat's environment.

`--listen stdio://` runs in process. The child was the 0.162.1 `codex` with only unnamed socket pairs open, and every tool and MCP process was its own child. The running 0.163.0 app-server daemon was not involved. The probes used no `ClaudeHeadless` code, so the Codex adapter does not depend on GUZ-634.

### Quiet periods (2026-10-22)

A 1 s poll of `activity/<id>` measured the longest silence while a worker was `running`. `[chat] worker_quiet_secs = 60` with `worker_check_secs = 10` posted `quiet 6x s; may be stuck on a prompt` once per silence and re-armed after new activity.

| Harness | Turn | Longest silence |
| --- | --- | --- |
| Cursor | One shell call `sleep 240` | 242 s: no ACP traffic while a tool runs |
| Cursor | First turn of the first spawn, prompt to first update | 132 s (a second spawn: 4 s) |
| Cursor | After a permission answer | 81 s |
| Cursor | Tool-free turn after resume | 2 s |
| opencode | One shell call `sleep 240` | 240 s: no session SSE event while a tool runs |
| opencode | Tool-free turn (2026-10-19) | 88 s |
| opencode | Tool-free turn after resume | 6 s |

Neither harness sends activity while a tool call runs, so silence equals the tool's run time. The 300 s default stays: it is above every observed silence outside tool calls (132 s at most) by more than 2x, and a tool call longer than 300 s costs one `check` message per episode.
