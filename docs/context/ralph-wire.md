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

Under `--dir` or `<tmp>/ralph-wire`, one folder per run named `<UTC timestamp>-<hex suffix>`. Directories are mode 0700, files 0600. Nothing is deleted or pruned.

- `session.json`: schema version, ralph version, provider, upstream, listen address, start time.
- `calls.jsonl`: one `CallRecord` per line, appended by a single writer thread.

Prompts and responses are stored in plaintext. Auth headers (`authorization`, `x-api-key`, cookies) are redacted. Only a response-header allowlist is stored; account identifiers are not.

## Inspecting recordings

`ralph wire ls` prints one row per inference call of a session (the latest unless `--session ID`), in the live-row format, then a total line. `--all` also lists non-inference calls as `other METHOD PATH`; they never count in totals. `ralph wire sessions` lists sessions under the root with call counts. `--dir` is the root in all three modes. `--json` prints `{rows, totals, prices_as_of}` for `ls` and an array for `sessions`; JSON rows carry the full usage outcome, including reasoning and other counters.

`ralph wire show N` prints one recorded call: its prompt segments, request model and parameters, and usage. `N` is the call's `seq` as shown by `ls` (not its line position). It takes `--session ID`, `--dir PATH`, and `--json`. Segments follow the provider's cache order: tools, then system, then messages. Each row has a label (`tools[0] Read`, `system[1]`, `msg[2].0 user/text`), `bytes`, `cumulative` (bytes through the end of that segment), and a cache `breakpoint` where the block sets `cache_control` (`5m` or `1h`). `--json` prints `{seq, body_is_json, segments: [{label, bytes, cumulative, breakpoint}], facts: {model, params}, usage, raw_usage}`; `breakpoint` is null or a cache tier (`{"kind": "ttl", "seconds": 300}`). A request body that was not JSON has no segments and `body_is_json` is false.

Segment sizes and cumulative offsets are approximate. Raw request bytes are not recorded, so each size is the length of the block re-encoded from the parsed request JSON; key order, whitespace, and escaping may differ from the bytes sent. Provider usage totals are ground truth.

- `usage=missing` rows and `usage=partial` rows are marked, never shown as zeros.
- Totals sum reported and partial usage only. The total line shows how many calls were counted, partial, and missing.
- Total `eq` is `-` (null in JSON) when no call was counted or any counted call is unrated.
- A priced row and the total line end with `$=0.001234` (USD, six decimals). Unpriced rows show no `$=`. In JSON each row and `totals` carry `usd` (null when unpriced), and `prices_as_of` is the provider table date (null when no row is priced). Total `usd` is null when no call was counted or any counted call is unpriced.
- A final `calls.jsonl` line without a newline that is a truncated record (session in progress or interrupted) is ignored. `ls` and `show` write a note on stderr; the dropped record is not available to `show N`.
- Any other malformed line is an error naming the file and line number.
- A missing `calls.jsonl` counts as zero calls.
- `sessions` skips a damaged session with a `ralph wire: skipped ID: ...` warning on stderr and lists the rest.
- An unknown session id or a missing root is an error for `ls`; `sessions` prints `no sessions`.

## Prices

`ls` prices each counted call from the provider's dated price table (Anthropic: as of 2026-10-21, https://platform.claude.com/docs/en/about-claude/pricing). Lookup uses the model id with any `[...]` suffix removed; the longest matching id prefix wins and a prefix matches only at a `-` boundary, so `claude-opus-4-5-20251101` matches `claude-opus-4-5`. Claude Haiku 5.5 has two tiers: prompts over 100,000 tokens (uncached input plus cache reads plus cache writes, per call) use the higher prices; exactly 100,000 uses the lower. Cost is tokens times the per-MTok price for uncached input, cache read, each cache-write tier (5m, 1h), and output. A call with an unspecified cache-write tier, an unknown model, or missing raw usage is unpriced.

Shipped ids (each confirmed on the models overview and model pages): `claude-fable-5-1`, `claude-fable-5`, `claude-opus-5-5`, `claude-opus-5`, `claude-opus-4-8`, `claude-opus-4-7`, `claude-opus-4-6`, `claude-opus-4-5`, `claude-sonnet-5-5`, `claude-sonnet-5`, `claude-sonnet-4-6`, `claude-haiku-5-5`, `claude-haiku-4-5`. Mythos and Sonnet 4.5 are not shipped; price them through an override.

Overrides in `config.toml` replace the table for a model prefix at every prompt size. See [`[wire.prices]`](../configuration.md#wireprices---ralph-wire-ls-price-overrides). Unset override fields are derived from the override `input` times the provider's billing multipliers (the same ones behind `eq`).

Limits:

- Standard rates only. Usage `speed` and `inference_geo` adjust the price: `fast` is 2x on Claude Opus 5.5, Opus 5, and Opus 4.8 (the fast-mode page says caching multipliers stack on fast pricing); `fast` on any other model is unpriced. `inference_geo: "us"` is 1.1x on all categories for Claude 4.6 and later; on Opus 4.5 and Haiku 4.5 it is unpriced. Any other `speed` or `inference_geo` value is unpriced. Factors also apply on top of an override.
- Batch calls are never priced: only `POST /v1/messages` is an inference call, so `/v1/messages/batches` traffic is `other` and excluded from totals.
- Discounts, credits, taxes, and Priority Tier commitments are not modeled. Server-tool charges (`server_tool_use`) are not priced.
- Prices are a snapshot. A model the table does not list needs an override.

## Proxy behaviour

- One thread and one connection per call; the connection closes after each response.
- Method, path, query, and body go upstream unchanged. Hop-by-hop headers and `host` are dropped. Other headers, including `accept-encoding` and credentials, are forwarded as given.
- Redirects are not followed and bodies are not decompressed. The response is streamed to the client chunk by chunk and copied into a buffer.
- Decoding happens after the response: gzip and deflate are decoded; `br` and `zstd` are recorded as an `Unsupported` decode error with usage `missing`.
- Client disconnect, a ralph reset, or Ctrl-C mid-stream records the call as incomplete. An unreachable upstream returns 502 and records the error.
- A request header with non-ASCII bytes is refused by `tiny_http`, which closes the connection with an empty reply. The call reaches neither the provider nor `calls.jsonl`.
- `metadata.user_id` is stored as `<redacted>` in both `facts.params` and `request_body`; the forwarded bytes are unchanged.

## Using it with chat

Start `ralph wire`, export the printed variable in the shell that will run `ralph chat`, then start the chat. The persistent claude process inherits it at spawn and respawns with ralph's env on a `reset_due` reset, so all turns route through the proxy. Records carry no turn ids; correlate them with `usage.jsonl` by time.

Workers started in tmux get the environment of the tmux server, not the chat shell. Export the variable before the tmux server starts, or their traffic is not recorded.
