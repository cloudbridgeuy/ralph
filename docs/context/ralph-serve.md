# ralph serve Command (experimental, hidden)

`ralph serve --store <DIR>` keeps one chat store open in a long-lived process and listens on a private Unix socket. It does not show in `ralph --help`. It does not run turns yet: a client can only greet it and ask it to stop. Turns and threads over the socket come in GUZ-1733.

## Code Locations

| Component | File | Description |
|-----------|------|-------------|
| CLI args | `crates/ralph/src/cli/mod.rs` | hidden `Serve(ServeArgs)`, required `--store <DIR>` |
| Shell | `crates/ralph/src/serve.rs` | `run()`, accept loop, connection threads, `classify()`, `answer()` |
| Signals | `crates/ralph/src/signal.rs` | `init_serve()`: SIGINT, SIGTERM and SIGHUP set the stop flag |
| Store gate, refusal | `crates/core/src/serve.rs` | `serve_store_gate()`, `serve_refusal()` |
| Supervisor | `crates/core/src/serve_supervisor.rs` | pure state machine: `Supervisor::start()`, `step()`, `Input`, `Effect` |
| Socket path | `crates/core/src/chat_socket.rs` | `serve_socket_path()` |
| Socket | `crates/ralph_server/src/chat/serve_socket.rs` | `ServeSocket::bind()`, `accept()` with the peer uid check |
| Frames | `crates/ralph_server/src/chat/serve_proto.rs` | `Envelope`, `ClientMsg`, `ServerMsg`, `hello_reply()` |

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

One JSON frame per line: `{"v":1,"id":<n>,"body":{...}}`, at most `MAX_FRAME_BYTES`. A frame with another `v` is an error.

| Client frame | Reply |
|---|---|
| `{"type":"hello","proto":1}` (must be first) | `{"type":"hello","proto":1}` |
| `hello` with another `proto` | `error`, then close |
| `{"type":"shutdown"}` after hello | none; `serve` stops |
| any other frame, or a frame before hello | `error`, then close |

## Stop

SIGINT, SIGTERM, SIGHUP or a client `shutdown` feed `Stop` to the supervisor. It closes the main session (releases the `CHAT` lease), then exits. A second stop does nothing. A `hello` that comes while it stops gets `error`.
