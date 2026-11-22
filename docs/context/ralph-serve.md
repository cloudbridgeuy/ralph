# ralph serve Command (experimental, hidden)

`ralph serve --store <DIR>` keeps one chat store open in a long-lived process and listens on a private Unix socket. It does not show in `ralph --help`. A client greets it, sends text to the main chat or to thread sessions, runs thread operations, and asks it to stop.

## Code Locations

| Component | File | Description |
|-----------|------|-------------|
| CLI args | `crates/ralph/src/cli/mod.rs` | hidden `Serve(ServeArgs)`, required `--store <DIR>` |
| Shell | `crates/ralph/src/serve/` | `run()`, accept loop, `connection.rs` (`classify()`, thread ops), `open.rs` (sessions), `sink.rs` (socket sink) |
| Signals | `crates/ralph/src/signal.rs` | `init_serve()`: SIGINT, SIGTERM and SIGHUP set the stop flag |
| Store gate, refusal | `crates/core/src/serve.rs` | `serve_store_gate()`, `serve_refusal()` |
| Supervisor | `crates/core/src/serve_supervisor.rs` | pure state machine: `Supervisor::start()`, `step()`, `Input`, `Effect` |
| Socket path | `crates/core/src/chat_socket.rs` | `serve_socket_path()` |
| Socket | `crates/ralph_server/src/chat/serve_socket.rs` | `ServeSocket::bind()`, `accept()` with the peer uid check |
| CLI arg | `crates/ralph/src/cli/mod.rs` | hidden `ralph chat --connect <SOCK>` |
| Client | `crates/ralph/src/chat/remote.rs` | `RemoteSession` (`SessionPort` over the socket), `run()`, `thread_request()`, `status_summary()` |
| Frames | `crates/ralph_server/src/chat/serve_proto.rs` | `Envelope`, `ClientMsg`, `ThreadCmd`, `ServerMsg`, `hello_reply()` |

## Store

- The store comes only from `--store`. A relative path is joined to the current directory.
- `serve` refuses to start when `RALPH_CHAT_STORE` is set and not empty. Unset it first.
- `serve` claims the store's `CHAT` lease (the same flock as `ralph chat`). When a chat or another serve holds it, `serve` exits with `ralph serve: store <DIR> is in use by pid <N>`. The lease `generation` is not used as a fence.

## Socket

- Path: `/tmp/ralph-<uid>/serve-<key12>/serve.sock`, where `<key12>` is the first 12 hex characters of the SHA-256 of the canonical store path (`StoreKey`).
- The directory is 0700 and the socket is 0600. The runtime root is checked by `ensure_store_root()`.
- `serve` holds the `CHAT` flock before it binds, so a socket file left at the path is stale and is removed.
- A peer whose uid is not the uid of `serve` is dropped before any frame is read.
- The socket directory is removed when `serve` stops.

## Protocol

One JSON frame per line: `{"v":1,"id":<n>,"body":{...}}`, at most `MAX_FRAME_BYTES`. A frame with another `v` is an error. Before `hello` a connection has a 30 s read timeout. After `hello` it has none. Every connection has a 2 s write timeout (`WRITE_TIMEOUT`): a client that stops reading gets an error on the write and is dropped, so a push cannot block the serve loop.

| Client body | Reply |
|---|---|
| `{"type":"hello","proto":1,"inbox_push":true}` (must be first, `inbox_push` defaults to false) | `{"type":"hello","proto":1}` |
| `{"type":"send","thread":null\|"name","text":"..."}` | `turn` frames with `{"type":"turn","event":{...}}`, then `{"type":"turn_done"}` |
| `{"type":"thread","op":"pull\|park\|revive\|drop","name":"n"}` | `{"type":"done","message":"..."}` |
| `{"type":"thread","op":"merge","name":"n","view":false}` | `done` |
| `{"type":"thread","op":"open","name":"n","model":null\|"m","auto":null\|"merge"\|"drop"}` | `done` |
| `{"type":"thread","op":"switch","name":"n"}` and `{"type":"thread","op":"main"}` | `done` with `attached to <key>` |
| `{"type":"cancel","thread":null\|"name"}` | `{"type":"done","message":"cancelled <key>"}` |
| `{"type":"answer","thread":null\|"name","worker":"id","request":"per_1","option":"once"}` | `{"type":"done","message":"Answered the permission request of worker <id>."}` or `error` with the host refusal |
| `{"type":"status","thread":null\|"name"}` | `{"type":"status","report":{"state":"Idle","msg":null},"unread":0,"blocked":["id"],"turns":0}` |
| `{"type":"shutdown"}` after hello | none; `serve` stops |

`send`, `cancel` and `status` with `"thread":null` go to the connection's attached session. It is main until `switch` or `main`. The attached key belongs to the connection only.

Every reply carries the request `id`. A failure replies `{"type":"error","message":"..."}`.

### Errors

| Error | Connection |
|---|---|
| frame before `hello`, bad frame, wrong `v`, `hello` with another `proto` | closed |
| session busy, session not open, session not running a turn (`cancel`), draining, invalid thread name, a failed thread op, a refused answer | stays open |

## Turns

- A session runs one turn at a time. A `send` to a busy session gets `error` `<key> is busy`. A `send` to a session that is not open gets `<key> is not open`.
- `cancel` stops the turn of a session key. Send it on another connection: the sender is blocked while its own turn runs. A `cancel` to an idle session gets `<key> is not running a turn`.
- Each turn gets a fresh cancel token. A `cancel` ends one turn only.
- `status` is computed on request. `report` is the prompt-phase program status. `blocked` is the sorted ids of workers waiting on a permission.
- A turn streams on the connection that sent it. Other connections do not see its events.
- Each open session runs the headless worker hosts for its own store (main, and each open thread). About once per second `serve` reads the `blocked/` markers of the open stores and pushes a `blocked` frame to every client that sent `hello`, with envelope id `0`: `{thread, worker, content}`. `content` is the front permission request (`request_id`, `tool`, `summary`, `options`) when it is new or changed, and `null` when the request settled. Empty or unparsable markers are skipped. Replies, turn events and pushes share one write lock per connection, so lines never interleave. A client with no read loop sees pushes at its next request. `ralph chat --connect` prints each push as one line.
- `SendIt` events are dropped. `/send-it` is not served.

## Threads

- `open` runs `prepare_open` (checks the thread, writes `model` to its meta) and then opens a session in the `serve` process. It does not start a Rex block. A second `open` of an open thread gets `thread <name> is already open`.
- `open` with `auto` set is refused with `auto merge/drop is not supported by serve yet`. The connection stays open and nothing is created. Running auto at close is a later ticket.
- `pull`, `park`, `drop` and `merge` need the thread claim. If the thread is open, `serve` closes its session first, then runs the operation. A failed or refused operation leaves the thread closed. Reopen it with `open`. `revive` does not close.
- These operations are refused with `<key> is busy` while the thread runs a turn.
- `merge` without `view` is confirmed by the client: `serve` answers yes. The merge notice is part of the `done` message. The merge is recorded in main as the CLI does.
- `ls` is not served.

## Stop

SIGINT, SIGTERM, SIGHUP or a client `shutdown` feed `Stop` to the supervisor. It moves to Draining. An idle session starts closing at once. Closing stops its maintenance host and waits for any tick, keepalive ping, or scheduled worker start already in progress. A session in a turn gets its turn cancelled (`Cancel`), and it closes only after the turn ends and its `turn_done` or `error` frame is written (`TurnDone` then `Close`). `serve` exits when every session is closed (main releases the `CHAT` lease and the runtime dir goes). If a session does not close within 5 s (`DRAIN_BOUND` in `crates/ralph/src/serve/mod.rs`), `serve` warns and exits anyway, so a stuck turn or maintenance task cannot hang shutdown. While it drains, `hello`, `send` and thread operations get `error` `ralph serve is shutting down`. A second stop does nothing.

## Maintenance

- Main and each open thread have their own attachments directory and maintenance host. The host runs queued watches, armed schedules, inbox checks, and configured idle keepalive. Main runs the daily drift check. A successful reply starts the usual memory nap.
- Each fresh report stays unread. Serve sends one `inbox` frame with envelope id `0` to every client that opted in during its first hello, regardless of the client's attached session. The body carries `thread` (null for main), `report` (worker, status, timestamp `at`, and text), and the session's total `unread` count. Reports that fit the frame travel in full. If a frame would exceed 1 MiB, serve caps each report field at 32 KiB for transport. The inbox retains the original report.
- `ralph chat --connect` opts in. At its next socket read it displays the session, worker, status, unread count, and report text on one line of up to 240 characters. A push can arrive between replies or during a turn.

## Limits

- `NOTICE_SINK` is process-wide. All sessions share it.
- Worker ownership is per open session. Each open gets the instance `pid@start#n` for its store, and the owner records of its workers carry it. When a thread or main closes and opens again in the same process, the new session reclaims the owner records the closed session left: it stops their agent groups and drops their queued prompts. Records of other processes follow the pid rules as before.
- There is no live end-to-end test. Unit tests cover the supervisor, the frames and the routing.

## Client (`ralph chat --connect <SOCK>`, hidden)

Served:

- Plain text: a `send` to the attached session.
- `/answer <worker> <request> <option>`: one `answer` frame for the attached key. All three words are required, else the usage line prints. The frame takes the same answer-file path as the `reply` tool, so a stale or unknown request is refused by the host. The connection thread waits up to the 30 s handoff timeout. An `error` leaves the chat open. The terminal chat prints `/answer is only served over --connect`.
- `/thread switch|main|open [--model]|pull|park|revive|drop|merge [--view]`: one `thread` frame. The reply text is printed. The attached thread changes only after a `done` reply.
- Ctrl-C: `cancel` for the attached key, sent on a second connection.
- Ctrl-D: detach. The client closes its connection. `shutdown` is never sent.
- SIGTERM / SIGHUP: detach and exit 0 at the prompt or mid-request; neither `cancel` nor `shutdown` is sent.
- Attach: the client sends `status` and prints `<key>: <state>, <n> unread[, blocked: <ids>]`. The terminal status line (OSC) is not sent.

Not served (prints `<line> is not served over --connect`): `/thread ls`, send-it, inbox, hold, shell, cost.

Errors:

- An `error` frame (`main is busy`, `ralph serve is shutting down`) is printed. The prompt returns.
- A closed connection or I/O error ends the client.

Limits:

- No re-bind of a running turn. A re-attach during a turn gets `<key> is busy`. That turn's events are lost.
- A re-attach always goes to main.
