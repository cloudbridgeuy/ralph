# Threads Guide

A thread is a named side chat forked from the main chat. Use it to follow one topic without filling main's memory. This guide shows the tasks. For storage, ids and internals, see [ralph chat: Threads](ralph-chat.md#threads).

It describes the behaviour of the current release.

## Create and open a thread

A thread has no separate create command. Open it by name and ralph creates it on first use.

- In a terminal: `ralph chat --thread <name>`.
- In main's chat inside Rex: `/fork <name>`. It opens the thread in a new Rex block next to main.
- In a thread's chat inside Rex: `/fork <name>`. It opens a new thread forked from main, next to this one.
- In any chat (main or a thread) outside Rex: `/fork <name>` opens nothing. It prints a command. Run that command in another terminal.

A name has 1 to 32 lowercase letters, digits or dashes. It cannot start with a dash. `main` is not a valid name.

`/fork` creates nothing itself. The chat it starts creates the thread. Inside a thread, `/fork <name>` forks a sibling thread from main, not a nested thread. It opens as it does from main.

The thread starts from main's memory as it was at that moment. This point is the fork point. The thread does not see main messages that come later until you pull them.

## Return to main

Switch back in the same terminal, or quit the thread chat.

- Type `/main` or `/thread main` to switch this chat to main. See [Switching threads](#switching-threads).
- Press Ctrl-D on an empty prompt to quit. Main's chat, if open, stays in its own terminal or Rex block.
- Press Ctrl-C at the prompt to quit. It quits also when the draft has text. During a turn, Ctrl-C cancels the turn. It clears the draft only inside the queue editor.

## Switching threads

Type `/thread <name>` to move this chat to another thread in the same terminal. Type `/main` to go back to main. The chat quits and a new `ralph chat` starts on the target.

- `/thread <name>` takes one name. `main` means main. `/main` takes no arguments.
- Tab after `/thread ` completes the names of threads that are not merged or parked, plus `main` when you are in a thread.
- An unknown name creates the thread, like `ralph chat --thread <name>`.
- The switch is refused when you are already in the target, when the thread is merged or parked, or when the target chat is open elsewhere. Main counts as a target: after `/fork`, `/main` from the thread is refused while main's chat is open.
- The switch is refused while workers or watches run. The message names them. Stop them first.
- Type the switch during a turn and it waits in the queue. It runs when the turn ends.
- A refused switch, or a failed save, puts the switch line, then the entries queued after it, then any text already in the prompt, back in the prompt. Nothing is lost and the order stays. A typed switch that is refused comes back to the prompt the same way.
- A multi-line entry with extra words after the switch argument (`/thread auth\nfix` or `/main\nfix`) gets the usage refusal and is not sent; the refusal returns it to the prompt. A line break is whitespace, so `/thread\nauth` switches to `auth`.
- ralph saves unsent text to the prompt history first: queued lines and the draft in the prompt. If the save fails, the switch is cancelled. With a signal pending, ralph exits instead.
- Saved schedules stay on disk. They do not fire while you are away.
- The new chat does not keep `--name`. It titles itself by the thread name.
- If another chat takes the target after the check, or the new chat cannot start, you land at the shell. Start `ralph chat` again.

## List threads

```bash
ralph thread ls
```

Inside a chat, type `/threads` for the same list. It works in main's chat and in a thread's chat, takes no arguments and starts no turn. A turn in progress does not block it, but it waits in the queue like `/thread`. `/threads`, `/pull`, `/park` and `/merge` run alone in the queue, never joined with other lines, in typed order.

The list is a table with one row per thread: `NAME`, `STATE` (`open`, `parked 7d ago` or `merged 3d ago`), `FORK` (`#<base>`), `CREATED`, `ACTIVE` (the age of the thread's `log.jsonl` file; `-` when the thread has no messages yet), `MSGS`, `WORKERS` and `UNREAD`. Open threads come first, then parked, then merged, each group by name. `WORKERS` and `UNREAD` show a dim `-` when they are zero. In a terminal the table uses the same borders and colours as `/workers`; piped or with `NO_COLOR` set, it prints as plain markdown pipes. With colour on and a narrow screen, the table drops `ACTIVE`, then `UNREAD`, then `WORKERS`, and below that prints one block per thread. Plain output always has all eight columns. A time that does not parse shows a dim `-`. When there are none, the command prints `no threads in <store>`, with the store path that it read. It is safe while a thread chat is open.

## Pull main's new messages

```bash
ralph thread pull <name>
```

This moves the fork point of the thread to the end of main. The next `ralph chat --thread <name>` starts with the newer main messages. The thread's own messages do not change.

- Close the thread chat first. Pull refuses while the chat is open.
- In main's chat, type `/pull <name>` for the same result. It runs in main's chat only, makes no model call and does not leave the chat. In a thread's chat it refuses with `Run /main, then /pull <name>.`; the thread's own chat holds its claim, so it cannot pull itself.
- `/threads`, `/pull`, `/park` and `/merge` only take a line that has the shape of the command: `/threads` alone, one valid name, and for `/merge` the optional `--view` (or its alias `--rebase`). Any other line, such as `/merge requests are failing on CI`, goes to the model as a normal message, so a prose line or a user skill with that name still works. Typed with no argument, `/pull`, `/park` and `/merge` print their usage.
- Pull works on a parked thread.
- Pull prints `already at main's end` and changes nothing when the thread is up to date.

## Finish a thread: merge

Merge sends the result of a thread to main's inbox. Main reads it on its next turn. The thread becomes read-only.

Squash merge:

```bash
ralph thread merge <name>
```

- In main's chat, type `/merge <name>`. It first checks the cheap refusals (unknown or merged thread, running workers or saved records, an open chat: `merge_precheck`) and prints them with no question. Then it asks `[y/n]`, because it makes a paid call. Any answer other than `y`, `yes`, `n` or `no` asks again; `n` or `no` cancels with `merge cancelled.` and the queued lines go on as usual. Ctrl-C, a closed input or a signal aborts instead: it prints `merge aborted; the command stays in the prompt.` and puts `/merge <name>` and the rest of the queue back in the prompt, with nothing sent. After `y` it reads the running workers again, so a worker that started during the question is refused. It then prints `merging <name> (one paid Haiku call, up to 3 if the digest is refused)...` and runs in the foreground, so the prompt returns when it ends. It does not leave the chat. In a thread's chat it refuses with `Run /main, then /merge <name>.`; the other refusals of the shell command, such as a failed digest, still apply after the answer.
- It makes one paid call to the Haiku model, and up to 3 if the model's digest is refused. The model writes a digest from a 200-line view of the thread.
- The digest is at most 2048 bytes and cites thread message ids as `#<name>:<id>`. ralph tries at most 3 times in total; each retry tells the model why the last digest was refused. A refused merge, or a digest that fails 3 times, changes nothing. If posting the digest to main's inbox fails, the thread stays merged and the error names `thread.json`, which holds the digest.

View merge:

```bash
ralph thread merge <name> --view
```

- It makes no model call.
- It posts the thread's own memory view, at most 100 lines and 64 KB, with a header line. It is not a git rebase; to bring main into a thread, use `pull`. No git command runs and no branch changes.
- It refuses a thread with no messages. It refuses a block over 64 KB. Use the squash merge for these.
- In main's chat, type `/merge <name> --view` for the same result. It asks no confirmation and makes no model call. It runs in main's chat only and does not leave the chat. In a thread's chat it refuses with `Run /main, then /merge <name>.` The flag text is the `VIEW_FLAG` constant in `crates/core/src/chat_thread_cmd.rs`.

Use the squash merge when you want a short digest. Use `--view` when you want main to see the memory view and you want no cost. `--rebase` is the old name of `--view`. It still works and is hidden from help. The inbox status is `view`; entries posted before the rename show `rebased`.

Merge refuses while a worker of the thread runs or while the thread has saved watches or schedules. Close or stop them from the thread's chat first.

## After a merge

A merged thread is read-only. You cannot open it, drop it or merge it again. Its memory stays readable:

- `ralph chat --thread <name> history ...` reads it. See [history commands](ralph-chat.md#history-commands).
- Main's tools resolve ids such as `#<name>:5`. The digest in main's inbox cites these ids.

## Park and revive

```bash
ralph thread park <name>
ralph thread revive <name>
```

Park stops a thread from opening. Its saved watches and schedules wait. They keep their records. Revive allows the thread to open again. Overdue schedules then fire late.

- Park refuses an open chat, running workers, a merged thread and a thread that is already parked.
- Revive refuses a thread that is not parked.
- In main's chat, type `/park <name>` to park. It prints `parked thread <name>; `ralph thread revive <name>` opens it again`, the same text as the shell command (`parked_message`), makes no model call and does not leave the chat. It refuses in a thread's chat with `Run /main, then /park <name>.` There is no `/revive`; run `ralph thread revive <name>`.
- `history` still reads a parked thread. `pull`, `drop` and `merge` work on it.

## Drop a thread

```bash
ralph thread drop <name>
```

Drop deletes the thread folder. It cannot be undone. It also deletes the saved watches and schedules of the thread, and it prints their count.

Drop refuses an open chat and running workers. It refuses a merged thread, because a merged thread is read-only. It does not refuse saved watches or schedules.

## Example

```bash
ralph chat --thread auth       # work on the auth topic, then quit with Ctrl-D
ralph thread ls                # NAME | STATE | FORK | CREATED | ACTIVE | MSGS | ...
ralph thread pull auth         # take main's newer messages
ralph chat --thread auth       # continue with the new view
ralph thread merge auth        # Haiku digest (1-3 calls); goes to main's inbox
ralph chat history zoom auth:0 # the merged thread is still readable
```

## Refusals

Each message is quoted in its stable part. `<name>` is the thread. The checks are in `crates/core/src/chat_thread.rs` unless the table says otherwise.

| Refusal (start of message) | Cause | Fix |
| --- | --- | --- |
| `store <path> is in use` | `ralph chat --thread <name>` found a chat that has the store open (`crates/core/src/chat_memory/claim.rs`). | Quit the other chat. |
| `thread <name> is in use` | `pull`, `/pull`, `drop`, `park`, `/park`, `revive`, `merge`, `/merge` or `/fork` found an open chat of the thread (`crates/ralph/src/chat/thread.rs`). | Quit the thread chat. |
| `thread <name> has running workers` | `drop`, `park`, `/park`, `merge` or `/merge` found a running worker (`guard_check`). | Close the workers from the thread's chat. |
| `thread <name> has N saved watch` | `merge` found a saved watch or schedule (`guard_check`). | Stop it from the thread's chat. |
| `thread <name> is merged into main` | You opened, forked, dropped, parked, revived, pulled or merged a merged thread (`merged_check`). | None. Read it with `history`. |
| `thread <name> is parked` | You opened, forked or parked a parked thread (`open_check`). | Run `ralph thread revive <name>`. |
| `thread <name> is not parked` | `revive` found a thread that is not parked. | None. |
| `/pull runs in main's chat.`, `/park runs in main's chat.` or `/merge runs in main's chat.` | You typed `/pull`, `/park` or `/merge` in a thread's chat (`op_refusal` in `crates/core/src/chat_thread_cmd.rs`). | Run `/main`, then the same command with the name. |
| `no thread <name>` | The name is unknown. | Run `ralph thread ls`. |
| `#N is after this thread's fork point` | A thread read a main id newer than its fork point. | Run `ralph thread pull <name>`, then reopen the chat. |

See also: [CLI reference](../cli-reference.md), [ralph chat](ralph-chat.md).
