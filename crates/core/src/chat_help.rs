//! `/help`: the chat commands, keys and status bar legend, printed by ralph
//! without a model turn.

use crate::chat::OLDER_VIEW_STEP;
use crate::chat_process::{process_variants, ProcessCell, ResetReason, CACHE_TTL_SECS};
use crate::status_mode::Mode;
use crate::status_segments::Kind;
use serde_json::{json, Value};
use std::path::Path;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Help,
    Inbox,
    Watch,
    Schedule,
    Diff,
    Workers,
    Hold,
    Nudge,
    SendIt,
    Config,
    View,
    Read,
    Zoom,
    Recall,
    Search,
}

impl Command {
    pub const ALL: [Command; 15] = [
        Command::Help,
        Command::Inbox,
        Command::Watch,
        Command::Schedule,
        Command::Diff,
        Command::Workers,
        Command::Hold,
        Command::Nudge,
        Command::SendIt,
        Command::Config,
        Command::View,
        Command::Read,
        Command::Zoom,
        Command::Recall,
        Command::Search,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Command::Help => "/help",
            Command::Inbox => "/inbox",
            Command::Watch => "/watch",
            Command::Schedule => "/schedule",
            Command::Diff => "/diff",
            Command::Workers => "/workers",
            Command::Hold => "/hold",
            Command::Nudge => "/nudge",
            Command::SendIt => "/send-it",
            Command::Config => "/config",
            Command::View => "/view",
            Command::Read => "/read",
            Command::Zoom => "/zoom",
            Command::Recall => "/recall",
            Command::Search => "/search",
        }
    }

    pub fn args(self, line: &str) -> Option<&str> {
        let args = line.strip_prefix(self.name())?;
        let takes_args = !matches!(self, Command::Help | Command::Config);
        (args.is_empty() || takes_args && args.starts_with(char::is_whitespace)).then_some(args)
    }

    fn usage(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Command::Help => &[("/help", "print this help; starts no turn")],
            Command::Inbox => &[
                (
                    "/inbox",
                    "send every unread worker report to the model, oldest first, in one turn",
                ),
                (
                    "/inbox next | N",
                    "send only report N, its position in /inbox list: oldest first, from 1; next is 1; later positions move up after a send or a drop",
                ),
                ("/inbox list", "list the unread reports with their positions; starts no turn"),
                ("/inbox peek N", "print report N; removes nothing"),
                ("/inbox drop N", "remove report N without sending it"),
            ],
            Command::Watch => &[
                (
                    "/watch start [--persistent] <cmd> -- <follow-up>",
                    "run <cmd> with sh -c as watch:wN (the lowest free N); the input splits at the last ` -- ` and both parts are required; when <cmd> exits, times out ([chat] watch_timeout_secs, default 1 hour) or is stopped, one report from watch:wN goes to the inbox with the exit code, the last 50 lines of output and the follow-up; it starts a turn only through /inbox or /hold; closing the chat kills every watch and posts nothing",
                ),
                (
                    "/watch start --persistent ...",
                    "save the watch so it runs again with its time left when ralph chat restarts; only /watch stop removes it",
                ),
                ("/watch list", "list the active watches and their time left"),
                (
                    "/watch stop <id>",
                    "kill the watch and post its stopped report; <id> is w1 or watch:w1; also removes a persistent one",
                ),
            ],
            Command::Schedule => &[
                (
                    "/schedule [list]",
                    "list the scheduled tasks: id, state, time, time left, command; starts no turn",
                ),
                (
                    "/schedule cancel <id>",
                    "remove a task, and stop its watch if it runs; /schedule creates nothing: the model proposes tasks and you arm each at a y/n prompt",
                ),
            ],
            Command::Diff => &[
                (
                    "/diff [last N]",
                    "the newest N finished chat turns combined; N defaults to 1; ralph keeps the last 20 turns",
                ),
                (
                    "/diff ago N",
                    "only the one chat turn N back; ago 1 is the same as last 1",
                ),
                (
                    "/diff worker <id>",
                    "a worker's changes from its spawn to its last turn snapshot, not the live tree",
                ),
                (
                    "/diff worker <id> last N | ago N",
                    "the same windows over the worker's turns",
                ),
                ("/diff now", "the working tree against the index"),
                (
                    "/diff ... <git diff args>",
                    "words that are not now, last N, ago N or worker <id> go to git diff: /diff turn 3 passes turn 3 to git",
                ),
            ],
            Command::Workers => &[
                (
                    "/workers",
                    "list live workers: id, harness, state, report (the worker's status and its age when newer than its last report, else the last report), head, +/-, behind; BEHIND counts against local origin/<base>, stale until a fetch; the model gets the same report from its `workers` tool, one line each, instead of inspecting processes; no turn",
                ),
                (
                    "/workers <id>",
                    "one worker in detail with its status and last 5 reports",
                ),
                (
                    "/workers costs",
                    "all-time token totals by harness and model from the global usage record, closed workers included; main is the chat process with its keepalive pings, nap the memory summaries; RUNS counts chat processes, naps or worker sessions; USD is n/a where no price is recorded and the total ends in + when part is unknown; no turn",
                ),
            ],
            Command::Hold => &[
                (
                    "/hold [<target> ...]",
                    "wait with no prompt; the model suggests it when the next step is waiting and no decision waits on the person, a target is a worker id, watch:<name> or schedule:<id>; with no target any report ends the hold, with several the first report from any of them does; typed with several targets it holds only when every target is listed, otherwise it is a normal message; one unknown target still arms the hold; when it suggests exactly /hold, /hold <target> or /hold <t1> <t2> with every target listed (a bare /hold needs one), ralph enters it at once unless chat.auto_hold is false; the first matching inbox report starts a turn, once; a watch needs the watch: prefix (/hold ci does not match watch:ci); a scheduled task reports as schedule:<id> and its worker as sched-<id>, and schedule:<id> matches both; Tab completes from workers, watches and pending and running schedules; Ctrl-Q or Ctrl-D leaves",
                ),
            ],
            Command::Nudge => &[(
                "/nudge [<id> [text]]",
                "send a worker a request for one line of progress with no model turn, then print what the worker did with it (busy, reported, taken, not started, no reaction after Ns); [text] replaces the default request; with no id it nudges the only running worker, or lists the running ids when there are several; refused while the worker waits on a permission request; while a turn runs it waits in the queue; Tab completes worker ids",
            )],
            Command::SendIt => &[
                (
                    "/send-it <instructions>",
                    "propose a run plan: the model restates the instructions as numbered steps, each with an exit criterion, plus the allowed actions, grants and caps, and stores it; ralph shows the plan and its hash; refused while an approved run is active; bare /send-it prints the usage and default caps ([chat] send_it_max_turns, send_it_max_usd)",
                ),
                (
                    "/send-it status",
                    "show the stored plan, its hash and whether it is approved; starts no turn",
                ),
                (
                    "/send-it edit",
                    "open the plan in the editor; any edit clears the approval; starts no turn",
                ),
                (
                    "/send-it go",
                    "approve the plan whose hash is on screen and run it unattended; counts only when typed at the prompt; the run stops at a cap, a failure, a decision for you, or a plan change",
                ),
                (
                    "/send-it off",
                    "delete the plan and its approval; starts no turn",
                ),
            ],
            Command::Config => &[(
                "/config",
                "the config this chat resolved at start: warnings, source files and paths, set RALPH_* and editor or color variables, start flags, and every merged key with its source (default, global, store, env or flag); notes config files changed on disk since start; starts no turn",
            )],
            Command::View => &[(
                "/view <file.md>",
                "print a local markdown file with the chat renderer; relative image paths resolve from the file's directory and draw inline where the terminal shows images; mermaid blocks draw as pictures where the terminal shows images and the text stays readable, else as text diagrams; refuses a missing file, a directory, a file that is not .md or .markdown, and a file over 2 MB; Tab completes the path from directories and .md or .markdown files; starts no turn",
            )],
            Command::Read => &[(
                "/read <id> [raw|summary] [--cursor <c>]",
                "print chat memory messages in order: an index like 5 or a range like 16-40; summary shows the leaf summary of each long message; a long range ends with `next cursor:`, pass it as --cursor with the same id and mode for the next page; the same text as the model's read tool; starts no turn",
            )],
            Command::Zoom => &[(
                "/zoom <id>",
                "open one chat memory node: a block id like 4-7 shows its two halves, an index like 5 the raw message; the same text as the model's zoom tool; starts no turn",
            )],
            Command::Recall => &[(
                "/recall <regex>",
                "list the raw messages that match the case-insensitive regex, newest first, at most 50; the rest of the line is the pattern; the same text as the model's recall tool; starts no turn",
            )],
            Command::Search => &[(
                "/search <words> [--cursor <c>]",
                "rank the raw messages by bare words, \"quoted phrases\" and identifiers such as GUZ-709; more hits end with `next cursor:`, pass it as --cursor with the same words; the same text as the model's search tool, without its role, date and limit filters; starts no turn",
            )],
        }
    }
}

const CELLS: &[(&str, &str)] = &[
    (
        "!<command>",
        "run the rest of the message as one !shell cell, then send it with its output",
    ),
    (
        "!!<command>",
        "run it as one !shell cell; the output stays local, no turn starts",
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
        keys: "Tab, Shift-Tab",
        effect: "after /hold , complete the word under the cursor from this chat's worker ids, watch:<name> and schedule:<id>; after /view , complete the path from directories and .md or .markdown files; Tab opens the menu and moves on, Shift-Tab moves back, Enter takes the pick",
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
        keys: "Alt-Enter, Ctrl-J, Ctrl-V, Ctrl-O, Tab, Shift-Tab",
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
        keys: "Ctrl-G",
        effect: "edit the queue draft in [chat] editor, $VISUAL, $EDITOR or vi; the turn keeps running",
    },
    KeyHelp {
        keys: "Ctrl-V",
        effect: "off here: no image paste",
    },
];

/// Status bar segments in the order they are explained.
pub const SEGMENTS: [Kind; 10] = [
    Kind::Mode(Mode::Idle),
    Kind::Hold(Mode::Idle),
    Kind::Queue(Mode::Idle),
    Kind::Mail(Mode::Idle),
    Kind::Name,
    Kind::Workers,
    Kind::Watches,
    Kind::Schedule,
    Kind::Process(Mode::Idle),
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
            "only during /hold: the source it waits for, or anyone",
        ),
        Kind::Queue(_) => (
            "turn running · QUEUED N",
            "only in the queue editor: turn state and queued entries; HELD N after a back-out; short forms running · QN, ▶ QN, done · QN, ✓ QN",
        ),
        Kind::Mail(_) => ("📬 N unread", "worker reports waiting; /inbox sends them"),
        Kind::Name => ("name", "the chat session name"),
        Kind::Workers => ("workers", "each worker and its state"),
        Kind::Watches => ("watch <id> <left>", "running watches and their time left"),
        Kind::Schedule => (
            "sched <id> <HH:MM>",
            "armed scheduled tasks: the one task and its time, N scheduled, or SN",
        ),
        Kind::Process(_) => ("process cell", "the live claude process (below)"),
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

/// The cell the process legend shows, rendered by the status bar formatter.
const EXAMPLE_PROCESS: ProcessCell = ProcessCell {
    turn: 12,
    max_turns: 30,
    ctx_percent: Some(62),
    cache_minutes: Some(41),
    write_heavy: true,
    reset: None,
    shift: Some(9),
    cost: None,
};

fn process_parts() -> Vec<(String, String)> {
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
                "estimated minutes left of the prompt cache ({} min TTL from the last reply or keepalive ping); cold when none",
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

pub fn help_with_store(width: usize, store: &Path) -> String {
    format!("{}\n\n{}", store_line(store), help_text(width))
}

fn store_line(store: &Path) -> String {
    format!("Store: {}", store.display())
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
    let mut process = process_parts();
    process.extend(RESETS.iter().map(|reason| {
        (
            format!("↻ {}", reason.label()),
            format!("first turn after a reset: {}", reset_help(*reason)),
        )
    }));
    let example = process_variants(&EXAMPLE_PROCESS)
        .into_iter()
        .next()
        .unwrap_or_default();
    section(
        &mut out,
        &format!("Process cell: {example} (shorter forms drop parts)"),
        process,
        width,
    );
    out
}

const TOOL_WIDTH: usize = 1000;

const IMAGES_HELP: &str = "\
Images
![alt](/absolute/path) in a reply draws PNG, JPEG, GIF or WebP inline in Rex, Ghostty and kitty; tmux needs allow-passthrough on.
Only local files draw; http(s) and data: URLs stay text.
Drawing is off with NO_COLOR, in pipes and in unsupported terminals: the raw ![alt](path) text shows.
A missing file, a file over 10 MiB or one that can't be decoded prints [image: alt (path)].
Under each drawn image a dim line with its full path shows; in terminals with OSC 8 links, click it to open the file.
Ctrl-V pastes a clipboard image as [Image #N: path].
";

const CONFIG_DOC: &str = include_str!("../../../docs/configuration.md");

fn topic_name(command: Command) -> &'static str {
    command.name().trim_start_matches('/')
}

fn topic_list() -> String {
    Command::ALL.map(topic_name).join(", ")
}

fn config_sections(doc: &str) -> Vec<(String, String)> {
    let mut sections: Vec<(String, String)> = Vec::new();
    let mut open = false;
    for line in doc.lines() {
        if line.starts_with("## ") || line.starts_with("### ") {
            open = false;
            if let Some(name) = line
                .strip_prefix("### `[")
                .and_then(|rest| rest.split_once("]`"))
                .map(|(name, _)| name)
            {
                sections.push((name.to_string(), String::new()));
                open = true;
            }
        }
        if let (true, Some((_, body))) = (open, sections.last_mut()) {
            body.push_str(line);
            body.push('\n');
        }
    }
    sections
        .into_iter()
        .map(|(name, body)| (name, body.trim_end().to_string()))
        .collect()
}

fn doc_block(doc: &str, title: &str) -> String {
    let heading = format!("## {title}");
    let mut lines = doc.lines().skip_while(|line| *line != heading);
    let first = lines.next().into_iter();
    first
        .chain(lines.take_while(|line| !line.starts_with("## ")))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}

fn section_list(sections: &[(String, String)]) -> String {
    sections
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn config_topic(topic: Option<&str>) -> Option<Option<&str>> {
    let topic = topic?.trim();
    let (head, rest) = topic.split_once(char::is_whitespace).unwrap_or((topic, ""));
    head.eq_ignore_ascii_case("config")
        .then(|| Some(rest.trim()).filter(|section| !section.is_empty()))
}

pub fn config_help(
    path: &Path,
    store_dir: Option<&Path>,
    section: Option<&str>,
) -> Result<String, String> {
    let sections = config_sections(CONFIG_DOC);
    let Some(section) = section.map(str::trim).filter(|section| !section.is_empty()) else {
        let store_header = store_dir.map_or(String::new(), |dir| format!("{}\n", store_line(dir)));
        let store_file_line = store_dir.map_or(String::new(), |dir| {
            format!(
                "\nStore config file: {} (optional; overrides the global file per key)",
                dir.join("config.toml").display()
            )
        });
        return Ok(format!(
            "{}Config file: {}{}\n\n{}\n\n{}\n\nSections: {}. Ask for one with topic `config <section>`.",
            store_header,
            path.display(),
            store_file_line,
            doc_block(CONFIG_DOC, "Configuration File Location"),
            doc_block(CONFIG_DOC, "Configuration Precedence"),
            section_list(&sections),
        ));
    };
    let wanted = section.trim_start_matches('[').trim_end_matches(']');
    sections
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
        .map(|(_, body)| body.clone())
        .ok_or_else(|| {
            format!(
                "Unknown config section '{section}'. Sections: {}.",
                section_list(&sections)
            )
        })
}

pub fn help_tool() -> Value {
    json!({
        "name": "help",
        "description": format!("ralph chat's own /help: its commands, cells, keys and status bar legend. Call it when the user asks how ralph itself works, then answer from its text. Without topic it returns the full help; topic returns one command's section: {}. Topic `config` explains ralph's settings file (path, precedence, sections); `config <section>` returns one section such as `config chat`. Topic `images` explains how a reply shows an image.", topic_list()),
        "inputSchema": {
            "type": "object",
            "properties": {
                "topic": {"type": "string", "description": "A command name such as diff or /diff, `images`, `config`, or `config <section>`. Omit it for the full help."}
            }
        }
    })
}

pub fn help_topic(topic: Option<&str>) -> Result<String, String> {
    let Some(topic) = topic.map(str::trim).filter(|topic| !topic.is_empty()) else {
        return Ok(help_text(TOOL_WIDTH));
    };
    let wanted = topic.strip_prefix('/').unwrap_or(topic);
    if wanted.eq_ignore_ascii_case("images") {
        return Ok(IMAGES_HELP.to_string());
    }
    let command = Command::ALL
        .into_iter()
        .find(|command| topic_name(*command).eq_ignore_ascii_case(wanted))
        .ok_or_else(|| {
            format!(
                "Unknown help topic '{topic}'. Topics: {}, images, config, config <section>.",
                topic_list()
            )
        })?;
    let mut out = String::new();
    section(&mut out, command.name(), pairs(command.usage()), TOOL_WIDTH);
    let name = topic_name(command);
    if config_sections(CONFIG_DOC)
        .iter()
        .any(|(section, _)| section == name)
    {
        out.push_str(&format!("Settings: help topic \"config {name}\"\n"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
