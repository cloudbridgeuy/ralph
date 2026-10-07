# Meeting MCP Server

Private MCP server that meeting personas (separate `claude -p` sessions) use to reach the moderator process. Tickets GUZ-375 (MCP server) and GUZ-376 (turn loop).

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

Invalid arguments return a tool result with `isError: true`. Unknown methods return JSON-RPC `-32601`. `send_message` success tells the model to end its turn.

## Layout

- `crates/core/src/meeting.rs`: pure types, tool schemas, validation, `handle_request` dispatch (`Outcome::{Silent, Respond, Relay}`), `finish_relay`, `build_mcp_args`.
- `crates/core/src/meeting_turn.rs`: pure turn state machine, plan validation, scene prompt, ask_human editor template and answer parsing.
- `crates/ralph/src/meeting_mcp.rs`: stdio loop, socket client `relay`.
- `crates/ralph/src/meeting.rs`: moderator shell (listener, turn loop).

## CLI and turn loop

```bash
ralph meeting --first pm --personas pm,dev,qa [--model dev=sonnet] [--max-turns 20] "<kickoff>"
```

- `MeetingPlan::new` (core, `meeting_turn.rs`) validates: at least two unique personas, `--first` in the roster, `--model persona=model` personas in the roster, `--max-turns >= 1`. Agent files are checked in the shell.
- The kickoff is turn 1 for `--first`. Each turn runs `claude --agent <persona>` via `build_command` with `--mcp-config`, `--model` (only if given), `--resume <session id>` after the persona's first turn, and a scene prompt that replaces the persona directive suffix in `--append-system-prompt`.
- A listener thread forwards socket events to the main loop, which answers them with `TurnState::on_event` while the speaker process runs. The first `send_message` per turn is recorded as the handoff; later ones get an error reply. `ask_human` pauses the spinner (`SpinnerSubprocessConfig.pause`), opens the editor, and returns the answer.
- After the process exits, `TurnState::finish_turn` returns `Step::Resume { persona, message }` or `Step::Stop(StopReason)` (`EndMeeting`, `TurnCap`, `HumanEnded`, `NoHandoff`, `Error`). Each turn starts with a persona banner and each handoff prints a route line `▶ from → message → to  [turn/max]`.
- Transcript (`ralph_core::meeting_transcript`): each line is appended to `<data_dir>/sessions/meeting-<YYYYmmdd-HHMMSS>/transcript.jsonl` as it happens, one JSON object `{from, to, text, at}` (`at` is UTC RFC 3339). Recorded: kickoff (`human` to first speaker), accepted `send_message` (rejected duplicates are not), `ask_human` question and answer (`to` lists the asker plus valid extra recipients, comma separated), `end_meeting` summary (`speaker` to `all`). The path prints when the meeting stops; write failures warn and the meeting continues.
- `ask_human` routing: the editor labels the question with the asking persona and prefills `to: <asker>` below the separator. The human may extend it (`to: alice, bob`). `parse_human_answer` (core) reads the first line below the separator; a missing `to:` line means asker only. The asker always receives the answer as the tool result. Unknown names are skipped with a warning. An empty answer (editor abort, or only a `to:` line or whitespace) ends the meeting: `parse_human_answer` returns `None`, `TurnState::end_by_human` makes `finish_turn` return `StopReason::HumanEnded` ahead of any handoff or turn cap, the asker is told to stop, and nothing is routed. The transcript gets the question and then an `end_line` from `human` to `all`. The meeting sets `SpinnerSubprocessConfig.cancel`, so the subprocess loop kills the persona's process group and returns the partial result; `finish_turn` still returns `HumanEnded` first. Editor errors still reply "Could not reach the human" and do not end the meeting.
- Deferred delivery: `TurnState::route_answer` stores a note ("The human answered <asker>'s question." with question and answer) per extra persona. `finish_turn` prepends the note to the message when that persona next receives a turn; notes are consumed once.
- Not built yet: full docs (GUZ-379).
