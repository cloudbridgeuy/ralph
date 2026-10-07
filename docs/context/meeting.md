# ralph meeting Command

`ralph meeting` moderates a turn-based conversation between personas in separate `claude -p` sessions. A private MCP server lets each persona send messages, ask the human, save shared office memory, or end the meeting.

## Flow

1. Moderator (`ralph meeting`) binds a Unix socket in a tempdir and builds per-persona args with `ralph_core::meeting::build_mcp_args(exe, socket, caller, personas)`, which returns `--mcp-config <json> --strict-mcp-config`.
2. `claude` spawns the hidden `ralph meeting-mcp --socket <path> --caller <persona> --personas a,b,c` and speaks newline-delimited JSON-RPC 2.0 (MCP stdio) with it. Logs go to stderr only.
3. Each valid `tools/call` becomes a `MeetingEvent` JSON line sent over the socket. The process blocks for one `MeetingReply` JSON line, which becomes the tool result.

## Tools

| Tool | Args | Event |
| --- | --- | --- |
| `send_message` | `to`, `text` | `SendMessage`; `to` must be a roster persona other than the caller |
| `ask_human` | `question` | `AskHuman`; reply text is the human answer (see ask_human routing) |
| `end_meeting` | `summary` | `EndMeeting` |
| `remember` | `text` | `Remember`; append durable knowledge to shared project office memory without ending the turn |

Tool arguments must be non-empty strings. Invalid tool arguments return a tool result with `isError: true`; a missing tool name returns JSON-RPC `-32602`. Unknown methods return JSON-RPC `-32601`. `send_message` success tells the model to end its turn.

## Layout

- `crates/core/src/meeting.rs`: pure types, tool schemas, validation, `handle_request` dispatch (`Outcome::{Silent, Respond, Relay}`), `finish_relay`, `build_mcp_args`.
- `crates/core/src/meeting_turn.rs`: pure turn state machine, plan validation, scene prompt, ask_human editor template and answer parsing.
- `crates/core/src/meeting_transcript.rs`: transcript line types, endpoint mapping, JSONL serialization.
- `crates/ralph/src/meeting_mcp.rs`: stdio loop, socket client `relay`.
- `crates/ralph/src/meeting.rs`: moderator shell (listener, turn loop, human routing, transcript writes).
- `crates/core/src/meeting_memory.rs`: pure note normalization, wake/cursor recovery, wake context, nap prompt and output decisions.
- `crates/ralph/src/meeting_memory.rs`: project-local append, atomic wake persistence, Haiku curator invocation.
- `crates/ralph/src/cli/mod.rs` and `crates/ralph/src/main.rs`: CLI arguments, agent-file checks, command dispatch.
- `crates/ralph/src/invoke.rs`: command construction and persona isolation flags.

## CLI and turn loop

```bash
ralph meeting --first pm --personas pm,dev,qa [--model dev=sonnet] [--max-turns 20] "<kickoff>"
```

- Required: `--first`, comma-separated `--personas`, and positional kickoff. `--model persona=model` is repeatable, once per persona. `--max-turns` defaults to 20; `--timeout` defaults to 600 seconds per turn. `--theme <NAME>` and `--no-background` control syntax highlighting.
- `MeetingPlan::new` (core, `meeting_turn.rs`) validates: at least two unique personas, `--first` in the roster, non-empty `persona=model` entries with no duplicate model assignment, model personas in the roster, `--max-turns >= 1`. Agent files are checked in the shell.
- The kickoff is turn 1 for `--first`. Each turn runs `claude --agent <persona>` via `build_command` with `--mcp-config`, `--model` (only if given), `--resume <session id>` when a prior turn returned that persona's session ID, and a scene prompt that replaces the persona directive suffix in `--append-system-prompt`.
- A listener thread forwards socket events to the main loop, which answers them with `TurnState::on_event` while the speaker process runs. Events from a non-speaker are refused. The first `send_message` per turn is recorded as the handoff; later ones are refused by the state machine. The MCP relay currently replaces every `send_message` socket reply with a delivery confirmation, so these refusals are not exposed to the caller. `ask_human` pauses the spinner (`SpinnerSubprocessConfig.pause`), opens the editor, and returns the answer without handing off the turn.
- After the process exits, `TurnState::finish_turn` returns `Step::Resume { persona, message }` or `Step::Stop(StopReason)` (`EndMeeting`, `TurnCap`, `HumanEnded`, `NoHandoff`, `Error`). Each turn starts with a persona banner and each handoff prints a route line `▶ from → message → to  [turn/max]`.
- Transcript (`ralph_core::meeting_transcript`): each line is appended to `<data_dir>/sessions/meeting-<YYYYmmdd-HHMMSS>/transcript.jsonl` as it happens, one JSON object `{from, to, text, at}` (`at` is UTC RFC 3339). Recorded: kickoff (`human` to first speaker), accepted `send_message` (rejected duplicates are not), `ask_human` question and answer (`to` lists the asker plus valid extra recipients, comma separated), `end_meeting` summary (`speaker` to `all`). `<data_dir>` uses `RALPH_DATA_DIR` if set, otherwise the platform data directory plus `ralph`. The path prints on a normal turn-loop stop, but not when a subprocess error propagates. Directory creation errors abort startup; append failures warn and the meeting continues.
- `ask_human` routing: the editor labels the question with the asking persona and prefills `to: <asker>` below the separator. The human may extend it (`to: alice, bob`). `parse_human_answer` (core) reads the first line below the separator; a missing `to:` line means asker only. The asker always receives the answer as the tool result. Unknown names are skipped with a warning. An empty answer (editor abort, or only a `to:` line or whitespace) ends the meeting: `parse_human_answer` returns `None`, `TurnState::end_by_human` makes `finish_turn` return `StopReason::HumanEnded` ahead of any handoff or turn cap, the asker is told to stop, and nothing is routed. The transcript gets the question and then an `end_line` from `human` to `all`. The meeting sets `SpinnerSubprocessConfig.cancel`, so the subprocess loop kills the persona's process group and returns the partial result; `finish_turn` still returns `HumanEnded` first. Editor errors still reply "Could not reach the human" and do not end the meeting.
- Deferred delivery: `TurnState::route_answer` stores a note ("The human answered <asker>'s question." with question and answer) per extra persona. `finish_turn` prepends the note to the message when that persona next receives a turn; notes are consumed once.

## Persona isolation

- Each persona has its own session ID, held in memory by the moderator and resumed only for that persona within this meeting. Plain-text output is shown to the human, not routed to other personas; only tool messages and routed human answers are delivered. Every persona sees the roster and original kickoff in the scene prompt.
- `build_command` uses `--setting-sources project,local`, excludes `~/.claude/CLAUDE.md` through `claudeMdExcludes` when the home directory is available, and blocks the `Agent` tool with `--disallowed-tools Agent`. Project and local settings remain active; this is context isolation, not a filesystem sandbox.
- `--strict-mcp-config` limits configured MCP servers to this meeting's server. The meeting scene prompt replaces the normal persona directive suffix; the meeting loop does not scan output for `<ralph-*>` orchestration directives.

## Shared office memory

- One office memory serves all personas and future meetings. The project root follows existing CLI semantics: the directory where `ralph meeting` starts, not an automatically discovered Git root. Start meetings from the same project directory. Storage is `.ralph/memory/`, ignored by the repository's root `/.ralph/` rule. There is no memory CLI and no automatic transcript extraction.
- `remember(text)` is explicit capture. Whitespace becomes single spaces; empty notes are rejected. Each accepted note appends one line to `LOG.txt`, which is never rewritten. Capture checks caller identity and active turn. Rejected capture and append failures return MCP `isError: true`. Success does not hand off the turn; the persona must still call `send_message` or `end_meeting`.
- At startup, every persona receives the same wake snapshot in its scene prompt: prior compact summary plus pending raw notes. Memory is JSON-encoded and labeled as knowledge, not instructions. New notes become wake context at the next meeting.
- `WAKE.json` contains both `summary` and `cursor` (the number of log lines already compressed). They are persisted together by atomic replacement, so the cursor cannot advance independently of its summary. Missing wake means an empty prior summary; all log notes remain pending. Invalid wake/cursor, a cursor ahead of the log, or a missing matching summary warns and replays all available raw notes. Read failures warn and continue, retaining readable raw notes where possible. Invalid UTF-8 is shown with replacement characters but is not compressed.
- After the turn loop stops, including subprocess failure, pending notes trigger a fresh `claude -p` curator with fixed `--model haiku`. It merges the prior summary and pending notes; successful non-empty output replaces `WAKE.json`. The curator uses the meeting timeout, disables tools and configured MCP servers, and uses the same global-instruction/settings isolation as personas while retaining project/local settings. It has no persona or resumed session.
- Nap failure warns, does not fail the meeting, and leaves LOG and prior wake/cursor unchanged. Short file locks serialize reads/appends/commits across meetings. If another nap changes WAKE during curation, this nap does not overwrite it; pending notes remain available for the next meeting. Notes appended during curation remain pending beyond the committed cursor. This is local context isolation, not a filesystem sandbox; project/local hooks and instructions remain active.
- Storage rejects symlinks in `.ralph`, its memory directory, and memory files, and rejects projects inside `~/.optmem`. On Unix, storage also rejects multiply linked memory files. These checks do not protect against hostile concurrent filesystem changes.
