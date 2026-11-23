# ralph web Command

`ralph web [--store <DIR>] [--port <N>]` serves a local HTTP API on 127.0.0.1. It runs in the foreground until Ctrl-C. It serves the embedded shell, its stylesheet and font, `GET /api/health`, `POST /api/render` and `GET /tokens.css`.

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
| Primitives | `crates/ralph_api/src/render/` | pure HTML builders, one file for each primitive, plus `escape.rs` |
| Markdown | `crates/ralph_api/src/render/{prose,md_event,md_link,code}.rs` | `render_markdown()`: pulldown-cmark events to HTML inside `<r-prose>`; `code.rs` highlights fenced blocks |
| Catalogue page | `crates/ralph_api/src/catalogue/` | `page()` builds `/catalogue`; `fixture.rs` parses the fixture |
| Fixture | `crates/ralph_api/fixtures/catalogue.json` | sanitised sample data for the catalogue |
| Fixture generator | `xtask/src/catalogue/` | `core.rs` is pure; `mod.rs` reads and writes files |

## Start

`ralph web` prints one line: `ralph web listening on http://127.0.0.1:<port>/?token=<token>`. It also prints the store path. Open the URL. The server answers 302, sets the cookie and sends the browser to `/`.

## Token

- `ralph web` makes a new token at each start. The token has 32 lowercase hex characters from `/dev/urandom`.
- A request can carry the token in one of three places. The first one found is used:
  1. `Authorization: Bearer <token>`
  2. the `token=` query parameter
  3. the `ralph_token_<port>` cookie. A request can have many `Cookie` headers and many cookies with this name. One matching value is enough. Quotes around a value are removed.
- The cookie name holds the bound port. A browser does not scope cookies by port. With one name, a second `ralph web` run on 127.0.0.1 would overwrite the cookie of the first run. A cookie of another port is not accepted.
- The cookie is `HttpOnly; SameSite=Strict; Path=/`. It has no `Max-Age`, so it lasts until the browser session ends.
- A browser sends every `ralph_token_<port>` cookie to every port on 127.0.0.1. Cookies are not scoped by port. Any other local HTTP service on 127.0.0.1 can read the token of a running `ralph web`. The token ends when `ralph web` stops.
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
| `POST /api/render` with `application/json` and `{"markdown": "<text>"}` | 200 `{"html":"<r-prose>…</r-prose>"}` from `render_markdown()`, with `no-store`, `nosniff` and `no-referrer` |
| `POST /api/render` with another content type | 415 `{"error":"unsupported media type"}`; `; charset=utf-8` and any case are accepted |
| `POST /api/render` with invalid JSON, no `markdown`, a non-string `markdown` or a non-UTF-8 body | 400 `{"error":"bad request"}` |
| other method on `/api/render` | 405 with `Allow: POST` |
| `GET /` with a valid `?token=` | 302 to `/` with `Set-Cookie` |
| `GET /` with the cookie or Bearer token | 200 `text/html` shell, with a `Content-Security-Policy` |
| `GET /assets/app.css` | 200 `text/css` |
| `GET /assets/geist-mono.woff2` | 200 `font/woff2` |
| other method on `/` or `/assets/*` | 405 with `Allow: GET` |
| `GET /catalogue` | 200 `text/html` page with the same headers and CSP as the shell |
| other method on `/catalogue` | 405 with `Allow: GET` |
| `GET /tokens.css` | 200 `text/css; charset=utf-8` with `Cache-Control: no-store` |
| other method on `/tokens.css` | 405 with `Allow: GET` |
| other path | 404 `{"error":"not found"}` |

The 503 is the only answer that does not pass the Host and token checks. The server sends it before `route()` runs, so a request without a token also gets it. The other rows apply in the order shown, after the 503 row.

Every answer has `Cache-Control: no-store`. Every answer is JSON, except `/tokens.css`, `/catalogue`, the embedded assets and the 302 hand-off (it has no body). `/tokens.css` needs the token like every route, after the Host check. The health answer does not show the store path.

## Embedded assets

- The files in `crates/ralph_api/assets/` are in the binary through `include_bytes!`. There is no build step.
- The CSP is `default-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`. Assets also send `Referrer-Policy: no-referrer` and `X-Content-Type-Options: nosniff`.
- The shell has no script. Its nav links use `#/name`.
- The font is the full variable font Geist Mono (`GeistMono[wght].woff2`) from vercel/geist-font tag v1.9.2, unchanged. It is not a latin-only subset. The repo name has no brackets: `fonts/geist-mono.woff2`.
- `fonts/NOTICE` records the tag, the source path and the SHA-256. `fonts/OFL.txt` is the upstream licence. `ralph licenses` prints both.
- The `[web]` font default starts with `"Geist Mono"`.
- `cargo xtask lint` runs a colour lint on `assets/`. It refuses raw colours, named colours (also in custom properties), `var()` outside `--r-*`, any `--r-*` definition (tokens come only from `/tokens.css`) and custom elements that are not one of the 14 allowed primitives. Twelve are built: r-box, r-rule, r-table, r-kv, r-pill, r-status, r-empty, r-error, r-spinner, r-spark, r-code and r-prose. The other two (r-tree and r-graph) pass the lint but have no builder yet. It skips binary assets (woff, woff2, png, jpg, jpeg, gif, ico, webp). It refuses `.svgz` files, because it cannot scan gzipped SVG; use plain `.svg`. A file that is not UTF-8, or that it cannot read, fails the lint.
- A body of 32 KB or more goes out with chunked transfer encoding. The test client decodes it.

## Primitives

The UI has no JavaScript. Each primitive is a hyphenated tag that the server renders as plain HTML. A browser treats an unknown hyphenated tag as an inline element, and `app.css` styles it. The colour lint allows only these tags.

| Tag | Builder | Notes |
|-----|---------|-------|
| `r-box` | `frame(shape, title, inner_html)` | shape `square`, `round` or `heavy`; optional title |
| `r-rule` | `rule()` | `role="separator"` |
| `r-table` | `table(headers, rows)` | `th scope="col"`; a short row is padded to the header count |
| `r-kv` | `kv(pairs)` | a `dl` of `dt` and `dd` |
| `r-pill` | `pill(tone, shape, icon, text)` | shape `square` or `round`; optional inline SVG icon with `aria-hidden` |
| `r-status` | `status(tone, text)` | `role="status"`, `aria-live="polite"`; a word (OK, Error, Warning, Info, Note) shows the tone, so colour is never the only signal |
| `r-empty` | `empty(text)` | the empty state |
| `r-error` | `error(text)` | `role="alert"` |
| `r-spinner` | `spinner()` | `role="status"` with visually hidden "Loading" text |
| `r-spark` | `spark_svg(values, w, h)` | returns `None` for an empty series; the caller renders `r-empty` |

- Every builder is a pure function that returns a `String`. One `escape()` handles text and attribute values (`& < > " '`).
- Attribute values come from closed enums (`Tone`: ok, bad, warn, info, accent; `Shape`; `PillShape`; `Icon`), never from free text. `inner_html` of `frame()` must be built from these builders.
- No builder writes a `style` attribute, because the CSP forbids inline style. No builder writes a raw colour. SVG uses `stroke="currentColor"` and classes.
- `spark_points()` drops NaN and infinite values. A flat series and a single point draw a mid line. Huge values do not overflow.
- `render_markdown(&str)` parses with tables, strikethrough and task lists. Its own emitter writes the HTML. Raw HTML is escaped as text, never passed through. Links keep `href` only for `http`, `https`, `mailto` and `#fragment`; the check strips control characters and spaces and ignores case. Other links keep their text without `href`. Links get `rel="noreferrer noopener"`. An image is never `<img>`: its alt text goes in a `<span>`. Tags nested deeper than 64 levels render as plain text. NUL bytes become U+FFFD. A fenced or indented code block becomes `<r-code data-lang="rust"><pre><code>…</code></pre></r-code>`. `highlight()` in `code.rs` uses the `two_face::syntax::extra_newlines()` set (built once in `prose.rs`) and the syntect `ClassedHTMLGenerator` with class prefix `t-`. There is no generated theme CSS; `app.css` maps the `t-*` scope classes to `--r-*` slots. `data-lang` is the first word of the info string, lower-cased, and must match `[a-z0-9+#-]{1,32}`; otherwise the attribute is dropped. An unknown language, no language, or a block over 64 KiB gives plain escaped text inside `r-code`. The `[theme]` syntax theme does not apply on the web.
- Goldens are in `crates/ralph_api/tests/golden/render/<primitive>_<state>.html`. Unit tests in `render/` compare them. The states are empty, sparse, dense, error and very-long where they apply, plus hostile text for escaping.
- `app.css` holds the primitive styles, including `r-prose`, `r-code` and the `t-*` scope rules. They use `var(--r-*)` colours only. Code stays on one line and scrolls sideways inside `pre`, which shows a `:focus-visible` outline. The spinner fades in after 200 ms from opacity 0, so a short load shows no spinner. `prefers-reduced-motion` turns its animation off.

## Catalogue

`GET /catalogue` shows every primitive in the states that apply to it. r-box, r-table, r-kv, r-pill, r-status, r-spark, r-prose and r-code show all five: empty, sparse, dense, error and very-long. r-empty shows empty, sparse and very-long. r-error shows empty, error and very-long. r-rule and r-spinner show only sparse. The r-prose and r-code states come from `render_markdown()` on short markdown constants in the catalogue module, not from the fixture, because the fixture holds no markdown; the error state is an `r-error`. The page has two columns, `<section data-theme="dark">` and `<section data-theme="light">`. The shell nav links to `/catalogue`. It is not `#/catalogue`: the shell has no script, so a hash link cannot reach a server page. The page has a link back to `/`.

- The route passes the same Host and token checks as the shell. It sends `no-store` and the shell CSP, unchanged.
- `include_str!` embeds the fixture. `catalogue::page()` renders it. A fixture that does not parse gives an `r-error` in both columns.
- `tokens.css` ends with `[data-theme="dark"]` and `[data-theme="light"]` blocks. They hold the two ANSI tables, so a column shows its own theme whatever the browser theme is.
- `crates/ralph_api/tests/catalogue_contract.rs` has the contract cases and goldens `catalogue_*.txt`. Unit tests in `catalogue/` scan the page for `style=` and for raw colours.

## Catalogue fixture

`cargo xtask catalogue --store <DIR> --out crates/ralph_api/fixtures/catalogue.json` derives the fixture. Run it on a copy of a store, not on the live store. The command only reads the store.

- It reads `usage.jsonl` for the cost series, durations, and turn and nap counts. It reads `log.jsonl` for role counts only. It reads the files in `status/` for titles.
- It keeps counts, durations, whole-dollar costs for each day (the last 60 days) and short titles. It drops paths, prompts, tool arguments and message bodies.
- A title with `/`, `\`, `@`, `sk-` or `token`, or with more than 200 characters, is dropped.
- `very_long_title_derived` is the longest real title, repeated to at least 300 characters.
- The command refuses to write a file that holds `sk-`, `/Users/`, `@` or `token`, or that is over 20 KB. A unit test checks the committed fixture for the same.

## Limits

- The server handles each request on its own thread. It runs at most 8 at the same time. The next one gets 503 at once.
- A client can send a long `Content-Length` and never send the body. That request keeps its slot until the client closes the socket. Read timeouts come with GUZ-1755.
- tiny_http 0.12.0 runs one thread for each open connection: the accept loop in `Server::from_listener` hands each connection to `TaskPool::spawn`, which starts a new thread when none is free (`lib.rs:323`, `util/task_pool.rs:69-76`). An admitted stalled request costs two threads: the tiny_http connection thread and the handler thread that holds the slot. Past 8 requests in progress, a stalled request costs the connection thread and a thread that sends the 503. All of these threads end when the client closes the socket. Only GUZ-1755 timeouts bound them.
- The server reads a request body up to 256 KiB, and only for an authorised `POST /api/render`. It checks Host and token on the headers first. A request that fails a check, or that targets another path, gets its answer without a body read, so `Expect: 100-continue` gets no `100 Continue`. A `Content-Length` over the cap gets 413 `{"error":"too large"}` at once, without a body read. The bytes read are checked again against the cap. The 413 comes after the 503, 403 and 401 checks. A request with a body under the cap that stalls keeps its slot until the client closes the socket.
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
- Each case compares the answer to `crates/ralph_api/tests/golden/<case>.txt`. A golden holds the status, the stable headers (`Content-Type`, `Cache-Control`, `Allow`, `Location`, `Set-Cookie`, `Content-Security-Policy`, `Referrer-Policy`, `X-Content-Type-Options`) and the body.
- `crates/ralph_api/tests/shell_contract.rs` does the same for the shell, `/assets/*` and the cookie hand-off. The font test checks bytes, with no golden. Cookie names in goldens show as `ralph_token_<port>`.
- `crates/ralph_api/tests/tokens_contract.rs` does the same for `/tokens.css`. Its goldens are `crates/ralph_api/tests/golden/tokens_*.txt`.
- `crates/ralph_api/tests/render_contract.rs` does the same for `POST /api/render`. Its goldens are `crates/ralph_api/tests/golden/render_*.txt`: ok, a hostile `<script>` input, 400, 401, 403, 405, 413 and 415. The pure parse and response logic is in `crates/ralph_api/src/render_api.rs`.
- Every new route ships with golden cases. Change a golden only as a compatibility decision, and say so in the commit.
- A missing or different golden fails the test with a line diff.
- To write or refresh goldens, run `UPDATE_GOLDEN=1 cargo test -p ralph_api --test health_contract`. Read every changed file before you commit.
- `cargo xtask lint` runs these tests, because it runs `cargo test --all-targets`.
