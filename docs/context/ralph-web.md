# ralph web Command

`ralph web [--store <DIR>] [--port <N>]` serves a local HTTP API on 127.0.0.1. It runs in the foreground until Ctrl-C. It serves the embedded shell, its stylesheet and font, `GET /api/health` and `GET /tokens.css`.

## Code Locations

| Component | File | Description |
|-----------|------|-------------|
| CLI args | `crates/ralph/src/cli/mod.rs` | `Web(WebArgs)`: `--store`, `--port` |
| Shell | `crates/ralph/src/web.rs` | `run()`: store, token, bind, print URL, serve |
| Tokens CSS | `crates/ralph_api/src/tokens.rs` | pure `tokens_css()`: the CSS text |
| Tokens input | `crates/ralph/src/web_tokens.rs` | `render_tokens()`: theme slots and `[web]` to CSS |
| Token | `crates/ralph_api/src/token.rs` | `Token::generate()`, `Token::matches()`, `extract_tokens()` |
| Routes | `crates/ralph_api/src/route.rs` | pure `route()`: Host check, token check, paths |
| Bind rule | `crates/ralph_api/src/addr.rs` | `BindPolicy`, `check_bind()`, `allowed_hosts()` |
| Admit | `crates/ralph_api/src/admit.rs` | pure `admit()` and `MAX_HANDLERS`: the cap rule |
| Server | `crates/ralph_api/src/server.rs` | `bind()`, `serve()` on tiny_http |
| Assets | `crates/ralph_api/src/assets.rs` | embedded table: `find()` |
| Asset files | `crates/ralph_api/assets/` | `index.html`, `app.css`, `fonts/` |
| Colour lint | `xtask/src/lint/web_colour.rs` | checks the files in `assets/` |

## Start

`ralph web` prints one line: `ralph web listening on http://127.0.0.1:<port>/?token=<token>`. It also prints the store path. Open the URL. The server answers 302, sets the cookie and sends the browser to `/`.

## Token

- `ralph web` makes a new token at each start. The token has 32 lowercase hex characters from `/dev/urandom`.
- A request can carry the token in one of three places. The first one found is used:
  1. `Authorization: Bearer <token>`
  2. the `token=` query parameter
  3. the `ralph_token_<port>` cookie. A request can have many `Cookie` headers and many cookies with this name. One matching value is enough. Quotes around a value are removed.
- The cookie name holds the bound port. A browser does not scope cookies by port. With one name, a second `ralph web` run on 127.0.0.1 would overwrite the cookie of the first run. A cookie of another port is not accepted.
- The cookie is `HttpOnly; SameSite=Strict; Path=/`.
- The compare takes the same time for every wrong value of the same length.
- The `token=` value is not URL-decoded. The token is hex, so it needs no decoding. A percent-encoded value does not match.
- Every request needs the token, also a request for an unknown path.

## Loopback rule

- `ralph web` binds only to an address in the `BindPolicy`. `BindPolicy::loopback()` allows `127.0.0.1` and `::1`. It is the only place that holds these values. The shell reads the bind address from `BindPolicy::primary()` and the host names from `bind_hosts()`. An IPv6 host is written as `[::1]:<port>`.
- `bind()` checks the requested address. It binds. Then it checks the address that the OS bound.
- Any other address is refused with an error that names the address.
- A busy port gives `could not bind 127.0.0.1:<N>: <os error>`.
- On macOS, the bind to `127.0.0.1:<N>` can succeed while another process holds `0.0.0.0:<N>` with `SO_REUSEADDR`. `ralph web` does not detect this. Use `--port 0` to avoid it.

## Host check

- `route()` accepts a request only when the `Host` header is in `allowed_hosts`. The shell builds the list after the bind: `127.0.0.1:<port>` and `localhost:<port>`.
- A missing or other Host gives 403. This stops DNS rebinding.

## Port

- `--port` sets the port on 127.0.0.1. The default is `0`, which picks a free port.
- The printed URL shows the real port.

## Routes

| Request | Answer |
|---------|--------|
| more than 8 requests in progress | 503 `{"error":"busy"}`, checked first, before Host and token |
| wrong Host | 403 `{"error":"forbidden host"}` |
| no token or wrong token | 401 `{"error":"unauthorized"}` |
| `GET /api/health` | 200 `{"status":"ok","version":"<version>"}` |
| other method on `/api/health` | 405 with `Allow: GET` |
| `GET /` with a valid `?token=` | 302 to `/` with `Set-Cookie` |
| `GET /` with the cookie or Bearer token | 200 `text/html` shell, with a `Content-Security-Policy` |
| `GET /assets/app.css` | 200 `text/css` |
| `GET /assets/geist-mono.woff2` | 200 `font/woff2` |
| other method on `/` or `/assets/*` | 405 with `Allow: GET` |
| `GET /tokens.css` | 200 `text/css; charset=utf-8` with `Cache-Control: no-store` |
| other method on `/tokens.css` | 405 with `Allow: GET` |
| other path | 404 `{"error":"not found"}` |

The 503 is the only answer that does not pass the Host and token checks. The server sends it before `route()` runs, so a request without a token also gets it. The other rows apply in the order shown, after the 503 row.

Every answer has `Cache-Control: no-store`. Every answer is JSON, except `/tokens.css` and the embedded assets. `/tokens.css` needs the token like every route, after the Host check. The health answer does not show the store path.

## Embedded assets

- The files in `crates/ralph_api/assets/` are in the binary through `include_bytes!`. There is no build step.
- The CSP is `default-src 'self'; img-src 'self' data:; frame-ancestors 'none'`. Assets also send `Referrer-Policy: no-referrer` and `X-Content-Type-Options: nosniff`.
- The shell has no script. Its nav links use `#/name`.
- The font is the full variable font Geist Mono (`GeistMono[wght].woff2`) from vercel/geist-font tag v1.9.2, unchanged. It is not a latin-only subset. The repo name has no brackets: `fonts/geist-mono.woff2`.
- `fonts/NOTICE` records the tag, the source path and the SHA-256. `fonts/OFL.txt` is the upstream licence. `ralph licenses` prints both.
- The `[web]` font default starts with `"Geist Mono"`.
- `cargo xtask lint` runs a colour lint on `assets/`. It refuses raw colours, `var()` outside `--r-*` and custom elements that are not one of the 14 primitives.
- A body of 32 KB or more goes out with chunked transfer encoding. The test client decodes it.

## Limits

- The server handles each request on its own thread. It runs at most 8 at the same time. The next one gets 503 at once.
- A client can send a long `Content-Length` and never send the body. That request keeps its slot until the client closes the socket. Read timeouts come with GUZ-1755.
- tiny_http 0.12.0 runs one thread for each open connection: the accept loop in `Server::from_listener` hands each connection to `TaskPool::spawn`, which starts a new thread when none is free (`lib.rs:323`, `util/task_pool.rs:69-76`). An admitted stalled request costs two threads: the tiny_http connection thread and the handler thread that holds the slot. Past 8 requests in progress, a stalled request costs the connection thread and a thread that sends the 503. All of these threads end when the client closes the socket. Only GUZ-1755 timeouts bound them.
- A stalled request never blocks the stop check.

## tokens.css

`ralph web` loads the store `config.toml` over the global file. It builds the CSS once at start-up from the `[theme]` slots and `[web]`. See [configuration](../configuration.md#web---ralph-web-fonts-and-ansi-colours). The file does not change while `ralph web` runs. Any invalid config stops `ralph web` before it binds, with the config error. This covers every section and both files, not only `[web]`.

- Slots come from the assistant Palette and the status, diff, input, tools and ui groups. They come in this order.
- Each slot is a variable `--r-<slot>`. Palette slots have no prefix, for example `--r-link`. Other groups have a prefix: `--r-status-mode-idle-fg`, `--r-diff-add-fg`, `--r-input-suggestion`, `--r-tools-name`, `--r-ui-spinner`.
- `--r-font` holds the `font` value. The generator drops `;`, braces, angle brackets, `\`, `/`, `*`, parentheses, square brackets, control characters and unbalanced quotes, so a font cannot break the stylesheet.
- `--r-ansi-0` to `--r-ansi-15` hold the ANSI table.
- A named colour or an index 0-15 becomes `var(--r-ansi-N)`.
- An index 16-255 becomes `#rrggbb` through the xterm cube and greys.
- A `#rrggbb` value passes through.
- A slot set to the terminal default (Reset) is omitted. A slot that is unset is omitted too.
- The dark table is in the default `:root` block. The light block overrides only the 16 ANSI variables inside `@media (prefers-color-scheme: light)`.

## Store

- `--store` sets the store. Without it, `RALPH_CHAT_STORE` is used, then `~/.ralph/chat`.
- The store must exist. `ralph web` does not create it. A missing store gives `store not found: <path>`.

## Shutdown

- SIGINT, SIGTERM and SIGHUP stop the server. The server checks for a stop every 100 ms.
- A stop exits with code 0.

## Contract tests

- `crates/ralph_api/tests/health_contract.rs` starts the real server in a thread on 127.0.0.1 port 0. Each test has its own server and its own temp store, so tests run in parallel.
- The client is a raw `TcpStream` in `crates/ralph_api/tests/common/mod.rs`. It can set any `Host` header, send two `Cookie` headers, or send no `Host`.
- The test config uses the fixed version `0.0.0-test`. The version never changes a golden.
- Each case compares the answer to `crates/ralph_api/tests/golden/<case>.txt`. A golden holds the status, the stable headers (`Content-Type`, `Cache-Control`, `Allow`) and the body.
- `crates/ralph_api/tests/shell_contract.rs` does the same for the shell, `/assets/*` and the cookie hand-off. The font test checks bytes, with no golden. Cookie names in goldens show as `ralph_token_<port>`.
- `crates/ralph_api/tests/tokens_contract.rs` does the same for `/tokens.css`. Its goldens are `crates/ralph_api/tests/golden/tokens_*.txt`.
- Every new route ships with golden cases. Change a golden only as a compatibility decision, and say so in the commit.
- A missing or different golden fails the test with a line diff.
- To write or refresh goldens, run `UPDATE_GOLDEN=1 cargo test -p ralph_api --test health_contract`. Read every changed file before you commit.
- `cargo xtask lint` runs these tests, because it runs `cargo test --all-targets`.
