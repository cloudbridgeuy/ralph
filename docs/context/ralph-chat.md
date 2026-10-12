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

- `crates/core/src/chat_memory/`: pure chat memory, split by concern. `mod.rs` holds the types (`Entry`, `Node`, `Tree`) and re-exports; `log.rs` parses and encodes the log and tree files; `tree.rs` has `cover`, `pending`, `frozen_block`, `tail_lines`, `zoom`, `recall`; `tools.rs` has the MCP tool schemas, `handle_request` and `finish_call`; `nap.rs` has the nap and retry prompts and `check_summary`; `usage.rs` has the per-call usage records.
- `crates/core/src/chat.rs`: `VIEW_LINES`, `FROZEN_WINDOW`, `SYSTEM_PROMPT`, `turn_system_prompt`, `turn_prompt`, `build_mcp_args`.
- `crates/core/src/chat_status.rs`: status line, footer hints, input routing, worker state parsing, memory size.
- `crates/core/src/chat_skills.rs`: `resolve_skills`, `plan_links`, `expand_slash_command`, `plugin_manifest`, `build_plugin_args`.
- `crates/ralph/src/chat.rs`: turn loop shell, reedline input, nap spawn.
- `crates/ralph/src/chat_memory.rs`: `Store` (files, LOCK, append, nap loop).
- `crates/ralph/src/chat_mcp.rs`: MCP stdio loop.
- `crates/ralph/src/invoke.rs`: `build_chat_command`, `build_memory_command`.
- `crates/ralph/src/cli/mod.rs` and `main.rs`: arguments, default dispatch, `chat-mcp` and `chat-nap` handlers.

## Turn loop

1. `Store::load_settled` takes the `LOCK` (blocking until any running nap finishes), then reads log and tree.
2. `frozen_block(entries, tree, VIEW_LINES, FROZEN_WINDOW, pinned)` and `tail_lines` build the memory view in two parts. `VIEW_LINES` is 200 lines total; `FROZEN_WINDOW` is 32 entries by default. The frozen part is the view of the first `entries / window * window` entries with budget `200 - window`; the chat process pins it and rebuilds only when the entry count crosses a multiple of the window, even if nap summaries land mid-window, so it stays byte-identical for the whole window (default 32 entries) and the prompt cache hits it. The tail is one line per entry after that point. The window comes from `--frozen-window <N>`, else `[chat] frozen_window` in the ralph config, else 32; valid values are 1 to 199, anything else fails chat start with an error.
3. The user message, or one `worker` entry per report for a worker turn, is appended to the log.
4. `build_chat_command` runs a fresh `claude --verbose --permission-mode bypassPermissions` with isolation args, `--append-system-prompt <SYSTEM_PROMPT + frozen block>` (`turn_system_prompt`; just `SYSTEM_PROMPT` while the frozen part is empty), the MCP args, `--output-format stream-json`, and `-p <prompt>`. There is no `--resume` and no `--agent`. The prompt is the tail lines, a blank line, `New message:`, then the message. Only tail lines precede the message; no task text follows it.
5. Output goes through the spinner subprocess runner and `StreamProcessor`, as in `ralph ask`.
6. On exit code 0 with response text, the reply is appended verbatim and a detached `ralph chat-nap` is spawned (stdio null; spawn failure only warns). Otherwise a warning `chat turn failed (exit N)` prints and the loop continues. The user message stays in the log without a reply.
7. An interrupted turn ends the chat. Other subprocess errors abort with an error.

`SYSTEM_PROMPT` tells the model that the message starts with the compressed chat, that `#<id>` lines are summaries, to use `zoom` and `recall` before guessing or asking, that `[worker ...]` messages come from workers, not the person, and never to claim manual saving. On a worker message the model may `reply` only with an answer already settled in the chat (a decision the person made, approved scope, a step in an agreed plan); merges, pushes, PRs, Linear status changes, new scope and anything public go to the person, and every worker reply is listed in its next message to the person. `spawn_worker` and `close_worker` are only for what the person asked in their latest message, and `abandon` only when the person said to drop the work. The chat MCP server enforces this with the pure `worker::command_allowed`: `spawn_worker` and `close_worker` need the newest log entry to be from the person, `reply` accepts a person or worker entry, and anything else is refused. Its delegation rules keep turns short: the model reads inline only to scope (at most 3 tool calls per question), sends multi-file reads, broad searches and test runs to `evidence-collector`, all code edits to `implementer`, and fixes to `reviser`, and every dispatch prompt states goal, question, known facts, owned files, output cap, stop condition and tool budget, "Report not-found, do not widen scope.", and "You are a subagent. Don't run mem." After 2 failed repair rounds it diagnoses; after a 3rd it waits for the person. It also must lead with the answer and end with one next step, show a 3-5 line plan when a request is ambiguous, treat a worker "done" as unverified until it checks the diff, tests, git and Linear, cite the `#<id>` behind facts from older chat, and say it has no erase tool when asked to forget.

## Worker inbox

Worker reports never interrupt the draft. A watcher thread polls every 500 ms and repaints the prompt in place (reedline `RepaintSignal`), keeping the buffer and cursor.

- Status line: the first prompt line above `>>> `, `📬 <unread> unread · workers <id> <state>, ... · mem <msgs> msgs ~<k> tok · cache <read> read <write> write <hit>% hit $<cost>`; the cache part comes from the last turn in `usage.jsonl` and is hidden before any turn. It sheds detail as the terminal narrows (states, then names to a count, then labels) and never exceeds the width. Dim when stdout is a tty and `NO_COLOR` is unset.
- Unread is the count of pending inbox files. Workers are tmux sessions named `ralph-<project>-<id>` (`<project>` = the git top-level folder name, letters, digits and dashes only, else `repo`), so the same id in two projects does not collide. Names are cosmetic: at spawn ralph tags each session with the tmux user options `@ralph_store` = the absolute store path and `@ralph_id` = the bare id, and the status line reads the id from `@ralph_id`. Only sessions tagged with this chat's store and an id are listed (sessions without both, including ones started before `@ralph_id`, are ignored; missing tmux or no server means none); opencode workers also get `@ralph_harness` = `opencode`. A claude worker's state is the status of its latest report, else `running`. For opencode workers ralph runs `opencode api session.active` (one call for all workers, every 5 s and after each turn or inbox read, only while an opencode worker runs): `busy` while `ses_ralph<id>` is in it, else the latest report status, else `idle`; if the call fails they fall back to the claude rule. Memory is the log entry count and the view size at about 4 chars per token, refreshed every 5 s and after each turn or inbox read.
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

`spawn_worker` takes `id`, `task`, optional `harness` (`claude` default, or `opencode`) and optional `model` (`provider/model`, opencode only, default `openai/gpt-5.6-sol`). `worker::parse_command` refuses an unknown harness, a `model` for claude, and a model without a provider and name. Both harnesses run in tmux session `ralph-<project>-<id>` in `.worktrees/<id>` on branch `worker/<id>` through a login `zsh -lic`. Claude gets `--mcp-config <store>/workers/<id>.json` and the worker rules via `--append-system-prompt`. opencode runs its TUI on the shared background service (no `--standalone`) as `opencode --auto --session ses_ralph<id> --prompt <worker rules + task>`; opencode creates that session on first spawn and resumes it on a later spawn with the same id. `opencode api session.active` lists every running session on the service, keyed by session id, whatever folder it is called from; ralph writes `<worktree>/opencode.json` with `model` and a local `ralph-worker` MCP server, and adds `/opencode.json` next to `/.worktrees/` in the shared `.git/info/exclude`. `--auto` approves every tool call the worker makes. Each worker signals ready by creating `<store>/ready/<id>`, which spawn deletes first: Claude through a `SessionStart` hook (matcher `startup`) in `<store>/workers/<id>.settings.json` passed with `--settings`; opencode through `<worktree>/.opencode/plugins/ralph-ready.ts` (also excluded), which writes the file on the first `session.created` or `session.inbox.enqueued` event whose `data.sessionID` is `ses_ralph<id>` and then stops listening. opencode's event bus is shared by the whole service, hence the session filter; the plugin stays loaded on the service after the worker exits, and rewriting the file on a respawn reloads it. Spawn polls up to 20 s, reading the pane only to accept the folder-trust prompt; a timeout is not an error ("not confirmed yet" plus the tmux session), but a gone tmux session or a trust prompt still showing is.

Ownership: `spawn_worker`, `reply` and `close_worker` look up the worker's session by name (`ralph-<project>-<id>`, or the older `ralph-<id>`) and compare its `@ralph_store` tag with this chat's store in the pure `worker::session_owner`. A session tagged with another store, or untagged, belongs to another chat: spawn, reply and close refuse it.

`close_worker` takes `id` and optional `abandon` (default false). Like `spawn_worker`, it needs the newest log entry to be from the person. The shell collects the facts (owner, worktree dirty per `git status --porcelain`, `git rev-list --count <base>..worker/<id>`, opencode = the session's harness tag or `<worktree>/opencode.json`), and the pure `worker::close_plan` decides. The base comes from the pure `worker::base_branch`, first non-empty wins: `[chat] worker_base` in the ralph config (read at each close), the branch checked out in the repo at spawn (spawn records it once in git config `branch.worker/<id>.ralphBase`, which `git branch -D` removes with the branch; a detached HEAD records nothing), the branch behind `refs/remotes/origin/HEAD`, then `main`. Workers spawned before this have no recorded base and fall through to `origin/HEAD`. A missing config file is normal; a config that fails to read or parse skips `worker_base`, and the close result (success or refusal) ends with a warning naming the config path, the error and the base used instead. It refuses a foreign worker always; it refuses unmerged commits or a dirty worktree unless `abandon` is set; and it refuses when no session, worktree or branch is left. Steps, skipping what is already gone: (a) opencode only: `opencode api session.remove --param sessionID=ses_ralph<id>`, where `HTTP 404` means already gone; (b) `tmux kill-session`; (c) `git worktree remove` (`--force` when dirty and abandoned); (d) `git branch -D worker/<id>`; (e) `git worktree prune`; (f) opencode only: `opencode api debug.location.evict --param location[directory]=<worktree>`; then remove `<store>/ready/<id>`. A failed step stops the close and names the step. Known limit (GUZ-491): evict does not stop the worker's plugin; its timers and child processes keep running in the shared `opencode serve --service` until it restarts.

At chat start, ids in `<store>/ready/` with no tmux session for this store print one hint line (`chat_status::orphan_hint`) that suggests `close_worker`. Chat never deletes them by itself.

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
- Isolation: `--setting-sources project,local` plus a `claudeMdExcludes` entry for `~/.claude/CLAUDE.md` when the home directory is known. Project and local settings stay active; this is context isolation, not a sandbox.
- User agents in `~/.claude/agents` do not load, so `build_agents_args` passes the bundled `SUBAGENTS` (`evidence-collector` haiku, `implementer` sonnet, `reviser` opus) as `--agents <json>`. These copies may drift from `~/.claude/agents`.
- Turns run with `bypassPermissions`.

## Usage

Every completed Claude call (each turn, including failed exits but not interrupted ones, and each nap attempt) appends one line to `usage.jsonl`: `{at, kind, trigger, input_tokens, cache_creation_input_tokens, cache_read_input_tokens, output_tokens, cost_usd, duration_ms}`, with `kind` `turn` or `nap`, and `trigger` (turn only) `user` or `worker`. Turn lines also carry `view_bytes`, `view_lines`, `view_head_hash` (FNV-1a 64 of the first 1024 bytes of the rendered view) and `view_hash` (of the whole view), to trace prompt-cache misses. Calls without usage data append nothing. Appends do not take `LOCK`. Total cost per kind:

```bash
jq -s 'group_by(.kind) | map({kind: .[0].kind, usd: (map(.cost_usd // 0) | add)})' ~/.ralph/chat/usage.jsonl
```

## Out of scope (v1)

- Mid-turn message injection.
- Logging tool calls and results into memory.
- Subagent reports.
