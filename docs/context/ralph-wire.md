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

`ralph wire ls` prints one row per inference call of a session (the latest unless `--session ID`), in the live-row format, then a total line. `--all` also lists non-inference calls as `other METHOD PATH`; they never count in totals. `ralph wire sessions` lists sessions under the root with call counts. `--dir` is the root in all three modes. `--json` prints `{rows, totals}` for `ls` and an array for `sessions`; JSON rows carry the full usage outcome, including reasoning and other counters.

- `usage=missing` rows and `usage=partial` rows are marked, never shown as zeros.
- Totals sum reported and partial usage only. The total line shows how many calls were counted, partial, and missing.
- Total `eq` is `-` (null in JSON) when no call was counted or any counted call is unrated.
- A malformed `calls.jsonl` line is an error naming the file and line number. An unknown session id or a missing root is an error for `ls`; `sessions` prints `no sessions`.

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
