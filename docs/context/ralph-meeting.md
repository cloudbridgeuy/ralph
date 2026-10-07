# Meeting MCP Server

Private MCP server that meeting personas (separate `claude -p` sessions) use to reach the moderator process. Ticket GUZ-375. The moderator turn loop is GUZ-376.

## Flow

1. Moderator binds a Unix socket and builds per-persona args with `ralph_core::meeting::build_mcp_args(exe, socket, caller, personas)`, which returns `--mcp-config <json> --strict-mcp-config`.
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
- `crates/ralph/src/meeting_mcp.rs`: stdio loop, socket client `relay`.

The moderator-side listener comes with GUZ-376. Wire protocol: one JSON `MeetingEvent` line in, one JSON `MeetingReply` line out.
