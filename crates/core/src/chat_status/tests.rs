use super::*;
use crate::changes::{ScopeQuery, TurnWindow};
use crate::diff_command::{DiffArgs, DiffTarget, WINDOW_USAGE};

#[test]
fn fit_truncates_wide_chars_by_display_width() {
    let line = "📬 2 unread".to_string();
    for width in 0..=12 {
        let fitted = fit(vec![line.clone()], width);
        assert!(fitted.width() <= width, "width {width}: {fitted:?}");
    }
    assert_eq!(fit(vec![line], 1), "");
}

#[test]
fn watch_states_report_the_time_left() {
    use crate::watch::{WatchKind, WatchSpec};
    let now = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    let spec = |id: &str, command: &str| WatchSpec {
        id: id.into(),
        command: command.into(),
        follow_up: None,
        kind: WatchKind::Normal,
        timeout: None,
        limit: None,
    };
    let saved = [
        SavedWatch::new(&spec("w1", "sleep 90"), now),
        SavedWatch::new(&spec("w2", "make"), now),
    ];
    assert_eq!(
        watch_states(&saved, now + chrono::Duration::seconds(30)),
        vec![
            WatchState {
                id: "w1".into(),
                left: Some("1m".into())
            },
            WatchState {
                id: "w2".into(),
                left: None
            }
        ]
    );
}

#[test]
fn watch_states_are_equal_one_second_apart_at_twelve_minutes() {
    use crate::watch::{WatchKind, WatchSpec};
    let now = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    let spec = WatchSpec {
        id: "w1".into(),
        command: "sleep 3600".into(),
        follow_up: None,
        kind: WatchKind::Normal,
        timeout: None,
        limit: None,
    };
    let saved = [SavedWatch::new(&spec, now)];
    let at = |secs| watch_states(&saved, now + chrono::Duration::seconds(secs));
    assert_eq!(at(2870), at(2871));
    assert_eq!(at(2870)[0].left.as_deref(), Some("12m"));
}

#[test]
fn only_read_only_routes_run_mid_turn() {
    let runs = |line: &str| route_input(line, 0, &EmptySubmit::Inbox).runs_mid_turn();
    for line in [
        "/inbox peek 1",
        "/inbox peek x",
        "/inbox list",
        "/inbox bogus a b",
        "/workers",
        "/workers w1",
        "/help",
        "/watch list",
        "/watch",
        "/diff last 0",
        "/diff \"x",
    ] {
        assert!(runs(line), "{line}");
    }
    for line in [
        "/inbox",
        "/inbox next",
        "/inbox 2",
        "/inbox drop 1",
        "/watch stop w1",
        "/watch start sleep 5 -- done",
        "/diff HEAD",
        "/workers a b",
        "/hold",
        "/hold w1",
        "hello",
        "",
    ] {
        assert!(!runs(line), "{line}");
    }
}

#[test]
fn route_input_parses_watch_commands() {
    assert_eq!(
        route_input("/watch list", 0, &EmptySubmit::Inbox),
        Route::Watch(WatchCommand::List)
    );
    assert_eq!(
        route_input("/watch", 3, &EmptySubmit::Inbox),
        Route::Watch(WatchCommand::Usage)
    );
    assert_eq!(
        route_input(" /watch stop w1 ", 0, &EmptySubmit::Inbox),
        Route::Watch(WatchCommand::Stop("watch:w1".into()))
    );
    assert!(matches!(
        route_input(
            "/watch start --persistent sleep 5 -- done",
            0,
            &EmptySubmit::Inbox
        ),
        Route::Watch(WatchCommand::Start(_))
    ));
    assert_eq!(
        route_input("/watchfoo", 0, &EmptySubmit::Inbox),
        Route::Send("/watchfoo".into())
    );
}

#[test]
fn route_input_parses_diff_commands() {
    let last = |count, extra: &[&str]| {
        Route::Diff(DiffArgs::Target(DiffTarget::Range {
            scope: ScopeQuery::Chat(TurnWindow::Last(count)),
            extra: extra.iter().map(|word| word.to_string()).collect(),
        }))
    };
    assert_eq!(
        route_input("/diff -U1 -- a", 0, &EmptySubmit::Inbox),
        last(1, &["-U1", "--", "a"])
    );
    assert_eq!(
        route_input(" /diff -- \"a b\" ", 3, &EmptySubmit::Inbox),
        last(1, &["--", "a b"])
    );
    assert_eq!(route_input("/diff", 0, &EmptySubmit::Inbox), last(1, &[]));
    assert_eq!(
        route_input("/diff last 2", 0, &EmptySubmit::Inbox),
        last(2, &[])
    );
    assert_eq!(
        route_input("/diff now -U1", 0, &EmptySubmit::Inbox),
        Route::Diff(DiffArgs::Target(DiffTarget::Now(vec!["-U1".into()])))
    );
    assert_eq!(
        route_input("/diff last 0", 0, &EmptySubmit::Inbox),
        Route::Diff(DiffArgs::Usage(WINDOW_USAGE))
    );
    assert_eq!(
        route_input("/diff 'a", 0, &EmptySubmit::Inbox),
        Route::Diff(DiffArgs::UnclosedQuote)
    );
    assert_eq!(
        route_input("/difffoo", 0, &EmptySubmit::Inbox),
        Route::Send("/difffoo".into())
    );
}

#[test]
fn route_input_parses_workers_commands() {
    let route = |line| route_input(line, 0, &EmptySubmit::Inbox);
    assert_eq!(route("/workers"), Route::Workers(None));
    assert_eq!(route("  /workers  "), Route::Workers(None));
    assert_eq!(
        route("/workers GUZ-1"),
        Route::Workers(Some("GUZ-1".into()))
    );
    assert_eq!(
        route(" /workers   GUZ-1 "),
        Route::Workers(Some("GUZ-1".into()))
    );
}

#[test]
fn only_bare_or_one_token_workers_route_locally() {
    for line in ["/workers a b", "/workers a b c", "/workersfoo"] {
        assert_eq!(
            route_input(line, 0, &EmptySubmit::Inbox),
            Route::Send(line.into())
        );
    }
}

#[test]
fn route_input_parses_hold_commands() {
    let route = |line| route_input(line, 0, &EmptySubmit::Inbox);
    assert_eq!(route("/hold"), Route::Hold(HoldFilter::Any));
    assert_eq!(route("  /hold  "), Route::Hold(HoldFilter::Any));
    assert_eq!(
        route("/hold GUZ-1"),
        Route::Hold(HoldFilter::Source("GUZ-1".into()))
    );
    assert_eq!(
        route(" /hold   watch:ci "),
        Route::Hold(HoldFilter::Source("watch:ci".into()))
    );
}

#[test]
fn only_bare_or_one_token_hold_routes_locally() {
    for line in ["/hold a b", "/holdfoo"] {
        assert_eq!(
            route_input(line, 0, &EmptySubmit::Inbox),
            Route::Send(line.into())
        );
    }
}

#[test]
fn footer_shows_inbox_hint_only_with_mail() {
    assert!(!footer(0, 200).contains("/inbox"));
    assert_eq!(
        footer(2, 200),
        "Enter send · Ctrl+J newline · /inbox or Enter on empty prompt reads 2 reports · Ctrl+G editor · Ctrl+D quit\n"
    );
    assert!(footer(2, 90).contains("/inbox read 2 reports"));
    assert!((0..120).all(|w| footer(2, w).trim_end().width() <= w));
}

#[test]
fn empty_route_decides_from_mode_and_unread() {
    let phrase = || "continue".to_string();
    let send = || Route::Send(phrase());
    let read = || Route::ReadInbox(Selection::All);
    assert_eq!(empty_route(0, &EmptySubmit::Inbox), Route::Ignore);
    assert_eq!(empty_route(2, &EmptySubmit::Inbox), read());
    assert_eq!(empty_route(0, &EmptySubmit::InboxFirst(phrase())), send());
    assert_eq!(empty_route(2, &EmptySubmit::InboxFirst(phrase())), read());
    assert_eq!(empty_route(0, &EmptySubmit::Phrase(phrase())), send());
    assert_eq!(empty_route(2, &EmptySubmit::Phrase(phrase())), send());
}

#[test]
fn route_input_sends_phrase_only_on_empty_line() {
    let phrase = EmptySubmit::Phrase("go".into());
    assert_eq!(route_input("  ", 3, &phrase), Route::Send("go".into()));
    assert_eq!(route_input("hi", 3, &phrase), Route::Send("hi".into()));
    assert_eq!(
        route_input("/inbox", 3, &phrase),
        Route::ReadInbox(Selection::All)
    );
}

#[test]
fn route_input_decides_from_line_and_unread() {
    assert_eq!(
        route_input(" /inbox ", 0, &EmptySubmit::Inbox),
        Route::ReadInbox(Selection::All)
    );
    assert_eq!(
        route_input("  ", 3, &EmptySubmit::Inbox),
        Route::ReadInbox(Selection::All)
    );
    assert_eq!(
        route_input("/inbox next", 0, &EmptySubmit::Inbox),
        Route::ReadInbox(Selection::Next)
    );
    assert_eq!(
        route_input("/inbox 2", 0, &EmptySubmit::Inbox),
        Route::ReadInbox(Selection::At("2".parse().unwrap()))
    );
    assert_eq!(route_input("", 0, &EmptySubmit::Inbox), Route::Ignore);
    assert_eq!(
        route_input(" hi ", 3, &EmptySubmit::Inbox),
        Route::Send("hi".into())
    );
    assert_eq!(
        route_input("/inbox list", 0, &EmptySubmit::Inbox),
        Route::ListInbox
    );
    assert_eq!(
        route_input("/inbox peek 3", 0, &EmptySubmit::Inbox),
        Route::PeekInbox("3".parse().unwrap())
    );
    assert_eq!(
        route_input("/inbox drop 3", 0, &EmptySubmit::Inbox),
        Route::DropInbox("3".parse().unwrap())
    );
    for usage in [
        "/inbox drop x",
        "/inbox drop",
        "/inbox drop 1 2",
        "/inbox peek x",
        "/inbox peek",
        "/inbox foo",
        "/inbox list x",
        "/inbox peek 1 2",
        "/inbox 0",
        "/inbox x",
        "/inbox 2 3",
        "/inbox next 1",
    ] {
        assert_eq!(
            route_input(usage, 3, &EmptySubmit::Inbox),
            Route::InboxUsage
        );
    }
    assert_eq!(
        route_input("/inboxfoo", 0, &EmptySubmit::Inbox),
        Route::Send("/inboxfoo".into())
    );
}

#[test]
fn store_workers_keep_only_this_stores_sessions() {
    let listing = "ralph-p-A\t/s\tA\nralph-p-B\t/other\tB\nralph-p-C\t\tC\nralph-D\t/s\nmain\t/s\n";
    let ids = |store| -> Vec<String> {
        store_workers(listing, store)
            .into_iter()
            .map(|worker| worker.id)
            .collect()
    };
    assert_eq!(ids("/s"), vec!["A"]);
    assert_eq!(ids("/other"), vec!["B"]);
    assert!(store_workers("", "/s").is_empty());
}

#[test]
fn workers_come_from_prefixed_sessions_with_last_known_state() {
    let listing = "main\t/s\nralph-p-GUZ-2\t/s\tGUZ-2\nralph-p-GUZ-1\t/s\tGUZ-1\nralph-\t/s\t\n";
    let workers = store_workers(listing, "/s");
    let ids: Vec<&str> = workers.iter().map(|worker| worker.id.as_str()).collect();
    assert_eq!(ids, vec!["GUZ-1", "GUZ-2"]);
    let reports = [
        Report {
            worker: "GUZ-2".into(),
            at: "a".into(),
            status: "question".into(),
            text: "x".into(),
        },
        Report {
            worker: "GUZ-2".into(),
            at: "b".into(),
            status: "done".into(),
            text: "y".into(),
        },
    ];
    let states = worker_states(
        &workers,
        &latest_states(&reports),
        &HashSet::new(),
        None,
        &HashSet::new(),
    );
    assert_eq!(states[0].state, "running");
    assert_eq!(states[1].state, "done");
}

#[test]
fn store_workers_take_the_bare_id_from_the_id_tag() {
    let workers = store_workers("ralph-my-proj-GUZ-1\t/s\tGUZ-1\n", "/s");
    assert_eq!(
        workers,
        vec![RunningWorker {
            id: "GUZ-1".into(),
            session: "ralph-my-proj-GUZ-1".into(),
            kind: HarnessId::Claude
        }]
    );
}

#[test]
fn running_worker_loads_legacy_cursor_records() {
    for (cursor, kind) in [(true, HarnessId::Cursor), (false, HarnessId::Claude)] {
        let record = serde_json::json!({
            "id": "A", "session": "", "opencode": false, "cursor": cursor
        });
        let worker: RunningWorker = serde_json::from_value(record).unwrap();
        assert_eq!(worker.kind, kind);
        let saved = serde_json::to_value(&worker).unwrap();
        assert!(saved.get("cursor").is_none());
        assert_eq!(
            serde_json::from_value::<RunningWorker>(saved).unwrap(),
            worker
        );
    }
}

#[test]
fn running_worker_loads_explicit_kinds_and_defaults_to_claude() {
    for (name, kind) in [
        ("tmux", HarnessId::Claude),
        ("claude", HarnessId::Claude),
        ("cursor", HarnessId::Cursor),
        ("opencode", HarnessId::Opencode),
    ] {
        let record = serde_json::json!({
            "id": "A", "session": "", "opencode": false, "kind": name
        });
        let worker: RunningWorker = serde_json::from_value(record).unwrap();
        assert_eq!(worker.kind, kind);
        assert_eq!(serde_json::to_value(worker).unwrap()["kind"], kind.as_str());
    }
    let worker: RunningWorker =
        serde_json::from_str(r#"{"id":"A","session":"s","opencode":false}"#).unwrap();
    assert_eq!(worker.kind, HarnessId::Claude);
}

#[test]
fn orphan_hint_lists_ready_ids_without_a_session() {
    let running = [RunningWorker {
        id: "A".into(),
        session: "ralph-p-A".into(),
        kind: HarnessId::Claude,
    }];
    let ready = ["C".to_string(), "A".to_string(), "B".to_string()];
    let hint = orphan_hint(&ready, &running, &[], &[]).unwrap();
    assert!(hint.contains(": B, C."), "{hint}");
    assert!(hint.contains("nothing was removed"));
    assert_eq!(orphan_hint(&ready[1..2], &running, &[], &[]), None);
    assert_eq!(orphan_hint(&[], &running, &[], &[]), None);
}

#[test]
fn orphan_hint_skips_an_acp_alive_cursor_worker() {
    let running = [
        RunningWorker {
            id: "claude-up".into(),
            session: "ralph-p-claude-up".into(),
            kind: HarnessId::Claude,
        },
        RunningWorker::headless("cursor-owned", HarnessId::Cursor),
    ];
    let ready = [
        "cursor-live".to_string(),
        "cursor-dead".to_string(),
        "cursor-owned".to_string(),
        "claude-down".to_string(),
        "claude-up".to_string(),
    ];
    let alive = ["cursor-live".to_string()];
    let hint = orphan_hint(&ready, &running, &alive, &[]).unwrap();
    assert!(hint.contains("cursor-dead"), "{hint}");
    assert!(hint.contains("claude-down"), "{hint}");
    assert!(!hint.contains("cursor-live"), "{hint}");
    assert!(!hint.contains("cursor-owned"), "{hint}");
    assert!(!hint.contains("claude-up"), "{hint}");
    assert_eq!(
        orphan_hint(&["cursor-live".to_string()], &[], &alive, &[]),
        None
    );
}

#[test]
fn orphan_hint_skips_a_resumable_cursor_session_whose_agent_is_dead() {
    let ready = [
        "cursor-resume".to_string(),
        "cursor-gone".to_string(),
        "claude-down".to_string(),
    ];
    let resumable = ["cursor-resume".to_string()];
    let hint = orphan_hint(&ready, &[], &[], &resumable).unwrap();
    assert!(hint.contains("cursor-gone"), "{hint}");
    assert!(hint.contains("claude-down"), "{hint}");
    assert!(!hint.contains("cursor-resume"), "{hint}");
    assert_eq!(
        orphan_hint(&["cursor-resume".to_string()], &[], &[], &resumable),
        None
    );
}

#[test]
fn orphan_hint_lists_a_never_saved_cursor_worker() {
    let ready = [
        "cursor-new".to_string(),
        "cursor-live".to_string(),
        "cursor-resume".to_string(),
    ];
    let alive = ["cursor-live".to_string()];
    let resumable = ["cursor-resume".to_string()];
    let hint = orphan_hint(&ready, &[], &alive, &resumable).unwrap();
    assert!(hint.contains("cursor-new"), "{hint}");
    assert!(!hint.contains("cursor-live"), "{hint}");
    assert!(!hint.contains("cursor-resume"), "{hint}");
}

#[test]
fn opencode_workers_are_busy_while_their_session_is_active() {
    let worker = |id: &str, kind| RunningWorker {
        id: id.into(),
        session: format!("ralph-p-{id}"),
        kind,
    };
    let workers = [
        worker("busy", HarnessId::Opencode),
        worker("idle", HarnessId::Opencode),
        worker("done", HarnessId::Opencode),
        worker("redo", HarnessId::Opencode),
        worker("claude", HarnessId::Claude),
    ];
    let seen = HashMap::from([
        ("done".to_string(), "done".to_string()),
        ("redo".to_string(), "done".to_string()),
    ]);
    let active = HashSet::from(["ses_ralphbusy".to_string(), "ses_ralphredo".to_string()]);
    let states = |active| -> Vec<String> {
        worker_states(&workers, &seen, &HashSet::new(), active, &HashSet::new())
            .into_iter()
            .map(|worker| worker.state)
            .collect()
    };
    assert_eq!(
        states(Some(&active)),
        vec!["busy", "idle", "done", "busy", "running"]
    );
    assert_eq!(
        states(None),
        vec!["running", "running", "done", "done", "running"]
    );
}

#[test]
fn a_sent_message_drops_a_report_status_but_keeps_other_statuses() {
    let worker = |id: &str, kind| RunningWorker {
        id: id.into(),
        session: format!("ralph-p-{id}"),
        kind,
    };
    let workers = [
        worker("redone", HarnessId::Claude),
        worker("quiet", HarnessId::Claude),
        worker("checked", HarnessId::Claude),
        worker("ask", HarnessId::Claude),
        worker("busy", HarnessId::Opencode),
    ];
    let seen = HashMap::from([
        ("redone".to_string(), "done".to_string()),
        ("quiet".to_string(), "done".to_string()),
        ("checked".to_string(), "check".to_string()),
        ("ask".to_string(), "question".to_string()),
        ("busy".to_string(), "done".to_string()),
    ]);
    let sent: HashSet<String> = ["redone", "checked", "ask", "busy"]
        .map(String::from)
        .into();
    let active = HashSet::from(["ses_ralphbusy".to_string()]);
    let states: Vec<String> = worker_states(&workers, &seen, &sent, Some(&active), &HashSet::new())
        .into_iter()
        .map(|worker| worker.state)
        .collect();
    assert_eq!(states, vec!["running", "done", "check", "running", "busy"]);
}

#[test]
fn a_pending_permission_overrides_a_report_and_counts_as_running() {
    let workers = [RunningWorker {
        id: "w".into(),
        session: "ralph-p-w".into(),
        kind: HarnessId::Cursor,
    }];
    let seen = HashMap::from([("w".to_string(), "question".to_string())]);
    let blocked = HashSet::from(["w".to_string()]);
    let states = worker_states(&workers, &seen, &HashSet::new(), None, &blocked);
    assert_eq!(states[0].state, "permission");
    assert!(states[0].is_running());
}

#[test]
fn schedule_states_list_armed_tasks_by_time_in_local_hours() {
    use crate::schedule::{Task, TaskKind, TaskStatus};
    let task = |id: &str, at: &str, status| Task {
        id: id.into(),
        kind: TaskKind::Command,
        command: "x".into(),
        follow_up: None,
        at: chrono::DateTime::parse_from_rfc3339(at).unwrap().into(),
        timeout_secs: None,
        status,
        late: false,
    };
    let tasks = [
        task("b", "2026-01-01T12:00:00Z", TaskStatus::Pending),
        task("a", "2026-01-01T10:00:00Z", TaskStatus::Pending),
        task("p", "2026-01-01T09:00:00Z", TaskStatus::Proposed),
        task("r", "2026-01-01T08:00:00Z", TaskStatus::Running),
    ];
    let offset = chrono::FixedOffset::east_opt(3600).unwrap();
    let now = chrono::DateTime::parse_from_rfc3339("2026-01-01T07:00:00Z")
        .unwrap()
        .into();
    let state = |id: &str, time: &str| ScheduleState {
        id: id.into(),
        time: time.into(),
    };
    assert_eq!(
        schedule_states(&tasks, now, offset),
        [state("a", "11:00"), state("b", "13:00")]
    );
    assert!(schedule_states(&[], now, offset).is_empty());
}

#[test]
fn schedule_states_mark_tasks_due_on_a_later_local_day() {
    use crate::schedule::{Task, TaskKind, TaskStatus};
    let task = |id: &str, at: &str| Task {
        id: id.into(),
        kind: TaskKind::Command,
        command: "x".into(),
        follow_up: None,
        at: chrono::DateTime::parse_from_rfc3339(at).unwrap().into(),
        timeout_secs: None,
        status: TaskStatus::Pending,
        late: false,
    };
    let tasks = [
        task("today", "2026-01-01T22:30:00Z"),
        task("tomorrow", "2026-01-01T23:30:00Z"),
        task("later", "2026-01-04T08:00:00Z"),
        task("missed", "2025-12-31T08:00:00Z"),
    ];
    let offset = chrono::FixedOffset::east_opt(3600).unwrap();
    let now = chrono::DateTime::parse_from_rfc3339("2026-01-01T20:00:00Z")
        .unwrap()
        .into();
    let times: Vec<(String, String)> = schedule_states(&tasks, now, offset)
        .into_iter()
        .map(|state| (state.id, state.time))
        .collect();
    let pair = |id: &str, time: &str| (id.to_string(), time.to_string());
    assert_eq!(
        times,
        [
            pair("missed", "09:00"),
            pair("today", "23:30"),
            pair("tomorrow", "00:30+1d"),
            pair("later", "09:00+3d"),
        ]
    );
}

#[test]
fn schedule_routes_locally_and_only_list_runs_mid_turn() {
    use crate::schedule_command::ScheduleCommand;
    let route = |line| route_input(line, 0, &EmptySubmit::Inbox);
    assert_eq!(route("/schedule"), Route::Schedule(ScheduleCommand::List));
    assert!(route("/schedule list").runs_mid_turn());
    assert!(route("/schedule x").runs_mid_turn());
    assert!(!route("/schedule cancel a").runs_mid_turn());
    assert_eq!(route("/schedulefoo"), Route::Send("/schedulefoo".into()));
}
