# Threads Guide

A thread is a named side chat forked from the main chat. Use it to follow one topic without filling main's memory. This guide shows the tasks. For storage, ids and internals, see [ralph chat: Threads](ralph-chat.md#threads).

It describes the behaviour of the current release.

## Create and open a thread

A thread has no separate create command. Open it by name and ralph creates it on first use.

- In a terminal: `ralph chat --thread <name>`.
- In main's chat inside Rex: `/fork <name>`. It opens the thread in a new Rex block next to main.
- In main's chat outside Rex: `/fork <name>` opens nothing. It prints a command. Run that command in another terminal.

A name has 1 to 32 lowercase letters, digits or dashes. It cannot start with a dash. `main` is not a valid name.

`/fork` creates nothing itself. The chat it starts creates the thread. `/fork` does not work inside a thread. Run it in main's chat.

The thread starts from main's memory as it was at that moment. This point is the fork point. The thread does not see main messages that come later until you pull them.

## Return to main

Quit the thread chat. Main's chat stays open in its own terminal or Rex block. There is no command to switch between chats.

- Press Ctrl-D on an empty prompt.
- Press Ctrl-C. It quits also when the draft has text.

## List threads

```bash
ralph thread ls
```

Each line shows the name, the fork point, the message count and the creation time. A line also shows `merged` or `parked` with a time, the running workers and the unread reports. The last three appear only when they are not zero. The command prints `no threads` when there are none. It is safe while a thread chat is open.

## Pull main's new messages

```bash
ralph thread pull <name>
```

This moves the fork point of the thread to the end of main. The next `ralph chat --thread <name>` starts with the newer main messages. The thread's own messages do not change.

- Close the thread chat first. Pull refuses while the chat is open.
- Pull works on a parked thread.
- Pull prints `already at main's end` and changes nothing when the thread is up to date.

## Finish a thread: merge

Merge sends the result of a thread to main's inbox. Main reads it on its next turn. The thread becomes read-only.

Squash merge:

```bash
ralph thread merge <name>
```

- It makes one paid call to the Haiku model. The model writes a digest from a 200-line view of the thread.
- The digest is at most 2048 bytes and cites thread message ids as `#<name>:<id>`. ralph asks again up to 3 times when the digest is not valid. A failed merge changes nothing.

Rebase merge:

```bash
ralph thread merge <name> --rebase
```

- It makes no model call.
- It posts the thread's own memory view, at most 100 lines and 64 KB, with a header line. It is not a git rebase. No git command runs and no branch changes.
- It refuses a thread with no messages. It refuses a block over 64 KB. Use the squash merge for these.

Use the squash merge when you want a short digest. Use `--rebase` when you want main to see the memory view and you want no cost.

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
ralph thread ls                # auth  forked at #42  6 messages ...
ralph thread pull auth         # take main's newer messages
ralph chat --thread auth       # continue with the new view
ralph thread merge auth        # one Haiku call; digest goes to main's inbox
ralph chat history zoom auth:0 # the merged thread is still readable
```

## Refusals

Each message is quoted in its stable part. `<name>` is the thread. The checks are in `crates/core/src/chat_thread.rs` unless the table says otherwise.

| Refusal (start of message) | Cause | Fix |
| --- | --- | --- |
| `store <path> is in use by pid` | `ralph chat --thread <name>` found a chat that has the store open (`crates/core/src/chat_memory/claim.rs`). | Quit the other chat. |
| `thread <name> is in use by pid` | `pull`, `drop`, `park`, `merge` or `/fork` found an open chat of the thread (`crates/ralph/src/chat/thread.rs`). | Quit the thread chat. |
| `thread <name> has running workers` | `drop`, `park` or `merge` found a running worker (`guard_check`). | Close the workers from the thread's chat. |
| `thread <name> has N saved watch` | `merge` found a saved watch or schedule (`guard_check`). | Stop it from the thread's chat. |
| `thread <name> is merged into main` | You opened, dropped, parked, pulled or merged a merged thread (`merged_check`). | None. Read it with `history`. |
| `thread <name> is parked` | You opened a parked thread (`open_check`). | Run `ralph thread revive <name>`. |
| `thread <name> is not parked` | `revive` found a thread that is not parked. | None. |
| `no thread <name>` | The name is unknown. | Run `ralph thread ls`. |
| `#N is after this thread's fork point` | A thread read a main id newer than its fork point. | Run `ralph thread pull <name>`, then reopen the chat. |

See also: [CLI reference](../cli-reference.md), [ralph chat](ralph-chat.md).
