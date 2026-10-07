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
| `ask_human` | `question` | `AskHuman`; reply text is the human answer |
| `end_meeting` | `summary` | `EndMeeting` |

Invalid arguments return a tool result with `isError: true`. Unknown methods return JSON-RPC `-32601`. `send_message` success tells the model to end its turn.

## Layout

- `crates/core/src/meeting.rs`: pure types, tool schemas, validation, `handle_request` dispatch (`Outcome::{Silent, Respond, Relay}`), `finish_relay`, `build_mcp_args`.
- `crates/core/src/meeting_turn.rs`: pure turn state machine, plan validation, scene prompt.
- `crates/ralph/src/meeting_mcp.rs`: stdio loop, socket client `relay`.
- `crates/ralph/src/meeting.rs`: moderator shell (listener, turn loop).

## CLI and turn loop

```bash
ralph meeting --first pm --personas pm,dev,qa [--model dev=sonnet] [--max-turns 20] "<kickoff>"
```

- `MeetingPlan::new` (core, `meeting_turn.rs`) validates: at least two unique personas, `--first` in the roster, `--model persona=model` personas in the roster, `--max-turns >= 1`. Agent files are checked in the shell.
- The kickoff is turn 1 for `--first`. Each turn runs `claude --agent <persona>` via `build_command` with `--mcp-config`, `--model` (only if given), `--resume <session id>` after the persona's first turn, and a scene prompt that replaces the persona directive suffix in `--append-system-prompt`.
- A listener thread forwards socket events to the main loop, which answers them with `TurnState::on_event` while the speaker process runs. The first `send_message` per turn is recorded as the handoff; later ones get an error reply. `ask_human` opens the editor and returns the answer.
- After the process exits, `TurnState::finish_turn` returns `Step::Resume { persona, message }` or `Step::Stop(StopReason)` (`EndMeeting`, `TurnCap`, `NoHandoff`, `Error`). Each handoff prints `speaker -> recipient`.
- Not built yet: transcript display and persistence (GUZ-377), human routing (GUZ-378), full docs (GUZ-379).
