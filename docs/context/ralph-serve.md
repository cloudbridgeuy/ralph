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

One JSON frame per line: `{"v":1,"id":<n>,"body":{...}}`, at most `MAX_FRAME_BYTES`. A frame with another `v` is an error. Before `hello` a connection has a 30 s read timeout. After `hello` it has none.

| Client body | Reply |
|---|---|
| `{"type":"hello","proto":1}` (must be first) | `{"type":"hello","proto":1}` |
| `{"type":"send","thread":null\|"name","text":"..."}` | `turn` frames with `{"type":"turn","event":{...}}`, then `{"type":"turn_done"}` |
| `{"type":"thread","op":"pull\|park\|revive\|drop","name":"n"}` | `{"type":"done","message":"..."}` |
| `{"type":"thread","op":"merge","name":"n","view":false}` | `done` |
| `{"type":"thread","op":"open","name":"n","model":null\|"m","auto":null\|"merge"\|"drop"}` | `done` |
| `{"type":"thread","op":"switch","name":"n"}` and `{"type":"thread","op":"main"}` | `done` with `attached to <key>` |
| `{"type":"shutdown"}` after hello | none; `serve` stops |

`send` with `"thread":null` goes to the connection's attached session. It is main until `switch` or `main`. The attached key belongs to the connection only.

Every reply carries the request `id`. A failure replies `{"type":"error","message":"..."}`.

### Errors

| Error | Connection |
|---|---|
| frame before `hello`, bad frame, wrong `v`, `hello` with another `proto` | closed |
| session busy, session not open, draining, invalid thread name, a failed thread op | stays open |

## Turns

- A session runs one turn at a time. A `send` to a busy session gets `error` `<key> is busy`. A `send` to a session that is not open gets `<key> is not open`.
- A turn streams on the connection that sent it. Other connections do not see its events.
- `SendIt` events are dropped. `/send-it` is not served.

## Threads

- `open` runs `prepare_open` (checks the thread, writes `model` to its meta) and then opens a session in the `serve` process. It does not start a Rex block. A second `open` of an open thread gets `thread <name> is already open`.
- `open` with `auto` set is refused with `auto merge/drop is not supported by serve yet`. The connection stays open and nothing is created. Running auto at close is a later ticket.
- `pull`, `park`, `drop` and `merge` need the thread claim. If the thread is open, `serve` closes its session first, then runs the operation. A failed or refused operation leaves the thread closed. Reopen it with `open`. `revive` does not close.
- These operations are refused with `<key> is busy` while the thread runs a turn.
- `merge` without `view` is confirmed by the client: `serve` answers yes. The merge notice is part of the `done` message. The merge is recorded in main as the CLI does.
- `ls` is not served.

## Stop

SIGINT, SIGTERM, SIGHUP or a client `shutdown` feed `Stop` to the supervisor. It moves to Draining. A turn in flight is cancelled, then every open session is closed (main last releases the `CHAT` lease), then it exits. While it drains, `hello`, `send` and thread operations get `error` `ralph serve is shutting down`. A second stop does nothing.

## Limits

- Serve sessions have no nap or host loop, no keepalive, no headless watchers and no attachments dir.
- `NOTICE_SINK` and worker ownership (`own_instance`) are process-wide. All sessions share them. Per-session ownership is a follow-up ticket.
- There is no live end-to-end test. Unit tests cover the supervisor, the frames and the routing.
