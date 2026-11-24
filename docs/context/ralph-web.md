# ralph web Command

`ralph web [--store <DIR>] [--port <N>] [--tailscale]` serves a local HTTP API on 127.0.0.1. With `--tailscale` it also serves on the Tailscale IP of the machine (see [Tailscale](#tailscale)). It runs in the foreground until Ctrl-C. It serves the embedded shell, its stylesheet and font, `GET /api/health`, `POST /api/render` and `GET /tokens.css`.

## Code Locations

| Component | File | Description |
|-----------|------|-------------|
| CLI args | `crates/ralph/src/cli/mod.rs` | `Web(WebArgs)`: `--store`, `--port`, `--tailscale` |
| Shell | `crates/ralph/src/web.rs` | `run()`: store, token, bind, print URL, serve |
| Tailscale shell | `crates/ralph/src/web_tailscale.rs` | `discover()`: runs `tailscale ip` and `tailscale status --json` |
| Tailnet parsing | `crates/ralph_api/src/tailnet.rs` | pure: `parse_ip_output()`, `parse_status_json()`, `pick_outcome()`, `TailnetError` |
| Tokens CSS | `crates/ralph_api/src/tokens.rs` | pure `tokens_css()`: the CSS text |
| Colour roles | `crates/ralph_api/src/roles.rs` | pure `role_lines()`: the colour role tokens |
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

The stylesheet uses a 13px text size on a 20px line, 12px meta text, weights 400, 500 and 600, 24px rows, and gutters in `ch`.

## Start

Without `--tailscale`, `ralph web` prints one line: `ralph web listening on http://127.0.0.1:<port>/?token=<token>`. It also prints the store path. With `--tailscale` it prints a list of URLs. Open the URL. The server answers 302, sets the cookie and sends the browser to `/`.

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

Loopback only is the default. `--tailscale` adds one address; see [Tailscale](#tailscale).

- `ralph web` binds only to an address in the `BindPolicy`. `BindPolicy::loopback()` allows `127.0.0.1` and `::1`. It is the only place that holds these values. The shell reads the bind address from `BindPolicy::primary()` and the host names from `bind_hosts()`. An IPv6 host is written as `[::1]:<port>`.
- `bind()` checks the requested address. It binds. Then it checks the address that the OS bound.
- Any other address is refused with an error that names the address.
- A busy port gives `could not bind 127.0.0.1:<N>: <os error>`.
- On macOS, the bind to `127.0.0.1:<N>` can succeed while another process holds `0.0.0.0:<N>` with `SO_REUSEADDR`. `ralph web` does not detect this. Use `--port 0` to avoid it.

## Host check

- `route()` accepts a request only when the `Host` header is in `allowed_hosts`. The shell builds the list after the bind: `127.0.0.1:<port>` and `localhost:<port>`. With `--tailscale` the list is longer.
- A missing or other Host gives 403. This stops DNS rebinding.

## Port

- `--port` sets the port. The default is `0`, which picks a free port.
- `[web] port` in `config.toml` sets a stable port. Precedence: `--port`, then `[web] port`, then `0`. `--port 0` forces a free port.
- A busy port exits with 1: `port <N> in use (from --port|[web] port|ephemeral): <os error>`.
- With `--tailscale` and an ephemeral port, a tailnet bind that hits a taken port retries the whole loopback and tailnet pair up to 5 attempts in total, each with a fresh port. A fixed port never retries.
- The printed URL shows the real port.

## Tailscale

`ralph web --tailscale` serves on the Tailscale IPv4 address too. Without the flag the server is loopback only: `BindPolicy::loopback()` is unchanged.

Detection:
- `ralph web` runs `tailscale ip`. It tries `tailscale` on PATH first. Then it tries `/Applications/Tailscale.app/Contents/MacOS/Tailscale`.
- It never guesses the address from 100.64.0.0/10. That range is shared CGNAT space. The address that `tailscale ip` reports must be in 100.64.0.0/10. Any other address exits with 1 and an error that names the range.
- It uses the first IPv4 address. It ignores IPv6 and extra addresses.
- It exits with 1 and no fallback to loopback only when:
  - Tailscale is not installed. The error names both paths.
  - Tailscale is stopped or logged out. The error shows the first stderr line.
  - Tailscale reports no IPv4 address.
- `tailscale status --json` gives the MagicDNS FQDN (`Self.DNSName`). The machine name is the first label of that FQDN. `ralph web` does not use `Self.HostName`. If it fails, `ralph web` prints a warning and allows only the IP.

Bind:
- The servers bind `127.0.0.1` and exactly the reported IP, on the same port. With port 0, the second bind uses the port of the first.
- The policy is `BindPolicy::loopback_and(ip)`. `bind()` checks it before and after the bind.
- There are two servers on two threads. They share one token and one stop predicate. If one fails, both stop.
- Allowed Host values, each with `:<port>`: `127.0.0.1`, `localhost`, the IP, the machine name and the FQDN. The compare is case-insensitive. Any other name gets 403, which stops DNS rebinding.
- The start output lists the URLs: loopback, the Tailscale IP and the FQDN when known. Each has `?token=`.
- A stable port helps the tailnet URL: set `[web] port` (see [Port](#port)).

Risk:
- The traffic is plain HTTP inside WireGuard. A browser treats it as an insecure context. The cookie cannot have the `Secure` attribute.
- The cookie and URL token work as before. `ralph web` does not read Tailscale identity headers.
- Every device that the tailnet ACLs allow can reach the server, shared nodes included. Only the token stands between them and the data.
- If writes need per-user identity, use `tailscale serve` instead.

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
| `GET /api/memory` with `?thread=` and `?node=` | 200 root nodes of the memory tree (`{thread, entries, nodes}`), or with `node` one zoom step (`{thread, node, children}`); `after` and `limit` are ignored |
| `GET /api/memory/entries` with `?thread=`, `?after=` and `?limit=` | 200 `{thread, entries, items, next_after}`; `limit` defaults to 50, is at most 200 and `0` is 400; `next_after` is `null` on the last page; `node` is 400 |
| `GET /api/memory/threads` | 200 `{store, threads: [{name, parked, merged}]}` sorted by name; `store` is the last component of the store path (`store` when there is none), never the full path, and the shell header shows it with a `read-only` badge; any key other than `token` is 400 |
| bad `thread`, `node`, `after` or `limit`, an unknown or repeated key, or bad percent-encoding on a memory route | 400 `{"error":"<reason>"}` |
| unknown thread, or `node` past the last entry | 404 `{"error":"not found"}` |
| memory store missing or unreadable | 500 `{"error":"memory unavailable"}`, with no path and no system text |
| other method on a memory route | 405 with `Allow: GET` |
| other path | 404 `{"error":"not found"}` |

The 503 is the only answer that does not pass the Host and token checks. The server sends it before `route()` runs, so a request without a token also gets it. The other rows apply in the order shown, after the 503 row.

Every answer has `Cache-Control: no-store`. Every answer is JSON, except `/tokens.css`, `/catalogue`, the embedded assets and the 302 hand-off (it has no body). `/tokens.css` needs the token like every route, after the Host check. The health answer does not show the store path.

## Memory routes

The three memory routes read the chat store in `--store` (`thread=main`, the default) or a thread under `threads/<name>`. `thread` is `main` or a valid thread name. Each request reads the files once and never caches them. A thread list is read for each request.

- Exposed: `log.jsonl` entries (role, time, full text), `tree.jsonl` summaries, thread names and their parked and merged flags.
- Not exposed: `tags.jsonl`, analytics, `history.jsonl`, config, paths.
- The read takes no lock and writes nothing: no `LOCK`, no index, no new directory. It uses `ralph_server::chat::memory::peek`, so a half-written last log line and a tree node past the log are not shown.
- Code: `crates/ralph_api/src/memory_api.rs` (pure response builders and the thin read), `memory/query.rs` (query parser), `memory/json.rs` (JSON bodies).

## Embedded assets

- The files in `crates/ralph_api/assets/` are in the binary through `include_bytes!`. There is no build step.
- The CSP is `default-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`. Assets also send `Referrer-Policy: no-referrer` and `X-Content-Type-Options: nosniff`.
- The shell loads one script, `/app.js` (`<script src="/app.js" defer>`, same origin, so the CSP is unchanged: no `'unsafe-inline'`). It has no inline script and no `style=`. Its nav links use `#/name`.
- The font is the full variable font Geist Mono (`GeistMono[wght].woff2`) from vercel/geist-font tag v1.9.2, unchanged. It is not a latin-only subset. The repo name has no brackets: `fonts/geist-mono.woff2`.
- `fonts/NOTICE` records the tag, the source path and the SHA-256. `fonts/OFL.txt` is the upstream licence. `ralph licenses` prints both.
- The `[web]` font default starts with `"Geist Mono"`.
- `cargo xtask lint` runs a colour lint on `assets/`. It refuses raw colours, named colours (also in custom properties), `var()` outside `--r-*`, any `--r-*` definition (tokens come only from `/tokens.css`) and custom elements that are not one of the 14 allowed primitives. Thirteen are built: r-box, r-rule, r-table, r-kv, r-tree, r-pill, r-status, r-empty, r-error, r-spinner, r-spark, r-code and r-prose. The fourteenth, r-graph, passes the lint but has no builder yet. It skips binary assets (woff, woff2, png, jpg, jpeg, gif, ico, webp). It refuses `.svgz` files, because it cannot scan gzipped SVG; use plain `.svg`. A file that is not UTF-8, or that it cannot read, fails the lint. It does not scan `url(...)` contents or comments (`/* */`, and `//` in `.js` and `.mjs` only), and it does not read a css id selector such as `#add` as a hex colour. A `color-mix()` may take a `var(--r-*)` token as its percentage. In `.css` files it judges each declaration as one statement, even when it spans lines; HTML `style=` and `<style>` content stay judged line by line.
- A body of 32 KB or more goes out with chunked transfer encoding. The test client decodes it.

## Primitives

The UI has no JavaScript. Each primitive is a hyphenated tag that the server renders as plain HTML. A browser treats an unknown hyphenated tag as an inline element, and `app.css` styles it. The colour lint allows only these tags.

| Tag | Builder | Notes |
|-----|---------|-------|
| `r-box` | `frame(shape, title, inner_html)` | shape `square`, `round` or `heavy`; optional title |
| `r-rule` | `rule()` | `role="separator"` |
| `r-table` | `table(headers, rows)`, `table_state(headers, inner_html)` | `th scope="col"`; a short row is padded to the header count; a column whose non-empty cells are all numbers gets `class="num"` on `th` and `td` (right, tabular digits); `table_state` puts one primitive (`r-empty`, `r-spinner` or `r-error`) in a `tr.state` row that spans every column; see [r-table look](#r-table-look) |
| `r-kv` | `kv(pairs)` | a `dl` of `dt` and `dd` |
| `r-pill` | `pill(tone, shape, icon, text)`, `pill_with(variant, …)` | shape `square` or `round`; tone fill is 8% of the role over `--r-bg`, which keeps 4.5:1 for every tone in both themes (checked in `roles.rs`); `variant="outline"` has no fill; the fill lives on the tone rules in `app.css`; optional inline SVG icon (check, cross, warning, info, dot, pause) with `aria-hidden`; the text is cut with `…` at 32ch, and text over 32 chars is also in `title` |
| `r-status` | `status(tone, text)` | `role="status"`, `aria-live="polite"`; a word (OK, Error, Warning, Info, Note) shows the tone, so colour is never the only signal |
| `r-empty` | `empty(text)` | the empty state |
| `r-error` | `error(text)` | `role="alert"` |
| `r-spinner` | `spinner()` | `role="status"` with visually hidden "Loading" text |
| `r-spark` | `spark_svg(values, w, h)` | returns `None` for an empty series; the caller renders `r-empty` |

- Every builder is a pure function that returns a `String`. One `escape()` handles text and attribute values (`& < > " '`).
- Attribute values come from closed enums (`Tone`: ok, bad, warn, info, accent, neutral; `Shape`; `PillShape`; `PillVariant`; `Icon`), never from free text. `inner_html` of `frame()` must be built from these builders.
- No builder writes a `style` attribute, because the CSP forbids inline style. No builder writes a raw colour. SVG uses `stroke="currentColor"` and classes.
- `spark_points()` drops NaN and infinite values. A flat series and a single point draw a mid line. Huge values do not overflow.
- `render_markdown(&str)` parses with tables, strikethrough and task lists. Its own emitter writes the HTML. Raw HTML is escaped as text, never passed through. Links keep `href` only for `http`, `https`, `mailto` and `#fragment`. An email autolink becomes a `mailto:` link. The check trims control characters and whitespace at both ends, drops control characters inside, writes each inner whitespace character as `%20`, and ignores case. A space inside the scheme gives no `href`. Other links keep their text without `href`. Links get `rel="noreferrer noopener"`. An image is never `<img>`: its alt text goes in a `<span>`. Tags nested deeper than 64 levels render as plain text. NUL bytes become U+FFFD. A table becomes `<table tabindex="0">`. A fenced or indented code block becomes `<r-code data-lang="rust"><pre tabindex="0"><code>…</code></pre></r-code>`. `highlight()` in `code.rs` uses the `two_face::syntax::extra_newlines()` set (built once in `prose.rs`) and the syntect `ClassedHTMLGenerator` with class prefix `t-`. There is no generated theme CSS; `app.css` maps the `t-*` scope classes to `--r-*` slots. `data-lang` is the first word of the info string, lower-cased, and must match `[a-z0-9+#-]{1,32}`; otherwise the attribute is dropped. An unknown language, no language, a block over 64 KiB, or a block with a line over 4 KiB, gives plain escaped text inside `r-code`. The `[theme]` syntax theme does not apply on the web.
- Goldens are in `crates/ralph_api/tests/golden/render/<primitive>_<state>.html`. Unit tests in `render/` compare them. The states are empty, sparse, dense, error and very-long where they apply, plus hostile text for escaping.
- `app.css` holds the primitive styles, including `r-prose`, `r-code` and the `t-*` scope rules. They use role colours for everything except the `t-*` scope rules. Those rules use `--r-ansi-N` slots. Rows and inner rules use `--r-line`. Boxes, headers and code use `--r-line-strong`. Accent is only for focus (a 2px outline), links and the accent tone. The info tone and the accent tone differ. Code stays on one line and scrolls sideways inside `pre`. Both `pre` and a prose `table` carry `tabindex="0"`, so the keyboard can scroll them, and both show a `:focus-visible` outline. The spinner fades in after 200 ms from opacity 0, so a short load shows no spinner. `prefers-reduced-motion` turns its animation off.

## Catalogue

`GET /catalogue` shows every primitive in the states that apply to it. r-box, r-table, r-kv, r-pill, r-status, r-spark, r-prose and r-code show all five: empty, sparse, dense, error and very-long. r-table also shows a loading state; its empty, loading and error states are state rows. r-pill also shows an outline state. r-empty shows empty, sparse and very-long. r-error shows empty, error and very-long. r-rule and r-spinner show only sparse. The r-prose and r-code states come from `render_markdown()` on short markdown constants in the catalogue module, not from the fixture, because the fixture holds no markdown; the error state is an `r-error`. The page has two columns, `<section data-theme="dark">` and `<section data-theme="light">`. The shell nav links to `/catalogue`. It is not `#/catalogue`: the shell has no script, so a hash link cannot reach a server page. The page has a link back to `/`.

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
- An authorised `POST /api/render` can announce a body under the cap (by `Content-Length` or chunked) and never send it. That request keeps its slot until the read timeout below closes it. Any other request gets its answer without a body read.
- tiny_http 0.12.0 runs one thread for each open connection: the accept loop in `Server::from_listener` hands each connection to `TaskPool::spawn`, which starts a new thread when none is free (`lib.rs:323`, `util/task_pool.rs:69-76`). An admitted stalled request costs two threads: the tiny_http connection thread and the handler thread that holds the slot. Past 8 requests in progress, a stalled request costs the connection thread and a thread that sends the 503. All of these threads end when the client closes the socket or a timeout fires.
- The server reads a request body up to 256 KiB, and only for an authorised `POST /api/render`. It checks Host and token on the headers first. A request that fails a check, or that targets another path, gets its answer without a body read, so `Expect: 100-continue` gets no `100 Continue`. A `Content-Length` over the cap gets 413 `{"error":"too large"}` at once, without a body read, on every route and method, before the Host and token checks. The bytes read are checked again against the cap. The 413 comes after the 503 check and before the 403 and 401 checks, so it leaks nothing about auth. 
- Each connection has a 10 s read timeout and a 10 s write timeout (`Limits` in `server.rs`). They come from the vendored tiny_http `Timeouts` (`vendor/tiny_http`). They apply to loopback too. A stalled request body is closed by the read timeout.
- After a rejected request (401, 403 or 413), the handler thread drains the unread body while it holds the handler slot. The drain stops after 64 KiB or 2 s since it started. It checks both between reads. One rejected request holds a slot for at most about 2 s plus one 10 s read, which is about 12 s.
- The drain reads through a fixed 8 KiB buffer (the vendored `EqualReader` in `vendor/tiny_http/src/util/equal_reader.rs`). A huge declared `Content-Length` does not allocate that size.
- Known limit: the slot stays held during the drain. tiny_http drops the request, and so drains it, inside `respond()`. To release the slot before the drain, the vendored crate needs a hook. This is not done.
- Known limit: a client that sends headers slowly (header slowloris) is bounded only by the per-read timeout. tiny_http reads headers before the handler slot is taken.
- Known limit: a drain that stops early leaves the connection open (keep-alive). tiny_http drops a `Connection` header set on a response and does not close on it. The leftover body bytes can be parsed as the next request on that connection. Garbage gets a 400 and a close. A crafted body is routed and authenticated like any request, so it gives no auth bypass.
- Residual risk: up to 8 allowed tailnet devices that keep reconnecting and sending rejected requests can still keep the server on 503. An accept cap or a per-request total deadline is the follow-up. It is not done.
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
- The dark table is in the default `:root` block. The light block overrides the 16 ANSI variables and the roles inside `@media (prefers-color-scheme: light)`.
- `[theme] border` does not drive web borders. The web uses `--r-line` and `--r-line-strong`.

### Colour roles

`role_lines()` in `roles.rs` writes the colour roles. Roles come from the ANSI table, so one table change moves every role. The roles are in the `:root` block, the light `@media` block and both `[data-theme]` blocks.

- There are 15 roles: `--r-bg`, `--r-ink`, `--r-ink-strong`, `--r-muted`, `--r-faint`, `--r-line`, `--r-line-strong`, `--r-surface`, `--r-hover`, `--r-accent`, `--r-selected`, `--r-ok`, `--r-warn`, `--r-info` and `--r-bad`.
- There are 3 heading roles: `--r-h1`, `--r-h2` and `--r-h3`. Each mixes its heading slot toward ink by `--r-head-k`. `--r-head-k` is 62% in dark and 50% in light. A heading role falls back to ink if its heading slot is unset.
- Each theme picks its sources with `--r-bg-src`, `--r-ink-src`, `--r-ink-strong-src`, `--r-accent-src` and `--r-bad-src`. Two more values, `--r-tone-k` and `--r-head-k`, set the mix strength.
- `--r-bg`, `--r-ink`, `--r-ink-strong`, `--r-accent` and `--r-bad` copy their `-src` pick.
- `--r-muted` is ink 62% in bg. `--r-faint` is ink 42% in bg.
- `--r-line` is ink 14% in bg. `--r-line-strong` is ink 30% in bg.
- `--r-surface` is ink 5% in bg. `--r-hover` is ink 9% in bg.
- `--r-selected` is accent 16% in bg.
- `--r-ok`, `--r-warn` and `--r-info` are ANSI 2, 3 and 6 at `--r-tone-k` in ink. `--r-tone-k` is 100% in dark and 62% in light.
- `--r-faint` and `--r-line-strong` are for decoration (guides, rules, borders). Do not use them for text. They do not reach 4.5:1: about 2.97 and 2.82 for `--r-faint`, and 2.07 and 2.00 for `--r-line-strong`, on the background in dark and light. `--r-muted` and the heading roles reach AA 4.5:1 text contrast in both themes. `--r-muted` gives 5.16 in dark and 5.37 in light.
- In the light theme, the roles take the background from ANSI slot 8. They take text and strong text from slot 7. The default light table has `#f2f2f2` in slot 8 and `#0c0c0c` in slot 7. Slot 0 is `#cccccc`. The page background does not use it. A custom `[web] ansi_light` table that changes slot 7 or 8 moves every role, so contrast can drop below AA. Dark uses slot 0 for the background, slot 7 for text and slot 15 for strong text.
- The slots `inline-code-bg`, `status-base-bg` and `ui-rule` come from roles. `inline-code-bg` and `status-base-bg` use `--r-surface`. `ui-rule` uses `--r-line-strong`.
- For these three slots, the web maps the slot to its role when the value equals the default colour. A user value that differs stays as written. A slot set to exactly the default colour also maps to the role, because the web cannot tell it from an unset slot.

## Store

- `--store` sets the store. Without it, `RALPH_CHAT_STORE` is used, then `./.ralph`.
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

## Memory screen

- Route `#/memory`. Other routes keep the placeholder text. All code is in `assets/app.js`: plain script, no framework, no build step. Pure helpers (`parseHash`, `buildHash`, `defaultAfter`, `olderAfter`, `newerAfter`, `navTarget`) sit apart from the DOM code.
- State is in the hash: `#/memory?thread=<name>&node=<id>&after=<n>`. Defaults (`main`, no node, no `after`) are left out. `hashchange` draws the screen again, so back and forward work. Zooming a node updates `node` with `replaceState`.
- The screen shows a thread select (`main` and the names from `/api/memory/threads`), thread and entry counts, the root nodes as an `r-tree`, and a `recent entries` table (20 rows) with `older` and `newer` links that change `after`. With no `after` in the hash, the table opens on the newest page (`max(0, entries - 20)`, from `defaultAfter`). An `after` in the hash, even `after=0`, overrides this. An `after` at or past the entry count opens the newest page (`clampAfter`).
- Zoom: click or Enter on a node button fetches `/api/memory?node=`. It adds the summary, then child blocks or entry rows (`#i role`, time, first line of the text). A node with no summary shows `summary pending` in the zoom summary row only. Its tree row shows the neutral `pending` pill.
- An entry row opens its full text. Summary and entry text go to `POST /api/render` and the returned `r-prose` HTML is the only `innerHTML` in the script. All other text uses `textContent`.
- Requests use the cookie (`credentials: same-origin`). The script never reads the token.
- States: loading (`r-spinner` after 200 ms), empty store (`r-empty`), HTTP error (`r-error` with status, server message and a retry button), and 404 for an unknown thread or node.
- Keyboard: Tab reaches the select, the links and one tree item (roving tabindex). Up and Down move between visible items. Right expands or goes to the first child. Left collapses or goes to the parent. Home and End go to the first and last item. Enter and Space zoom. Focus returns to the same item after a redraw.
- Not shown: `tags.jsonl`, analytics, `history.jsonl`, paths.
- Contract tests: `tests/memory_screen_contract.rs` (script served like the other assets, one `innerHTML`, no `eval(`, `style.`, `<img`, only primitive tags).

## r-tree markup

- `tree(&[TreeNode])` in `render/tree.rs` builds `<r-tree><ul role="tree">…</ul></r-tree>`. No nodes gives `<r-tree><r-empty>…</r-empty></r-tree>`.
- A `Branch` is `<li role="treeitem" aria-expanded aria-level><button type="button">guides, chevron, <span class="id">label</span></button>cells<ul role="group">…</ul></li>`. A `Leaf` has `<span class="label">` with the same parts and no button. A `Row` is `<li role="none">` and holds a ready `r-error` or `r-empty`.
- Cells come from `Cells { count, extent, summary }`. They follow the label as `<span class="n">`, the extent bar and `<span class="sum">`. Each row is a CSS grid: `24ch 7ch 9ch 1fr`. The count column fits six digits. The nested `ul` spans all columns on row 2. The label column is fixed, so a deep label can clip.
- Only native tags go inside `r-tree`, plus inline `svg`, `path`, `rect` and `r-pill`. The depth comes from `.g` guide spans, one per level below the first. `aria-level` starts at 1.
- The chevron is an inline SVG that turns 90 degrees when the branch is open. The CSS has no `data:` URI and no inline style.
- The extent bar is an inline SVG with `viewBox="0 0 100 6"` and `preserveAspectRatio="none"`. A grey track holds one span from `lo` to `hi` of `total`. A span is never narrower than 1 unit, so a one-entry node stays visible when the total is 5,000 or more. Without an extent the cell is an empty `<span class="extent">`.
- `Summary` is `Empty`, `Text` or `Pending`. Text is escaped and its whitespace is collapsed to one line. The CSS adds the ellipsis when the text is too long. `Pending` draws one neutral `r-pill` with the text `pending`.
- `aria-expanded="false"` hides the child group in CSS. The children stay in the HTML.
- The button has no default chrome and a `:focus-visible` outline. `/app.js` makes the button toggle the branch.
- Every label and summary text goes through `escape()`. Long text wraps with `overflow-wrap: anywhere`.
- The memory screen does not call the Rust renderer. `app.js` builds the same markup, and `tests/memory_screen_contract.rs` reads the `tree_extent` and `tree_pending` goldens and checks that the script builds the same classes, SVG and attributes. It also checks the keyboard selectors `li[role=treeitem] > button` and `:scope > ul[role=group]`.
- The catalogue shows empty, sparse, dense (4 levels), error, very-long, collapsed, expanded, hostile, pending and focus; the code is in `catalogue/tree.rs`.

## r-table look

- The header is muted, 12px and does not wrap. `.num` cells align right with tabular digits. A stray number in a text column does not change that column.
- `.clip` cuts a cell on one line with a CSS ellipsis on the real column width. `app.js` sends the first line of an entry whole and no longer cuts it at a fixed length.
- Rows reserve a 2px left bar. Hover fills with `--r-hover` and leaves the text colour alone. `aria-selected="true"` fills with `--r-selected` and shows an accent bar. `.open` shows the accent bar with no fill. These are styles only; the screen sets no row state yet.
- `.stackable` is opt-in and only the entries table has it. At 760px or narrower it hides the header and turns each row into two lines: `#`, role and time, then the text, clipped at two lines.
