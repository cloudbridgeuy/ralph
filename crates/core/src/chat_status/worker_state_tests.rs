use super::*;
use crate::worker::WorkerId;

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
        Report::from_worker(
            &WorkerId::parse("GUZ-2").unwrap(),
            chrono::DateTime::UNIX_EPOCH,
            crate::worker::ReportStatus::Question,
            "x".into(),
        ),
        Report::from_worker(
            &WorkerId::parse("GUZ-2").unwrap(),
            chrono::DateTime::UNIX_EPOCH,
            crate::worker::ReportStatus::Done,
            "y".into(),
        ),
    ];
    let states = worker_states(
        &workers,
        &latest_states(&reports),
        &HashSet::new(),
        None,
        &HashSet::new(),
    );
    assert_eq!(states[0].state, WorkerActivity::Running);
    assert_eq!(
        states[1].state,
        WorkerActivity::Reported(ReportKind::Worker(crate::worker::ReportStatus::Done))
    );
}

#[test]
fn store_workers_take_the_bare_id_from_the_id_tag() {
    let workers = store_workers("ralph-my-proj-GUZ-1\t/s\tGUZ-1\n", "/s");
    assert_eq!(
        workers,
        vec![RunningWorker {
            id: "GUZ-1".into(),
            session: "ralph-my-proj-GUZ-1".into(),
            kind: HarnessId::ClaudeTmux
        }]
    );
}

#[test]
fn running_worker_loads_explicit_kinds() {
    for (name, kind) in [
        ("claude-tmux", HarnessId::ClaudeTmux),
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
}

#[test]
fn a_record_without_kind_reads_as_headless_claude() {
    let worker: RunningWorker =
        serde_json::from_str(r#"{"id":"A","session":"s","opencode":false}"#).unwrap();
    assert_eq!(worker.kind, HarnessId::Claude);
}

#[test]
fn orphan_hint_lists_ready_ids_without_a_session() {
    let running = [RunningWorker {
        id: "A".into(),
        session: "ralph-p-A".into(),
        kind: HarnessId::ClaudeTmux,
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
            kind: HarnessId::ClaudeTmux,
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
        worker("claude", HarnessId::ClaudeTmux),
    ];
    let seen = HashMap::from([
        (
            WorkerId::parse("done").unwrap(),
            ReportKind::parse("done").unwrap(),
        ),
        (
            WorkerId::parse("redo").unwrap(),
            ReportKind::parse("done").unwrap(),
        ),
    ]);
    let active = HashSet::from(["ses_ralphbusy".to_string(), "ses_ralphredo".to_string()]);
    let states = |active| -> Vec<String> {
        worker_states(&workers, &seen, &HashSet::new(), active, &HashSet::new())
            .into_iter()
            .map(|worker| worker.state.to_string())
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
        worker("redone", HarnessId::ClaudeTmux),
        worker("quiet", HarnessId::ClaudeTmux),
        worker("checked", HarnessId::ClaudeTmux),
        worker("ask", HarnessId::ClaudeTmux),
        worker("busy", HarnessId::Opencode),
    ];
    let seen = HashMap::from([
        (
            WorkerId::parse("redone").unwrap(),
            ReportKind::parse("done").unwrap(),
        ),
        (
            WorkerId::parse("quiet").unwrap(),
            ReportKind::parse("done").unwrap(),
        ),
        (
            WorkerId::parse("checked").unwrap(),
            ReportKind::parse("check").unwrap(),
        ),
        (
            WorkerId::parse("ask").unwrap(),
            ReportKind::parse("question").unwrap(),
        ),
        (
            WorkerId::parse("busy").unwrap(),
            ReportKind::parse("done").unwrap(),
        ),
    ]);
    let sent: HashSet<String> = ["redone", "checked", "ask", "busy"]
        .map(String::from)
        .into();
    let active = HashSet::from(["ses_ralphbusy".to_string()]);
    let states: Vec<String> = worker_states(&workers, &seen, &sent, Some(&active), &HashSet::new())
        .into_iter()
        .map(|worker| worker.state.to_string())
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
    let seen = HashMap::from([(
        WorkerId::parse("w").unwrap(),
        ReportKind::parse("question").unwrap(),
    )]);
    let blocked = HashSet::from(["w".to_string()]);
    let states = worker_states(&workers, &seen, &HashSet::new(), None, &blocked);
    assert_eq!(states[0].state, WorkerActivity::Permission);
    assert!(states[0].is_running());
}

#[test]
fn latest_states_keep_only_worker_senders() {
    let watch = crate::worker::WatchId::parse("watch:ci").unwrap();
    let reports = [
        Report::watch(&watch, chrono::DateTime::UNIX_EPOCH, "x".into()),
        Report::check(
            &WorkerId::parse("A").unwrap(),
            chrono::DateTime::UNIX_EPOCH,
            "y".into(),
        ),
    ];
    let states = latest_states(&reports);
    assert_eq!(states.len(), 1);
    assert_eq!(states.get("A"), Some(&ReportKind::Check));
}
