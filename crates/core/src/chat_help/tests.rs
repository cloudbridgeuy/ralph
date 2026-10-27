use super::*;
use crate::chat_hold::HoldFilter;
use crate::chat_status::{
    route_input, EmptySubmit, Route, ScheduleState, Status, WatchState, WorkerState,
};
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
        schedule: vec![ScheduleState {
            id: "nightly".into(),
            time: "23:00".into(),
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
            Schedule,
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

#[test]
fn help_topic_without_topic_is_the_full_help() {
    let full = help_text(TOOL_WIDTH);
    assert_eq!(help_topic(None), Ok(full.clone()));
    assert_eq!(help_topic(Some("  ")), Ok(full));
}

#[test]
fn help_topic_selects_one_command_section() {
    for command in Command::ALL {
        let name = command.name();
        for topic in [name, topic_name(command), &name.to_uppercase()] {
            let text = help_topic(Some(topic)).unwrap();
            assert!(text.starts_with(&format!("{name}\n")), "{topic}");
            for (label, _) in command.usage() {
                assert!(has_row(&text, label), "{topic} lacks {label}");
            }
            let settings = usize::from(text.contains("Settings: "));
            assert_eq!(text.lines().count(), command.usage().len() + 1 + settings);
        }
    }
}

#[test]
fn inbox_help_explains_positions() {
    let text = help_topic(Some("inbox")).unwrap();
    assert!(text.contains("oldest first, from 1"));
    assert!(text.contains("move up after a send or a drop"));
}

#[test]
fn watch_help_explains_reports() {
    let text = help_topic(Some("watch")).unwrap();
    assert!(text.contains(&format!("last {} lines", crate::watch::TAIL_MAX_LINES)));
    for part in [
        "watch_timeout_secs",
        "watch:wN",
        "/inbox or /hold",
        "w1 or watch:w1",
    ] {
        assert!(text.contains(part), "{part}");
    }
    assert!(has_row(&text, "/watch start --persistent ..."));
}

#[test]
fn diff_help_tells_last_from_ago() {
    let text = help_topic(Some("diff")).unwrap();
    assert!(has_row(&text, "/diff [last N]") && has_row(&text, "/diff ago N"));
    assert!(text.contains(&format!("last {} turns", crate::snapshot::KEEP_TURNS)));
    for part in [
        "ago 1 is the same as last 1",
        "not the live tree",
        "/diff turn 3",
    ] {
        assert!(text.contains(part), "{part}");
    }
}

#[test]
fn help_topic_points_to_matching_config_section() {
    assert!(help_topic(Some("workers"))
        .unwrap()
        .contains("Settings: help topic \"config workers\""));
    assert!(!help_topic(Some("inbox")).unwrap().contains("Settings:"));
}

#[test]
fn unknown_help_topic_lists_every_command() {
    let error = help_topic(Some("nope")).unwrap_err();
    assert!(error.starts_with("Unknown help topic 'nope'."));
    for command in Command::ALL {
        assert!(error.contains(topic_name(command)));
    }
}

#[test]
fn help_tool_names_every_topic() {
    let tool = help_tool();
    assert_eq!(tool["name"], "help");
    let description = tool["description"].as_str().unwrap();
    assert!(description.contains(&topic_list()));
    assert!(tool["inputSchema"].get("required").is_none());
}

#[test]
fn config_sections_split_on_headings() {
    let doc = "## A\n### `[x]` - X\nbody\n#### sub\nmore\n### Other\nno\n### `[y.z]`\nyy\n";
    assert_eq!(
        config_sections(doc),
        [
            (
                "x".to_string(),
                "### `[x]` - X\nbody\n#### sub\nmore".to_string()
            ),
            ("y.z".to_string(), "### `[y.z]`\nyy".to_string()),
        ]
    );
}

#[test]
fn config_sections_cover_every_config_field() {
    let names: Vec<String> = config_sections(CONFIG_DOC)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    for field in [
        "theme", "ask", "chat", "mermaid", "cursor", "codex", "claude", "opencode", "changes",
        "wire", "tickets", "workers",
    ] {
        assert!(
            names
                .iter()
                .any(|name| name == field || name.starts_with(&format!("{field}."))),
            "{field}"
        );
    }
}

#[test]
fn config_help_selects_a_section_or_lists_them() {
    let path = Path::new("/x/ralph/config.toml");
    let overview = config_help(path, None).unwrap();
    assert!(overview.contains("/x/ralph/config.toml"));
    assert!(overview.contains("RALPH_CONFIG_DIR"));
    assert!(overview.contains("Configuration Precedence"));
    for (name, _) in config_sections(CONFIG_DOC) {
        assert!(overview.contains(&name), "{name}");
    }
    let chat = config_help(path, Some("[Chat]")).unwrap();
    assert!(chat.starts_with("### `[chat]`"));
    assert!(!chat.contains("`[cursor]`"));
    let error = config_help(path, Some("nope")).unwrap_err();
    assert!(error.contains("Unknown config section 'nope'") && error.contains("workers"));
}

#[test]
fn config_topic_parses_section() {
    assert_eq!(config_topic(Some("config")), Some(None));
    assert_eq!(config_topic(Some(" Config  chat ")), Some(Some("chat")));
    assert_eq!(config_topic(Some("diff")), None);
    assert_eq!(config_topic(None), None);
}
