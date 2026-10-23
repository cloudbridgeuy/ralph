//! `/help`: the chat commands, keys and status bar legend, printed by ralph
//! without a model turn.

use crate::chat::OLDER_VIEW_STEP;
use crate::chat_thread::{ResetReason, CACHE_TTL_SECS};
use crate::status_mode::Mode;
use crate::status_segments::Kind;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Help,
    Inbox,
    Watch,
    Diff,
}

impl Command {
    pub const ALL: [Command; 4] = [Command::Help, Command::Inbox, Command::Watch, Command::Diff];

    pub fn name(self) -> &'static str {
        match self {
            Command::Help => "/help",
            Command::Inbox => "/inbox",
            Command::Watch => "/watch",
            Command::Diff => "/diff",
        }
    }

    /// The text after the command word, or `None` when `line` is another
    /// message (`/difffoo`).
    pub fn args(self, line: &str) -> Option<&str> {
        line.strip_prefix(self.name())
            .filter(|args| args.is_empty() || args.starts_with(char::is_whitespace))
    }

    fn usage(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Command::Help => &[("/help", "print this help; starts no turn")],
            Command::Inbox => &[
                (
                    "/inbox",
                    "send every unread worker report to the model (starts a turn)",
                ),
                (
                    "/inbox next | N",
                    "send only the oldest report, or report N",
                ),
                ("/inbox list", "list the unread reports; starts no turn"),
                ("/inbox peek N", "print report N; removes nothing"),
                ("/inbox drop N", "remove report N without sending it"),
            ],
            Command::Watch => &[
                (
                    "/watch start [--persistent] <cmd> -- <follow-up>",
                    "run <cmd>; its exit posts a watch report",
                ),
                ("/watch list", "list the active watches and their time left"),
                (
                    "/watch stop <id>",
                    "kill the watch (also removes a persistent one)",
                ),
            ],
            Command::Diff => &[
                ("/diff [last N | ago N]", "changes of the last chat turn(s)"),
                (
                    "/diff worker <id> [last N | ago N]",
                    "a worker's changes since spawn, or of its turns",
                ),
                ("/diff now", "the working tree against the index"),
                ("/diff ... <git diff args>", "extra words go to git diff"),
            ],
        }
    }
}

const CELLS: &[(&str, &str)] = &[
    (
        "!<command>",
        "run the rest of the message as one !shell cell, then send it with its output",
    ),
    (
        "```!<runtime> fence",
        "run the block (for example ```!zsh) as a cell before the message goes out",
    ),
];

pub struct KeyHelp {
    /// Comma-separated key labels, such as `Alt-Enter, Ctrl-J`.
    pub keys: &'static str,
    pub effect: &'static str,
}

pub const PROMPT_KEYS: &[KeyHelp] = &[
    KeyHelp {
        keys: "Enter",
        effect: "send the message",
    },
    KeyHelp {
        keys: "Alt-Enter, Ctrl-J",
        effect: "insert a newline",
    },
    KeyHelp {
        keys: "Ctrl-G",
        effect: "edit the draft in [chat] editor, $VISUAL, $EDITOR or vi",
    },
    KeyHelp {
        keys: "Ctrl-V",
        effect: "paste a clipboard image as [Image #N: path]",
    },
    KeyHelp {
        keys: "Esc Esc",
        effect: "clear the whole draft (emacs mode)",
    },
    KeyHelp {
        keys: "Ctrl-Z, Ctrl-O",
        effect: "undo, redo",
    },
    KeyHelp {
        keys: "Up, Down, Ctrl-R",
        effect: "browse and search the message history",
    },
    KeyHelp {
        keys: "Ctrl-C, Ctrl-D",
        effect: "quit ralph chat",
    },
];

pub const VI_KEYS: &[KeyHelp] = &[
    KeyHelp {
        keys: "Esc",
        effect: "leave insert mode; the prompt changes from >>> to :::",
    },
    KeyHelp {
        keys: "Ctrl-U",
        effect: "clear the whole draft (insert and normal mode)",
    },
];

pub const TURN_KEYS: &[KeyHelp] = &[
    KeyHelp {
        keys: "Ctrl-C",
        effect: "cancel the turn; the message comes back as the draft",
    },
    KeyHelp {
        keys: "Ctrl-Q",
        effect: "open the queue editor; the turn keeps running, its output is held",
    },
];

pub const QUEUE_KEYS: &[KeyHelp] = &[
    KeyHelp {
        keys: "Enter",
        effect: "add the text as a queue entry and close the editor",
    },
    KeyHelp {
        keys: "Ctrl-Q",
        effect: "back out: add the draft and hold the queue for the next prompt",
    },
    KeyHelp {
        keys: "Ctrl-D",
        effect: "close the editor and add nothing",
    },
    KeyHelp {
        keys: "Ctrl-C",
        effect: "cancel the turn",
    },
];

/// Status bar segments in the order they are explained.
pub const SEGMENTS: [Kind; 8] = [
    Kind::Mode(Mode::Idle),
    Kind::Queue(Mode::Idle),
    Kind::Mail(Mode::Idle),
    Kind::Store,
    Kind::Workers,
    Kind::Watches,
    Kind::Thread(Mode::Idle),
    Kind::Cost(Mode::Idle),
];

/// Modes in the order `status_mode::mode` tries them.
pub const MODES: [Mode; 4] = [Mode::Nap, Mode::Inbox, Mode::Work, Mode::Idle];

pub const RESETS: [ResetReason; 3] = [
    ResetReason::Tokens,
    ResetReason::Turns,
    ResetReason::Restart,
];

fn segment_help(kind: Kind) -> (&'static str, &'static str) {
    match kind {
        Kind::Mode(_) => ("mode block", "NAP, INBOX, WORK or IDLE (below)"),
        Kind::Queue(_) => (
            "turn running · QUEUED N",
            "only in the queue editor: turn state and queued entries; HELD N after a back-out; short forms running · QN, ▶ QN, done · QN, ✓ QN",
        ),
        Kind::Mail(_) => ("📬 N unread", "worker reports waiting; /inbox sends them"),
        Kind::Store => ("store", "the chat store path (last two parts)"),
        Kind::Workers => ("workers", "each worker and its state"),
        Kind::Watches => ("watch <id> <left>", "running watches and their time left"),
        Kind::Thread(_) => ("thread cell", "the live claude process (below)"),
        Kind::Cost(_) => (
            "$4.12 (+0.31)",
            "session total over the claude processes since this chat started; the delta is the last turn's cost; nap cost is not included",
        ),
    }
}

fn mode_help(mode: Mode) -> &'static str {
    match mode {
        Mode::Nap => "a nap compresses the memory tree",
        Mode::Inbox => "worker reports are unread",
        Mode::Work => "a worker is running, busy or waits for permission, or a watch exists",
        Mode::Idle => "none of the above",
    }
}

fn reset_help(reason: ResetReason) -> &'static str {
    match reason {
        ResetReason::Tokens => {
            "the context reached [chat] reset_tokens, or claude's context was full"
        }
        ResetReason::Turns => "the process reached [chat] reset_turns",
        ResetReason::Restart => {
            "the process died, or a turn failed or was cancelled, so claude restarted"
        }
    }
}

fn thread_parts() -> Vec<(String, String)> {
    vec![
        (
            "T12/30".into(),
            "turns on the live claude process over [chat] reset_turns; 0 with no live process"
                .into(),
        ),
        (
            "ctx 62%".into(),
            "the last turn's context tokens over [chat] reset_tokens; – when unknown".into(),
        ),
        (
            "cache 41m".into(),
            format!(
                "estimated minutes left of the prompt cache ({} min TTL from the last reply); cold when none",
                CACHE_TTL_SECS / 60
            ),
        ),
        (
            "!".into(),
            "the last turn wrote at least half of its input to the cache (a cache rebuild)".into(),
        ),
        (
            "shift ~9".into(),
            format!(
                "about 9 entries until the older view moves to the next {OLDER_VIEW_STEP}-entry cut"
            ),
        ),
    ]
}

fn rows(out: &mut String, rows: &[(String, String)]) {
    let width = rows.iter().map(|(left, _)| left.width()).max().unwrap_or(0);
    for (left, right) in rows {
        let pad = width - left.width();
        out.push_str(&format!("  {left}{}  {right}\n", " ".repeat(pad)));
    }
}

fn section(out: &mut String, title: &str, lines: Vec<(String, String)>) {
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(title);
    out.push('\n');
    rows(out, &lines);
}

fn pairs(table: &[(&str, &str)]) -> Vec<(String, String)> {
    table
        .iter()
        .map(|(left, right)| (left.to_string(), right.to_string()))
        .collect()
}

fn keys(table: &[KeyHelp]) -> Vec<(String, String)> {
    table
        .iter()
        .map(|key| (key.keys.to_string(), key.effect.to_string()))
        .collect()
}

pub fn help_text() -> String {
    let mut out = String::new();
    let commands = Command::ALL
        .iter()
        .flat_map(|command| pairs(command.usage()))
        .collect();
    section(&mut out, "Commands (ralph answers these itself)", commands);
    section(&mut out, "Cells", pairs(CELLS));
    section(&mut out, "Keys at the prompt", keys(PROMPT_KEYS));
    section(
        &mut out,
        "Keys with [chat] edit_mode = \"vi\"",
        keys(VI_KEYS),
    );
    section(&mut out, "Keys during a turn", keys(TURN_KEYS));
    section(&mut out, "Keys in the queue editor", keys(QUEUE_KEYS));
    let segments = SEGMENTS
        .iter()
        .map(|kind| {
            let (name, text) = segment_help(*kind);
            (name.to_string(), text.to_string())
        })
        .collect();
    section(&mut out, "Status bar, left to right", segments);
    let modes = MODES
        .iter()
        .map(|mode| (mode.label().to_string(), mode_help(*mode).to_string()))
        .collect();
    section(&mut out, "Mode block (first match wins)", modes);
    let mut thread = thread_parts();
    thread.extend(RESETS.iter().map(|reason| {
        (
            format!("↻ {}", reason.label()),
            format!("first turn after a reset: {}", reset_help(*reason)),
        )
    }));
    section(
        &mut out,
        "Thread cell: T12/30 · ctx 62% · cache 41m! · shift ~9 (shorter forms drop parts)",
        thread,
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_status::{route_input, EmptySubmit, Route, Status, WatchState, WorkerState};
    use crate::chat_thread::{SessionCost, ThreadCell};
    use crate::status_mode::mode;
    use crate::status_queue::QueueCell;
    use crate::status_segments::{segments, Context};
    use std::mem::discriminant;

    fn full_status() -> Status {
        Status {
            unread: 2,
            workers: vec![WorkerState {
                id: "w1".into(),
                state: "running".into(),
            }],
            watches: vec![WatchState {
                id: "watch:w1".into(),
                left: Some("5m".into()),
            }],
            thread: Some(ThreadCell {
                turn: 12,
                max_turns: 30,
                ctx_percent: Some(62),
                cache_minutes: Some(41),
                write_heavy: true,
                reset: None,
                shift: Some(9),
                cost: Some(SessionCost {
                    total: 4.12,
                    delta: Some(0.31),
                }),
            }),
            napping: false,
            queue: Some(QueueCell {
                running: true,
                count: 1,
                held: false,
            }),
        }
    }

    #[test]
    fn every_command_routes_locally_and_has_help() {
        let text = help_text();
        for command in Command::ALL {
            let route = route_input(command.name(), 0, &EmptySubmit::Inbox);
            assert!(
                !matches!(route, Route::Send(_)),
                "{} is not routed",
                command.name()
            );
            assert!(
                text.contains(command.name()),
                "{} has no help",
                command.name()
            );
        }
    }

    #[test]
    fn help_routes_to_help() {
        assert_eq!(route_input("/help", 0, &EmptySubmit::Inbox), Route::Help);
        assert_eq!(
            route_input("/helpfoo", 0, &EmptySubmit::Inbox),
            Route::Send("/helpfoo".into())
        );
    }

    #[test]
    fn every_built_segment_has_a_legend() {
        let built = segments(
            &full_status(),
            &Context {
                store: "a/b".into(),
            },
        );
        for segment in built {
            assert!(
                SEGMENTS
                    .iter()
                    .any(|kind| discriminant(kind) == discriminant(&segment.kind)),
                "{:?} has no /help legend",
                segment.kind
            );
        }
    }

    #[test]
    fn every_mode_has_a_legend() {
        let statuses = [
            Status {
                napping: true,
                ..full_status()
            },
            full_status(),
            Status {
                unread: 0,
                ..full_status()
            },
            Status::default(),
        ];
        let text = help_text();
        for status in statuses {
            let current = mode(&status);
            assert!(MODES.contains(&current), "{current:?} has no /help legend");
            assert!(text.contains(current.label()));
        }
    }

    #[test]
    fn thread_legend_explains_each_part_and_reset_mark() {
        let text = help_text();
        for part in [
            "T12/30",
            "ctx 62%",
            "cache 41m",
            "shift ~9",
            "↻ tokens",
            "↻ turns",
            "↻ restart",
        ] {
            assert!(text.contains(part), "{part} is missing");
        }
        assert!(text.contains("(a cache rebuild)"));
    }

    #[test]
    fn help_text_has_every_key() {
        let text = help_text();
        for table in [PROMPT_KEYS, VI_KEYS, TURN_KEYS, QUEUE_KEYS] {
            for key in table {
                assert!(text.contains(key.keys));
            }
        }
    }
}
