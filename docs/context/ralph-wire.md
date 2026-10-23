# ralph wire

`ralph wire` is a local HTTP proxy between a harness and a provider. It forwards every request unchanged and records one line per call. It sets no environment variables and starts no harness.

```bash
ralph wire [--provider anthropic] [--port N] [--upstream URL] [--dir PATH] [--responses]
ralph wire ls [--session ID] [--dir PATH] [--all] [--json]
ralph wire sessions [--dir PATH] [--json]
```

Flags are listed in the [CLI reference](../cli-reference.md#ralph-wire).

## Startup output

Listen URL, session directory, a plaintext warning, and a hint such as `export ANTHROPIC_BASE_URL=http://127.0.0.1:PORT`. Then one live row per finished inference call: sequence, UTC time, model, status, uncached input, cache read, cache writes by tier (`w5m=`, `w1h=`, `w?=`), output, cache hit ratio, input-equivalent cost (`-` when unrated), and `INCOMPLETE: <reason>` when the call did not finish. Non-inference calls are recorded but not shown. Run until Ctrl-C.

## Storage

Under `--dir` or `<tmp>/ralph-wire`, one folder per run named `<UTC timestamp>-<hex suffix>`. Directories are mode 0700, files 0600. Nothing is deleted or pruned. An existing root is used only when it is a non-symlink directory owned by the current user and its mode is 0700. Otherwise `ralph wire` refuses it and prints the path, what is wrong, and the command that fixes it, such as `chmod 700 <path>`.

- `session.json`: schema version, ralph version, provider, upstream, listen address, start time.
- `calls.jsonl`: one `CallRecord` per line, appended by a single writer thread.

Prompts and responses are stored in plaintext. Auth headers (`authorization`, `x-api-key`, cookies) are redacted. Only a response-header allowlist is stored; account identifiers are not.

## Inspecting recordings

`ralph wire ls` prints one row per inference call of a session (the latest unless `--session ID`), in the live-row format, then a total line. `--all` also lists non-inference calls as `other METHOD PATH`; they never count in totals. `ralph wire sessions` lists sessions under the root with call counts. `--dir` is the root in all three modes. `--json` prints `{rows, totals, prices_as_of}` for `ls` and an array for `sessions`; JSON rows carry the full usage outcome, including reasoning and other counters.

`ralph wire show N` prints one recorded call: the `session: ID` line, `call N`, a line `METHOD PATH status S` (`status -` when no response status was recorded, and prefixed with `other ` for a non-inference call), its prompt segments, request model and parameters, and usage. `N` is the call's `seq` as shown by `ls` (not its line position). It takes `--session ID`, `--dir PATH`, and `--json`. Segments follow the provider's cache order: tools, then system, then messages. Each row has a label (`tools[0] Read`, `system[1]`, `msg[2].0 user/text`), `bytes`, `cumulative` (bytes through the end of that segment), and a cache `breakpoint` where the block sets `cache_control`. A breakpoint is `5m` (no `ttl`, or `ttl` `"5m"`), `1h` (`ttl` `"1h"`), or `?` for any other `ttl` value, which is an unspecified tier. A null `cache_control` is no breakpoint. A JSON request body with no segments prints `no segments`; a body that was not JSON prints `request body is not JSON; no segments`.

`--json` prints `{session, seq, kind, method, path, status, body_is_json, segments: [{label, source, bytes, cumulative, breakpoint}], facts: {model, params}, usage, raw_usage}`. `kind` is `inference` or `other`; `status` is null when none was recorded; `source` is `tools`, `system`, or `messages`; `breakpoint` is null or a cache tier (`{"kind": "ttl", "seconds": 300}`, or `{"kind": "unspecified"}` for an unrecognised `ttl`). A request body that was not JSON has no segments and `body_is_json` is false.

`ralph wire diff A [B]` explains why call B did not hit the prompt cache that call A wrote. It takes `--session ID`, `--dir PATH`, and `--json`. With one argument, `A` is the call to explain and the base is chosen from earlier inference calls with JSON bodies: the one sharing the most leading identical segments (segments are compared without their `cache_control` marker, so a moved breakpoint is not a difference), ties to the highest `seq`; when no earlier call shares a segment, `a` is null and the verdict is `no_base` (a cold write when `cache_read` is 0, otherwise a cache read that came from outside the session). An unknown `seq` is an error. A must come before B and both must be inference calls, else the command fails. It reports differing model and parameters, differing cache-relevant headers (Anthropic: `anthropic-beta`, `anthropic-version`), the count of shared segments, the first diverging segment with its byte offset and ~40-byte excerpts, and the segments removed and added after it. Unlike `show`, the excerpts print prompt text from both calls. Segment text is re-encoded, so offsets are approximate. String `content` and a string `system` are encoded as one text block (`msg[i].0 role/text`, `system[0]`), so the two forms compare equal.

The `cache:` line gives one verdict, tested in order: no cache breakpoint in the shared prefix; B's `cache_read` is at least 90% (`CACHE_AGREE_RATIO`) of the estimated servable tokens, so the cache served the prefix (`prefix_changed` with the diverging label when the prompts diverge; otherwise `prefix_shared` with `relation` `extends`, `prefix_of`, or `identical`, printed as "b extends a", "b is a prefix of a", or "prompts are identical"); model changed; cache-relevant header changed; cache-relevant parameter changed (Anthropic: `thinking`, `tool_choice`); TTL expired; otherwise unexplained, listing causes wire cannot see (eviction, different account or workspace, provider-side change). Servable tokens are exact when an anchor exists: the highest-`seq` inference call with counted usage (partial counts), `seq` at or before B, whose segments through the last shared breakpoint are identical to B's (compared without their `cache_control` marker, as for base selection) and whose own last breakpoint is that same segment. Its `cache_read` plus cache writes are its prompt tokens through that breakpoint (`method` `anchor`, with the anchor `seq`; it may be B itself). Without an anchor, servable bytes (segments through the last shared breakpoint) are scaled by B's prompt tokens per byte (uncached input + cache read + cache writes) and marked `~` (`method` `bytes`). The TTL gap is refresh-aware: it runs from B's start to the end of the latest call in `[A, B)` whose segments through the breakpoint are identical to B's (this comparison keeps the breakpoint marker, so only a call that set it refreshes that entry), not from A. Limits: Anthropic's 20-block lookback is not modelled, and servable tokens are counted only through B's breakpoints in the shared prefix (a breakpoint only A set is not used). A breakpoint with an unspecified tier skips this check. Missing usage on B gives `usage_missing`; a non-JSON body on either side gives `no_segments`.

`--json` prints `{a, b, base_picked, facts: [{name, a, b}], headers: [{name, a, b}], shared_segments, divergence: {index, label_a, label_b, offset, a_excerpt, b_excerpt} | null, removed, added, explanation: {servable_bytes, servable_tokens, method: {kind: "anchor", seq} | {kind: "bytes"} | null, cache_read, verdict: {kind, ...}}}`; `a` is null for `no_base`; `verdict.kind` is one of `no_segments`, `no_base`, `no_shared_breakpoint`, `usage_missing`, `prefix_changed` (`{label}`, the diverging segment of B, never null), `prefix_shared` (`{relation}`: `extends`, `prefix_of`, or `identical`), `model_changed` (`{a, b}`), `header_changed` (`{names}`), `param_changed` (`{names}`), `ttl_expired` (`{gap_secs, ttl_secs, since_seq}`), `unexplained` (`{invisible}`). Options follow the subcommand name: `ralph wire diff --json 3 4` works, `ralph wire --json diff 3 4` does not.

Segment sizes and cumulative offsets are approximate. Raw request bytes are not recorded, so each size is the length of the block re-encoded from the parsed request JSON; key order, whitespace, and escaping may differ from the bytes sent, so the byte total is below the request `Content-Length`. Provider usage totals are ground truth.

- `usage=missing` rows and `usage=partial` rows are marked, never shown as zeros.
- Totals sum reported and partial usage only. The total line shows how many calls were counted, partial, and missing.
- Total `eq` is `-` (null in JSON) when no call was counted or any counted call is unrated.
- A priced row ends with `$=0.001234` (USD, six decimals), after `usage=partial` and `INCOMPLETE` when those marks are present. Unpriced rows show no `$=`. In JSON each row and `totals` carry `usd` (null when unpriced), and `prices_as_of` is the provider table date (null when no row is priced). Total `usd` is null when no call was counted, any counted call is unpriced, or any counted call is partial; the partial row still shows its own price.
- A final `calls.jsonl` line without a newline that is a truncated record (session in progress or interrupted) is ignored. `ls` and `show` write a note on stderr; the dropped record is not available to `show N`.
- Any other malformed line is an error naming the file and line number.
- A missing `calls.jsonl` counts as zero calls.
- `sessions` skips a damaged session with a `ralph wire: skipped ID: ...` warning on stderr and lists the rest.
- An unknown session id or a missing root is an error for `ls`; `sessions` prints `no sessions`.

## Prices

`ls` prices each counted call from the provider's dated price table (Anthropic: as of 2026-10-21, https://platform.claude.com/docs/en/about-claude/pricing). Lookup uses the model id with any `[...]` suffix removed; the longest matching id prefix wins and a prefix matches only an exact id or that id plus an 8-digit date (`-20261021`), so `claude-opus-4-5-20251101` matches `claude-opus-4-5` and `claude-opus-5-6` does not match `claude-opus-5`. An override uses the same rule. Claude Haiku 5.5 has two tiers: prompts over 100,000 tokens (uncached input plus cache reads plus cache writes, per call) use the higher prices; exactly 100,000 uses the lower. Cost is tokens times the per-MTok price for uncached input, cache read, each cache-write tier (5m, 1h), and output. A call with an unspecified cache-write tier, an unknown model, or missing raw usage is unpriced.

Shipped ids (each confirmed on the models overview and model pages): `claude-fable-5-1`, `claude-fable-5`, `claude-opus-5-5`, `claude-opus-5`, `claude-opus-4-8`, `claude-opus-4-7`, `claude-opus-4-6`, `claude-opus-4-5`, `claude-sonnet-5-5`, `claude-sonnet-5`, `claude-sonnet-4-6`, `claude-haiku-5-5`, `claude-haiku-4-5`. Mythos and Sonnet 4.5 are not shipped; price them through an override.

Overrides in `config.toml` replace the table for a model prefix at every prompt size. See [`[wire.prices]`](../configuration.md#wireprices---ralph-wire-ls-price-overrides). Unset override fields are derived from the override `input` times the provider's billing multipliers (the same ones behind `eq`).

Limits:

- The table is standard speed and global geography. Usage `speed` and `inference_geo` then adjust the price: `fast` is 2x on Claude Opus 5.5, Opus 5, and Opus 4.8 (the fast-mode page says caching multipliers stack on fast pricing); `fast` on any other model is unpriced. `inference_geo: "not_available"` is priced like `global`. `inference_geo: "us"` is 1.1x on all categories for Claude 4.6 and later; on Opus 4.5 and Haiku 4.5 it is unpriced. Any other `speed` or `inference_geo` value is unpriced. Factors also apply on top of an override.
- Batch calls are never priced: only `POST /v1/messages` is an inference call, so `/v1/messages/batches` traffic is `other` and excluded from totals.
- Discounts, credits, taxes, and Priority Tier commitments are not modeled. Server-tool charges (`server_tool_use`) are not priced.
- Prices are a snapshot. A model the table does not list needs an override.

## Proxy behaviour

- One thread and one connection per call; the connection closes after each response.
- Method, path, query, and body go upstream unchanged. Hop-by-hop headers and `host` are dropped. Other headers, including `accept-encoding` and credentials, are forwarded as given.
- A path that does not start with `/` gets 400 and goes nowhere. A request body over 32 MiB gets 413 and is not forwarded (the Messages API limit is 32 MB; Batch and Files uploads over 32 MiB are refused by the proxy).
- Response capture stops at 32 MiB while the relay continues; the call is marked `INCOMPLETE` with a truncation error. Responses to `HEAD` and with status 1xx, 204, or 304 relay no body.
- Live rows stop if stdout closes; recording continues.
- Redirects are not followed and bodies are not decompressed. The response is streamed to the client chunk by chunk and copied into a buffer.
- Decoding happens after the response: gzip and deflate are decoded; `br` and `zstd` are recorded as an `Unsupported` decode error with usage `missing`.
- Client disconnect or a ralph reset records the call as incomplete. Ctrl-C, SIGTERM, or SIGHUP accepts no new calls, waits up to 2 seconds for calls already started, then records any still open as incomplete (`INCOMPLETE: interrupted`) and exits. An unreachable upstream returns 502 and records the error.
- A request header with non-ASCII bytes is refused by `tiny_http`, which closes the connection with an empty reply. The call reaches neither the provider nor `calls.jsonl`.
- `metadata.user_id` is stored as `<redacted>` in both `facts.params` and `request_body`; the forwarded bytes are unchanged.

## Using it with chat

Start `ralph wire`, export the printed variable in the shell that will run `ralph chat`, then start the chat. The persistent claude process inherits it at spawn and respawns with ralph's env on a reset (`reset_reason` returns a reason), so all turns route through the proxy. Records carry no turn ids; correlate them with `usage.jsonl` by time.

Workers started in tmux get the environment of the tmux server, not the chat shell. Export the variable before the tmux server starts, or their traffic is not recorded.
