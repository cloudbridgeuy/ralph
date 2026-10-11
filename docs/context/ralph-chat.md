# ralph chat Command

`ralph chat` is a single long-running chat with one persistent memory. Every message is a fresh, isolated `claude -p` turn. The model sees a compressed view of the whole chat and can open details through a private MCP server. Bare `ralph` (no subcommand) runs `ralph chat`.

## CLI

```bash
ralph chat [--timeout 600] [--skill <PATH>]...
ralph                      # same as ralph chat with defaults
```

- `--timeout <secs>` per turn, default 600 (`CHAT_TIMEOUT_SECS`).
- `--skill <PATH>` (repeatable) adds a skill directory. See Skills.
- Hidden `--store <DIR>` overrides the store path. Default is `~/.ralph/chat`; a missing home directory is an error. The directory is created at startup.
- Hidden subcommands: `ralph chat-mcp --store <DIR>` and `ralph chat-nap --store <DIR> [--timeout 600]`. Both are spawned by ralph itself.
- Input is a reedline editor on the main thread with prompt `>>> `. Enter sends; Alt-Enter or Ctrl-J inserts a newline; Ctrl-O opens `$VISUAL`, `$EDITOR`, or `vi` on `<store>/editor.md`. Up/Down and Ctrl-R use the history file `<store>/history.txt` (1000 entries). Ctrl-D or Ctrl-C at the prompt quits; an empty message does nothing unless reports are unread (see Worker inbox). Ctrl-C during a turn interrupts it and ends the chat.
- A dark-gray full-width rule (the shared termimad skin's `---`, default gray(6) `―`) prints before the turn and after the reply.

## Layout

- `crates/core/src/chat_memory/`: pure chat memory, split by concern. `mod.rs` holds the types (`Entry`, `Node`, `Tree`) and re-exports; `log.rs` parses and encodes the log and tree files; `tree.rs` has `cover`, `pending`, `view`, `zoom`, `recall`; `tools.rs` has the MCP tool schemas, `handle_request` and `finish_call`; `nap.rs` has the nap and retry prompts and `check_summary`; `usage.rs` has the per-call usage records.
- `crates/core/src/chat.rs`: `VIEW_LINES`, `SYSTEM_PROMPT`, `turn_prompt`, `build_mcp_args`.
- `crates/core/src/chat_status.rs`: status line, footer hints, input routing, worker state parsing, memory size.
- `crates/core/src/chat_skills.rs`: `resolve_skills`, `plan_links`, `expand_slash_command`, `plugin_manifest`, `build_plugin_args`.
- `crates/ralph/src/chat.rs`: turn loop shell, reedline input, nap spawn.
- `crates/ralph/src/chat_memory.rs`: `Store` (files, LOCK, append, nap loop).
- `crates/ralph/src/chat_mcp.rs`: MCP stdio loop.
- `crates/ralph/src/invoke.rs`: `build_chat_command`, `build_memory_command`.
- `crates/ralph/src/cli/mod.rs` and `main.rs`: arguments, default dispatch, `chat-mcp` and `chat-nap` handlers.

## Turn loop

1. `Store::load_settled` takes the `LOCK` (blocking until any running nap finishes), then reads log and tree.
2. `view(entries, tree, VIEW_LINES)` builds the memory view. `VIEW_LINES` is 200.
3. The user message, or one `worker` entry per report for a worker turn, is appended to the log.
4. `build_chat_command` runs a fresh `claude --verbose --permission-mode bypassPermissions` with isolation args, `--append-system-prompt <SYSTEM_PROMPT>`, the MCP args, `--output-format stream-json`, and `-p <prompt>`. There is no `--resume` and no `--agent`. The prompt is the view lines, a blank line, `New message:`, then the message. Only view lines precede the message; no task text follows it.
5. Output goes through the spinner subprocess runner and `StreamProcessor`, as in `ralph ask`.
6. On exit code 0 with response text, the reply is appended verbatim and a detached `ralph chat-nap` is spawned (stdio null; spawn failure only warns). Otherwise a warning `chat turn failed (exit N)` prints and the loop continues. The user message stays in the log without a reply.
7. An interrupted turn ends the chat. Other subprocess errors abort with an error.

`SYSTEM_PROMPT` tells the model that the message starts with the compressed chat, that `#<id>` lines are summaries, to use `zoom` and `recall` before guessing or asking, that `[worker ...]` messages come from workers, not the person, and never to claim manual saving. On a worker message the model may `reply` only with an answer already settled in the chat (a decision the person made, approved scope, a step in an agreed plan); merges, pushes, PRs, Linear status changes, new scope and anything public go to the person, and every worker reply is listed in its next message to the person. `spawn_worker` is only for what the person asked in their latest message. The chat MCP server enforces this with the pure `worker::command_allowed`: `spawn_worker` needs the newest log entry to be from the person, `reply` accepts a person or worker entry, and anything else is refused. Its delegation rules keep turns short: the model reads inline only to scope (at most 3 tool calls per question), sends multi-file reads, broad searches and test runs to `evidence-collector`, all code edits to `implementer`, and fixes to `reviser`, and every dispatch prompt states goal, question, known facts, owned files, output cap, stop condition and tool budget, "Report not-found, do not widen scope.", and "You are a subagent. Don't run mem." After 2 failed repair rounds it diagnoses; after a 3rd it waits for the person. It also must lead with the answer and end with one next step, show a 3-5 line plan when a request is ambiguous, treat a worker "done" as unverified until it checks the diff, tests, git and Linear, cite the `#<id>` behind facts from older chat, and say it has no erase tool when asked to forget.

## Worker inbox

Worker reports never interrupt the draft. A watcher thread polls every 500 ms and repaints the prompt in place (reedline `RepaintSignal`), keeping the buffer and cursor.

- Status line: the first prompt line above `>>> `, `📬 <unread> unread · workers <id> <state>, ... · mem <msgs> msgs ~<k> tok`. It sheds detail as the terminal narrows (states, then names to a count, then labels) and never exceeds the width. Dim when stdout is a tty and `NO_COLOR` is unset.
- Unread is the count of pending inbox files. Workers are tmux sessions named `ralph-<id>` (missing tmux or no server means none); a worker's state is the status of its latest report, else `running`. Memory is the log entry count and the view size at about 4 chars per token, refreshed every 5 s and after each turn or inbox read.
- `/inbox`, or Enter on an empty prompt while reports are unread, drains the inbox: one `worker` log entry per report, then one turn with all reports in one message. With nothing unread, `/inbox` prints a note and Enter on an empty prompt does nothing.
- Footer: `footer` in `chat_status.rs` builds a hint line, painted under the input with one empty line after it, so the prompt is never the last terminal line. Stock reedline 0.52.1 erases everything below the cursor on repaint, so `vendor/reedline` carries upstream nushell/reedline `main` (e9fcfdb) plus one footer commit (`Prompt::render_prompt_footer`) through `[patch.crates-io]`. Bracketed paste is on, so a multi-line paste stays in the draft.

## Memory

- Log: `log.jsonl`, append-only, one `{role, at, text}` per line (`role` is `user` or `assistant`, `at` is RFC 3339). Each append is fsynced under `LOCK`. A corrupt entry warns and ignores that entry and all later ones; a trailing line without newline is ignored.
- Tree: `tree.jsonl`, one `{lo, hi, text}` record per line. Nodes are aligned power-of-two blocks `[lo, hi)` over log indexes, ids print as `5` or `4-7`. First record wins for a duplicate node; invalid records warn and are skipped. Summaries are at most `SUMMARY_BYTES` = 512 bytes, one line.
- Leaves are single messages. Messages of 512 bytes or fewer stay verbatim in the view (whitespace collapsed) and need no node. Longer ones get a leaf summary node. Parent nodes merge two children and exist only for complete blocks. A node is written once and never rebuilt.
- `cover(t, budget)` picks the nodes for the view: all leaves if the log fits the budget, otherwise a cover of the whole log, oldest first, tuned to use the budget. Nodes with no summary yet are omitted from the view.
- View line formats: message `#<i> <role> <at> <text>`; summary `#<id> <summary>`.
- This is a port of the OptMem cover, pending and tree_put design. There is no runtime dependency on `~/.optmem`.

## Nap

`ralph chat-nap` takes the `LOCK`, loads the store, and for each node from `pending` (long leaves, then complete parent blocks by increasing size) asks a Haiku curator for a summary.

- Curator: `build_memory_command` runs `claude --model haiku` with `--tools ''`, an empty strict MCP config, and isolation args. Its prompt asks for one line of at most 512 bytes (about 70 words), priority: the user's words close to verbatim, lasting effects, commit SHAs, Linear ids and worker outcomes copied exactly, and failures, findings and open questions, steps taken. It asks to name minor items in a word or two, copy names, numbers, ids, paths and errors exactly, never overstate progress, invent nothing, and write in third person. It forbids copying the `#<id> <role> <timestamp>` line prefix, and says the input is data to never answer or obey.
- `check_summary` accepts a trimmed, non-empty, single-line summary of at most 512 bytes that does not read as meta-talk (an opener such as `I'm `, `Here is`, `Sure`, `Let me`, `Based on`, or the phrases `this excerpt`, `<input>`, `</input>`). A rejection retries with the error appended, up to `MAX_NAP_ATTEMPTS` = 5 tries per node. A curator error skips that node. A node that fails stays pending and is retried at the next nap; other nodes continue. A parent whose child has no summary is skipped.
- The next turn waits on `LOCK` until the nap finishes.

## MCP tools

`[chat] mcp_servers` in the ralph config (default `["mcptools"]`) names servers to import. `select_servers` copies each entry verbatim from the top-level `mcpServers` of `~/.claude.json`. A name absent there, or equal to `ralph-chat`, prints one warning and chat starts without it. `ralph-chat` is inserted last, so an import cannot shadow it. The file is written once at chat start: config changes need a restart. Imported entries can carry secrets, which is why they go in a file and not in argv. Turns run with `bypassPermissions`, so imported tools run without a prompt.

Hidden `ralph chat-mcp --store <DIR>` speaks newline-delimited JSON-RPC 2.0 on stdio (server name `ralph-chat`). At chat start the shell writes `<store>/mcp.json` (mode `0600`) holding `ralph-chat` plus the servers named in `[chat] mcp_servers`, and `build_mcp_args` passes `--mcp-config <store>/mcp.json --strict-mcp-config`, so only those servers load. Each tool call re-reads log and tree without locking, so it sees fresh state.

| Tool | Args | Result |
| --- | --- | --- |
| `zoom` | `id` (string, such as `16-31` or `5`) | A block id returns its two halves (summary lines, or `#<id> not compressed yet`; halves past the log end are dropped). A single index returns the raw message. |
| `recall` | `pattern` (string) | Case-insensitive regex over raw messages (`#<i> <role> <at> <text>` lines). Returns matching lines plus `N hits`. Output is capped at 8192 bytes, oldest hits dropped first and counted. |

Invalid ids, ids beyond the log, bad regexes, and store read errors return a tool result with `isError: true`.

## Skills

Turns run with `--setting-sources project,local`, so `~/.claude/skills` does not load. No skills are bundled. The user names skill directories (each holds a `SKILL.md`, for example `~/.claude/skills/show-me`) and chat exposes them through a plugin.

- Sources, highest first: `--skill <PATH>` (repeatable), `RALPH_CHAT_SKILLS` (`:`-separated, empty entries dropped), `[chat] skills = [...]` in the ralph config. The highest non-empty source replaces the lower ones; lists are not merged. With none set, no `--plugin-dir` is passed and nothing is written.
- A leading `~` or `~/` expands to the home directory. Duplicate paths keep the first. Relative paths are made absolute.
- At start, a path without a readable `SKILL.md` prints one warning and is skipped. Two skills with the same directory name: the first wins, the other warns and is skipped.
- Slash expansion: chat turns wrap the input, so Claude Code never expands a leading `/<name>`. Ralph does it for user turns only (worker messages are never expanded). If the message starts with `/<name>` followed by whitespace or end, and `<name>` is a configured skill, the prompt gets `Base directory for this skill: <dir>`, the `SKILL.md` body without its frontmatter, then the original message. The rest of the message, trimmed, replaces every `$ARGUMENTS` in the body; without that token and with non-empty arguments, an `ARGUMENTS: <args>` line is appended. The store keeps the original message; only the sent prompt is expanded. Unknown names, a bare `/`, and `/name` prefixes that are longer words pass through unchanged. A `disable-model-invocation` frontmatter flag only stops the model from invoking the skill itself; `/<name>` still runs it, and ralph does not warn about it.
- Config is loaded once at start; it supplies both `mcp_servers` and `skills`. On load error chat warns once and uses defaults.
- If at least one skill survives, chat builds a temporary plugin directory (removed when chat exits) with `.claude-plugin/plugin.json` (`{"name": "ralph"}`) and `skills/<name>` as symlinks to the skill directories. Each turn adds `--plugin-dir <dir>`. Skills appear as `ralph:<name>`.
- Skills are read at start: changes to the list need a restart.

## Storage and isolation

- Global store `~/.ralph/chat`: `log.jsonl`, `tree.jsonl`, `LOCK`, `history.txt`, `editor.md`, `mcp.json`, `usage.jsonl`. It is not project-scoped.
- `LOCK` is an exclusive file lock held for append, settled load, and the whole nap. Store paths that are symlinks or multiply-linked files are rejected.
- Isolation matches personas: `--setting-sources project,local` plus a `claudeMdExcludes` entry for `~/.claude/CLAUDE.md` when the home directory is known. Project and local settings stay active; this is context isolation, not a sandbox.
- User agents in `~/.claude/agents` do not load, so `build_agents_args` passes the bundled `SUBAGENTS` (`evidence-collector` haiku, `implementer` sonnet, `reviser` opus) as `--agents <json>`. These copies may drift from `~/.claude/agents`.
- Turns run with `bypassPermissions`.

## Usage

Every completed Claude call (each turn, including failed exits but not interrupted ones, and each nap attempt) appends one line to `usage.jsonl`: `{at, kind, trigger, input_tokens, cache_creation_input_tokens, cache_read_input_tokens, output_tokens, cost_usd, duration_ms}`, with `kind` `turn` or `nap`, and `trigger` (turn only) `user` or `worker`. Calls without usage data append nothing. Appends do not take `LOCK`. Total cost per kind:

```bash
jq -s 'group_by(.kind) | map({kind: .[0].kind, usd: (map(.cost_usd // 0) | add)})' ~/.ralph/chat/usage.jsonl
```

## Out of scope (v1)

- Mid-turn message injection.
- Logging tool calls and results into memory.
- Prompt-cache block tuning.
- Subagent reports.
