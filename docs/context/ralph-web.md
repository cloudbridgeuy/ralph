# ralph web Command

`ralph web [--store <DIR>] [--port <N>]` serves a local HTTP API on 127.0.0.1. It runs in the foreground until Ctrl-C. Today it has one route, `GET /api/health`.

## Code Locations

| Component | File | Description |
|-----------|------|-------------|
| CLI args | `crates/ralph/src/cli/mod.rs` | `Web(WebArgs)`: `--store`, `--port` |
| Shell | `crates/ralph/src/web.rs` | `run()`: store, token, bind, print URL, serve |
| Token | `crates/ralph_api/src/token.rs` | `Token::generate()`, `Token::matches()`, `extract_token()` |
| Routes | `crates/ralph_api/src/route.rs` | pure `route()`: Host check, token check, paths |
| Bind rule | `crates/ralph_api/src/addr.rs` | `BindPolicy`, `check_bind()`, `allowed_hosts()` |
| Server | `crates/ralph_api/src/server.rs` | `bind()`, `serve()` on tiny_http |

## Start

`ralph web` prints one line: `ralph web listening on http://127.0.0.1:<port>/?token=<token>`. It also prints the store path.

## Token

- `ralph web` makes a new token at each start. The token has 32 lowercase hex characters from `/dev/urandom`.
- A request can carry the token in one of three places. The first one found is used:
  1. `Authorization: Bearer <token>`
  2. the `token=` query parameter
  3. the `ralph_token` cookie
- The compare takes the same time for every wrong value of the same length.
- Every request needs the token, also a request for an unknown path.

## Loopback rule

- `ralph web` binds only to an address in the `BindPolicy`. `BindPolicy::loopback()` allows `127.0.0.1` and `::1`. It is the only place that holds these values.
- `bind()` checks the requested address. It binds. Then it checks the address that the OS bound.
- Any other address is refused with an error that names the address.
- A busy port gives `could not bind 127.0.0.1:<N>: <os error>`.

## Host check

- `route()` accepts a request only when the `Host` header is in `allowed_hosts`. The shell builds the list after the bind: `127.0.0.1:<port>` and `localhost:<port>`.
- A missing or other Host gives 403. This stops DNS rebinding.

## Port

- `--port` sets the port on 127.0.0.1. The default is `0`, which picks a free port.
- The printed URL shows the real port.

## Routes

| Request | Answer |
|---------|--------|
| wrong Host | 403 `{"error":"forbidden host"}` |
| no token or wrong token | 401 `{"error":"unauthorized"}` |
| `GET /api/health` | 200 `{"status":"ok","version":"<version>"}` |
| other method on `/api/health` | 405 with `Allow: GET` |
| other path | 404 `{"error":"not found"}` |

Every answer is JSON with `Cache-Control: no-store`. The health answer does not show the store path.

## Store

- `--store` sets the store. Without it, `RALPH_CHAT_STORE` is used, then `~/.ralph/chat`.
- The store must exist. `ralph web` does not create it. A missing store gives `store not found: <path>`.

## Shutdown

- SIGINT, SIGTERM and SIGHUP stop the server. The server checks for a stop every 100 ms.
- A stop exits with code 0.
