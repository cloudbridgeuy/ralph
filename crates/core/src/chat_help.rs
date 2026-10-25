//! `/help`: the chat commands, keys and status bar legend, printed by ralph
//! without a model turn.

use crate::chat::OLDER_VIEW_STEP;
use crate::chat_thread::{thread_variants, ResetReason, ThreadCell, CACHE_TTL_SECS};
use crate::status_mode::Mode;
use crate::status_segments::Kind;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Help,
    Inbox,
    Watch,
    Diff,
    Workers,
    Hold,
}

impl Command {
    pub const ALL: [Command; 6] = [
        Command::Help,
        Command::Inbox,
        Command::Watch,
        Command::Diff,
        Command::Workers,
        Command::Hold,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Command::Help => "/help",
            Command::Inbox => "/inbox",
            Command::Watch => "/watch",
            Command::Diff => "/diff",
            Command::Workers => "/workers",
            Command::Hold => "/hold",
        }
    }

    /// The text after the command word, or `None` when `line` is another
    /// message (`/difffoo`). `/help` takes no arguments, so `/help me` is a
    /// message too.
    pub fn args(self, line: &str) -> Option<&str> {
        let args = line.strip_prefix(self.name())?;
        let takes_args = self != Command::Help;
        (args.is_empty() || takes_args && args.starts_with(char::is_whitespace)).then_some(args)
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
            Command::Workers => &[
                (
                    "/workers",
                    "list live workers: id, harness, state, report (the worker's status and its age when newer than its last report, else the last report), head, +/-, behind; BEHIND counts against local origin/<base>, stale until a fetch; no turn",
                ),
                (
                    "/workers <id>",
                    "one worker in detail with its status and last 5 reports",
                ),
            ],
            Command::Hold => &[
                (
                    "/hold [<worker-id> | watch:<name>]",
                    "wait with no prompt; the first matching inbox report starts a turn, once; Ctrl-Q or Ctrl-D leaves",
                ),
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
        keys: "Suggested reply",
        effect: "the prompt can start filled with a reply the model suggests; Enter sends it; Ctrl-U clears it (Esc Esc too in emacs mode)",
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
        keys: "Ctrl-C",
        effect: "quit ralph chat, also with text in the draft",
    },
    KeyHelp {
        keys: "Ctrl-D",
        effect: "quit ralph chat on an empty draft; else delete the character under the cursor",
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
    KeyHelp {
        keys: "Alt-Enter, Ctrl-J, Ctrl-V, Ctrl-O",
        effect: "insert mode only; normal mode adds just Ctrl-G and Ctrl-U",
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
    KeyHelp {
        keys: "Any text",
        effect: "open the queue editor seeded with the text; other keys are dropped",
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
        effect: "on an empty draft close the editor and add nothing; else delete the character under the cursor",
    },
    KeyHelp {
        keys: "Ctrl-C",
        effect: "on a non-empty draft clear it, the turn keeps running; on an empty draft cancel the turn and hold the queue",
    },
    KeyHelp {
        keys: "Ctrl-V, Ctrl-G",
        effect: "off here: no image paste and no external editor",
    },
];

/// Status bar segments in the order they are explained.
pub const SEGMENTS: [Kind; 9] = [
    Kind::Mode(Mode::Idle),
    Kind::Hold(Mode::Idle),
    Kind::Queue(Mode::Idle),
    Kind::Mail(Mode::Idle),
    Kind::Store,
    Kind::Workers,
    Kind::Watches,
    Kind::Thread(Mode::Idle),
    Kind::Cost(Mode::Idle),
];

/// Modes in the order `status_mode::mode` tries them.
pub const MODES: [Mode; 5] = [Mode::Hold, Mode::Nap, Mode::Inbox, Mode::Work, Mode::Idle];

pub const RESETS: [ResetReason; 3] = [
    ResetReason::Tokens,
    ResetReason::Turns,
    ResetReason::Restart,
];

fn segment_help(kind: Kind) -> (&'static str, &'static str) {
    match kind {
        Kind::Mode(_) => ("mode block", "HOLD, NAP, INBOX, WORK or IDLE (below)"),
        Kind::Hold(_) => (
            "from <source>",
            "only during /hold: the source it waits for, or any",
        ),
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
        Mode::Hold => "/hold waits for a matching inbox report",
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

/// The cell the thread legend shows, rendered by the status bar formatter.
const EXAMPLE_THREAD: ThreadCell = ThreadCell {
    turn: 12,
    max_turns: 30,
    ctx_percent: Some(62),
    cache_minutes: Some(41),
    write_heavy: true,
    reset: None,
    shift: Some(9),
    cost: None,
};

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
            "shift ~9 msg".into(),
            format!(
                "about 9 entries until the older view moves to the next {OLDER_VIEW_STEP}-entry cut"
            ),
        ),
    ]
}

const INDENT: &str = "  ";
const GAP: &str = "  ";
const STACKED_INDENT: &str = "      ";
/// A section narrower than this for its text puts the text under the label.
const MIN_TEXT_WIDTH: usize = 30;

/// Splits a word wider than `width` into pieces that fit.
fn pieces(word: &str, width: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut piece = String::new();
    for c in word.chars() {
        if !piece.is_empty() && piece.width() + c.width().unwrap_or(0) > width {
            pieces.push(std::mem::take(&mut piece));
        }
        piece.push(c);
    }
    pieces.push(piece);
    pieces
}

/// Word-wraps `text` to lines of at most `width` display columns.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace().flat_map(|word| pieces(word, width)) {
        if !line.is_empty() && line.width() + 1 + word.width() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(&word);
    }
    lines.push(line);
    lines
}

fn indented(out: &mut String, first: &str, rest: &str, lines: Vec<String>) {
    for (i, line) in lines.iter().enumerate() {
        out.push_str(if i == 0 { first } else { rest });
        out.push_str(line);
        out.push('\n');
    }
}

fn rows(out: &mut String, rows: &[(String, String)], width: usize) {
    let label = rows.iter().map(|(left, _)| left.width()).max().unwrap_or(0);
    let column = INDENT.len() + label + GAP.len();
    let text_width = width.saturating_sub(column);
    for (left, right) in rows {
        if text_width >= MIN_TEXT_WIDTH {
            let first = format!("{INDENT}{left}{}{GAP}", " ".repeat(label - left.width()));
            indented(out, &first, &" ".repeat(column), wrap(right, text_width));
        } else {
            let label_width = width.saturating_sub(INDENT.len());
            indented(out, INDENT, INDENT, wrap(left, label_width));
            let text_width = width.saturating_sub(STACKED_INDENT.len());
            indented(out, STACKED_INDENT, STACKED_INDENT, wrap(right, text_width));
        }
    }
}

fn section(out: &mut String, title: &str, lines: Vec<(String, String)>, width: usize) {
    if !out.is_empty() {
        out.push('\n');
    }
    indented(out, "", "", wrap(title, width));
    rows(out, &lines, width);
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

/// The help, wrapped to `width` columns.
pub fn help_text(width: usize) -> String {
    let mut out = String::new();
    let commands = Command::ALL
        .iter()
        .flat_map(|command| pairs(command.usage()))
        .collect();
    section(
        &mut out,
        "Commands (ralph answers these itself)",
        commands,
        width,
    );
    section(&mut out, "Cells", pairs(CELLS), width);
    section(&mut out, "Keys at the prompt", keys(PROMPT_KEYS), width);
    section(
        &mut out,
        "Keys with [chat] edit_mode = \"vi\"",
        keys(VI_KEYS),
        width,
    );
    section(&mut out, "Keys during a turn", keys(TURN_KEYS), width);
    section(
        &mut out,
        "Keys in the queue editor",
        keys(QUEUE_KEYS),
        width,
    );
    let segments = SEGMENTS
        .iter()
        .map(|kind| {
            let (name, text) = segment_help(*kind);
            (name.to_string(), text.to_string())
        })
        .collect();
    section(&mut out, "Status bar, left to right", segments, width);
    let modes = MODES
        .iter()
        .map(|mode| (mode.label().to_string(), mode_help(*mode).to_string()))
        .collect();
    section(&mut out, "Mode block (first match wins)", modes, width);
    let mut thread = thread_parts();
    thread.extend(RESETS.iter().map(|reason| {
        (
            format!("↻ {}", reason.label()),
            format!("first turn after a reset: {}", reset_help(*reason)),
        )
    }));
    let example = thread_variants(&EXAMPLE_THREAD)
        .into_iter()
        .next()
        .unwrap_or_default();
    section(
        &mut out,
        &format!("Thread cell: {example} (shorter forms drop parts)"),
        thread,
        width,
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_hold::HoldFilter;
    use crate::chat_status::{route_input, EmptySubmit, Route, Status, WatchState, WorkerState};
    use crate::chat_thread::SessionCost;
    use crate::status_mode::mode;
    use crate::status_queue::QueueCell;
    use crate::status_segments::{segments, Context};
    use std::mem::discriminant;

    /// The exhaustive match stops compiling when `$ty` gains a variant not
    /// named here; each named variant must then be in `$list`.
    macro_rules! assert_lists_every {
        ($list:expr, $ty:ident { $($variant:ident),+ $(,)? }) => {{
            let _exhaustive = |value: $ty| match value {
                $($ty::$variant)|+ => {}
            };
            $(assert!(
                $list.contains(&$ty::$variant),
                concat!(stringify!($ty), "::", stringify!($variant), " is not listed")
            );)+
        }};
    }

    fn thread(reset: Option<ResetReason>) -> ThreadCell {
        ThreadCell {
            reset,
            cost: Some(SessionCost {
                total: 4.12,
                delta: Some(0.31),
            }),
            ..EXAMPLE_THREAD
        }
    }

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
            thread: Some(thread(None)),
            napping: false,
            queue: Some(QueueCell {
                running: true,
                count: 1,
                held: false,
            }),
            hold: None,
        }
    }

    fn has_row(text: &str, label: &str) -> bool {
        text.lines().any(|line| {
            line.strip_prefix(INDENT)
                .is_some_and(|rest| rest.starts_with(&format!("{label} ")))
        })
    }

    #[test]
    fn lists_hold_every_variant() {
        assert_lists_every!(
            Command::ALL,
            Command {
                Help,
                Inbox,
                Watch,
                Diff,
                Workers,
                Hold
            }
        );
        assert_lists_every!(
            MODES,
            Mode {
                Hold,
                Nap,
                Inbox,
                Work,
                Idle
            }
        );
        assert_lists_every!(
            RESETS,
            ResetReason {
                Tokens,
                Turns,
                Restart
            }
        );
    }

    #[test]
    fn every_command_routes_locally_and_has_help() {
        let text = help_text(160);
        for command in Command::ALL {
            let route = route_input(command.name(), 0, &EmptySubmit::Inbox);
            assert!(
                !matches!(route, Route::Send(_)),
                "{} is not routed",
                command.name()
            );
            assert!(
                has_row(&text, command.name()),
                "{} has no help",
                command.name()
            );
        }
    }

    #[test]
    fn only_bare_help_routes_to_help() {
        assert_eq!(route_input("/help", 0, &EmptySubmit::Inbox), Route::Help);
        assert_eq!(
            route_input("  /help  ", 0, &EmptySubmit::Inbox),
            Route::Help
        );
        for line in ["/help x", "/help me fix the build", "/helpfoo"] {
            assert_eq!(
                route_input(line, 0, &EmptySubmit::Inbox),
                Route::Send(line.into())
            );
        }
    }

    #[test]
    fn no_line_exceeds_the_width() {
        for width in [80, 160] {
            for line in help_text(width).lines() {
                assert!(line.width() <= width, "{width} columns: {line}");
            }
        }
    }

    #[test]
    fn wrapping_keeps_every_word() {
        let words = |width| {
            help_text(width)
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        assert_eq!(words(80), words(160));
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
                hold: Some(HoldFilter::Any),
                ..full_status()
            },
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
        let text = help_text(160);
        for status in statuses {
            let current = mode(&status);
            assert!(MODES.contains(&current), "{current:?} has no /help legend");
            assert!(has_row(&text, current.label()));
        }
    }

    #[test]
    fn thread_legend_covers_every_part_of_the_real_cell() {
        let text = help_text(160);
        for reset in [None].into_iter().chain(RESETS.map(Some)) {
            let longest = thread_variants(&thread(reset)).remove(0);
            for part in longest.split(" · ") {
                let (part, rebuild) = part.strip_suffix('!').map_or((part, false), |p| (p, true));
                assert!(has_row(&text, part), "{part} has no legend row");
                assert!(!rebuild || has_row(&text, "!"), "! has no legend row");
            }
        }
    }
}
