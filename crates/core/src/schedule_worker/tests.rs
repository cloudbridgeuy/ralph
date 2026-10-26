use super::*;

fn utc(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

fn worker(id: &str, at: &str, status: TaskStatus) -> Task {
    Task {
        id: id.to_string(),
        kind: TaskKind::Worker {
            brief: "do it".to_string(),
        },
        follow_up: None,
        at: utc(at),
        timeout_secs: Some(3600),
        status,
        late: false,
        deadline: None,
    }
}

fn running(id: &str, started: &str) -> Task {
    Task {
        deadline: Some(utc(started) + ChronoDuration::seconds(3600)),
        ..worker(id, started, TaskStatus::Running)
    }
}

fn report(worker: &str, at: &str, status: &str) -> Report {
    Report {
        worker: worker.to_string(),
        at: at.to_string(),
        status: status.to_string(),
        text: "t".to_string(),
    }
}

fn alive(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| id.to_string()).collect()
}

const TICK: Duration = Duration::from_millis(500);

#[test]
fn ids_are_prefixed_and_sources_match_both_spellings() {
    assert_eq!(worker_id("nightly"), "sched-nightly");
    assert!(source_matches_worker("sched-a", "sched-a"));
    assert!(source_matches_worker("schedule:a", "sched-a"));
    assert!(!source_matches_worker("schedule:a", "sched-b"));
    assert!(!source_matches_worker("schedule:", "sched-"));
    assert!(!source_matches_worker("a", "sched-a"));
    assert!(!source_matches_worker("schedule:a", "a"));
}

#[test]
fn worker_tasks_are_told_from_commands() {
    let mut task = worker("a", "2026-01-01T10:00:00Z", TaskStatus::Pending);
    assert!(is_worker_task(&task));
    task.kind = TaskKind::Command {
        command: "x".to_string(),
    };
    assert!(!is_worker_task(&task));
}

#[test]
fn the_deadline_uses_the_timeout_or_the_default() {
    let now = utc("2026-01-01T10:00:00Z");
    let mut task = worker("a", "2026-01-01T10:00:00Z", TaskStatus::Pending);
    assert_eq!(worker_deadline(&task, now), utc("2026-01-01T11:00:00Z"));
    task.timeout_secs = None;
    assert_eq!(worker_deadline(&task, now), utc("2026-01-01T12:00:00Z"));
}

#[test]
fn the_brief_carries_a_late_notice_only_when_late() {
    let now = utc("2026-01-01T12:05:00Z");
    let offset = FixedOffset::east_opt(0).unwrap();
    let mut task = worker("a", "2026-01-01T10:30:00Z", TaskStatus::Pending);
    assert_eq!(worker_brief(&task, now, offset).unwrap(), "do it");
    task.late = true;
    assert_eq!(
        worker_brief(&task, now, offset).unwrap(),
        "Ran late: due 10:30, started 12:05.\n\ndo it"
    );
    task.kind = TaskKind::Command {
        command: "x".to_string(),
    };
    assert_eq!(worker_brief(&task, now, offset), None);
}

#[test]
fn a_due_worker_fires_and_a_future_one_waits() {
    let tasks = [
        worker("due", "2026-01-01T09:59:59Z", TaskStatus::Pending),
        worker("later", "2026-01-01T10:00:01Z", TaskStatus::Pending),
        worker("proposed", "2026-01-01T09:00:00Z", TaskStatus::Proposed),
    ];
    let plan = plan_worker_tick(&tasks, &[], &[], utc("2026-01-01T10:00:00Z"), TICK);
    assert_eq!(plan.fire.len(), 1);
    assert_eq!(plan.fire[0].task.id, "due");
    assert!(plan.ended.is_empty());
}

#[test]
fn the_cap_counts_running_workers_and_keeps_the_rest_pending() {
    let tasks = [
        running("r1", "2026-01-01T09:30:00Z"),
        worker("p1", "2026-01-01T09:50:00Z", TaskStatus::Pending),
        worker("p2", "2026-01-01T09:40:00Z", TaskStatus::Pending),
    ];
    let plan = plan_worker_tick(
        &tasks,
        &[],
        &alive(&["sched-r1"]),
        utc("2026-01-01T10:00:00Z"),
        TICK,
    );
    let fired: Vec<&str> = plan.fire.iter().map(|f| f.task.id.as_str()).collect();
    assert_eq!(fired, ["p2"]);
}

#[test]
fn a_full_cap_fires_nothing() {
    let tasks = [
        running("r1", "2026-01-01T09:30:00Z"),
        running("r2", "2026-01-01T09:31:00Z"),
        worker("p", "2026-01-01T09:40:00Z", TaskStatus::Pending),
    ];
    let plan = plan_worker_tick(
        &tasks,
        &[],
        &alive(&["sched-r1", "sched-r2"]),
        utc("2026-01-01T10:00:00Z"),
        TICK,
    );
    assert!(plan.fire.is_empty());
}

#[test]
fn an_ending_worker_frees_its_slot_in_the_same_tick() {
    let tasks = [
        running("r1", "2026-01-01T09:30:00Z"),
        running("r2", "2026-01-01T09:31:00Z"),
        worker("p", "2026-01-01T09:40:00Z", TaskStatus::Pending),
    ];
    let reports = [report("sched-r1", "2026-01-01T09:45:00Z", "done")];
    let plan = plan_worker_tick(
        &tasks,
        &reports,
        &alive(&["sched-r1", "sched-r2"]),
        utc("2026-01-01T10:00:00Z"),
        TICK,
    );
    assert_eq!(plan.ended.len(), 1);
    assert_eq!(plan.ended[0].0.id, "r1");
    assert_eq!(plan.fire.len(), 1);
}

#[test]
fn a_start_within_one_tick_is_on_time_and_later_is_late() {
    let tasks = [
        worker("a", "2026-01-01T09:59:59.700Z", TaskStatus::Pending),
        worker("b", "2026-01-01T09:59:58Z", TaskStatus::Pending),
    ];
    let plan = plan_worker_tick(&tasks, &[], &[], utc("2026-01-01T10:00:00Z"), TICK);
    let lateness: Vec<(&str, bool)> = plan
        .fire
        .iter()
        .map(|f| (f.task.id.as_str(), f.late))
        .collect();
    assert_eq!(lateness, [("b", true), ("a", false)]);
}

#[test]
fn an_already_late_task_stays_late() {
    let mut task = worker("a", "2026-01-01T10:00:00Z", TaskStatus::Pending);
    task.late = true;
    let tasks = [task];
    let plan = plan_worker_tick(&tasks, &[], &[], utc("2026-01-01T10:00:00Z"), TICK);
    assert!(plan.fire[0].late);
}

#[test]
fn reports_end_a_run_by_their_status() {
    let tasks = [running("a", "2026-01-01T09:00:00Z")];
    let now = utc("2026-01-01T09:30:00Z");
    for (status, end) in [
        ("done", WorkerEnd::Done),
        ("blocked", WorkerEnd::Attention),
        ("question", WorkerEnd::Attention),
    ] {
        let reports = [report("sched-a", "2026-01-01T09:20:00Z", status)];
        let plan = plan_worker_tick(&tasks, &reports, &alive(&["sched-a"]), now, TICK);
        assert_eq!(plan.ended.len(), 1, "{status}");
        assert_eq!(plan.ended[0].1, end, "{status}");
    }
}

#[test]
fn the_latest_report_wins() {
    let tasks = [running("a", "2026-01-01T09:00:00Z")];
    let reports = [
        report("sched-a", "2026-01-01T09:10:00Z", "question"),
        report("sched-a", "2026-01-01T09:20:00Z", "done"),
    ];
    let plan = plan_worker_tick(
        &tasks,
        &reports,
        &alive(&["sched-a"]),
        utc("2026-01-01T09:30:00Z"),
        TICK,
    );
    assert_eq!(plan.ended[0].1, WorkerEnd::Done);
}

#[test]
fn old_foreign_and_non_report_posts_do_not_end_a_run() {
    let tasks = [running("a", "2026-01-01T09:00:00Z")];
    let reports = [
        report("sched-a", "2026-01-01T08:59:59Z", "done"),
        report("sched-b", "2026-01-01T09:20:00Z", "done"),
        report("schedule:a", "2026-01-01T09:20:00Z", "done"),
        report("sched-a", "2026-01-01T09:20:00Z", "check"),
        report("sched-a", "not a time", "done"),
    ];
    let plan = plan_worker_tick(
        &tasks,
        &reports,
        &alive(&["sched-a"]),
        utc("2026-01-01T09:30:00Z"),
        TICK,
    );
    assert!(plan.ended.is_empty());
}

#[test]
fn a_run_past_its_deadline_times_out() {
    let tasks = [running("a", "2026-01-01T09:00:00Z")];
    let alive = alive(&["sched-a"]);
    let before = plan_worker_tick(&tasks, &[], &alive, utc("2026-01-01T09:59:59Z"), TICK);
    assert!(before.ended.is_empty());
    let at = plan_worker_tick(&tasks, &[], &alive, utc("2026-01-01T10:00:00Z"), TICK);
    assert_eq!(at.ended[0].1, WorkerEnd::TimedOut);
}

#[test]
fn a_report_beats_the_deadline() {
    let tasks = [running("a", "2026-01-01T09:00:00Z")];
    let reports = [report("sched-a", "2026-01-01T09:50:00Z", "done")];
    let plan = plan_worker_tick(
        &tasks,
        &reports,
        &alive(&["sched-a"]),
        utc("2026-01-01T10:30:00Z"),
        TICK,
    );
    assert_eq!(plan.ended[0].1, WorkerEnd::Done);
}

#[test]
fn a_missing_harness_vanishes_only_after_the_startup_grace() {
    let tasks = [running("a", "2026-01-01T09:00:00Z")];
    let young = plan_worker_tick(&tasks, &[], &[], utc("2026-01-01T09:00:59Z"), TICK);
    assert!(young.ended.is_empty());
    let old = plan_worker_tick(&tasks, &[], &[], utc("2026-01-01T09:01:00Z"), TICK);
    assert_eq!(old.ended[0].1, WorkerEnd::Vanished);
}

#[test]
fn a_live_harness_never_vanishes() {
    let tasks = [running("a", "2026-01-01T09:00:00Z")];
    let plan = plan_worker_tick(
        &tasks,
        &[],
        &alive(&["sched-a"]),
        utc("2026-01-01T09:30:00Z"),
        TICK,
    );
    assert!(plan.ended.is_empty());
}

#[test]
fn command_tasks_are_left_to_the_watch_path() {
    let mut task = worker("a", "2026-01-01T09:00:00Z", TaskStatus::Pending);
    task.kind = TaskKind::Command {
        command: "x".to_string(),
    };
    let mut live = task.clone();
    live.status = TaskStatus::Running;
    let tasks = [task, live];
    let plan = plan_worker_tick(&tasks, &[], &[], utc("2026-01-01T10:00:00Z"), TICK);
    assert!(plan.fire.is_empty() && plan.ended.is_empty());
}

#[test]
fn chat_reports_keep_the_schedule_tag() {
    let report = chat_report("a", "text".to_string(), "2026-01-01T10:00:00Z");
    assert_eq!(report.worker, "schedule:a");
    assert_eq!(report.status, WATCH_STATUS);
    assert_eq!(report.text, "text");
}

#[test]
fn report_texts_name_the_task_and_the_kept_branch() {
    let task = worker("a", "2026-01-01T10:00:00Z", TaskStatus::Running);
    assert_eq!(
        start_failure_text("a", "boom"),
        "Scheduled worker `a` did not start: boom."
    );
    assert_eq!(
        timed_out_text(&task, None),
        "Scheduled worker `a` timed out after 1h. Its harness was stopped. Worktree and branch worker/sched-a are kept."
    );
    assert_eq!(
        timed_out_text(&task, Some("tmux gone")),
        "Scheduled worker `a` timed out after 1h. Its harness could not be stopped: tmux gone. Worktree and branch worker/sched-a are kept."
    );
    assert_eq!(
        vanished_text(&task),
        "Scheduled worker `a` stopped without a report. Worktree and branch worker/sched-a are kept."
    );
}

#[test]
fn only_new_scheduled_worker_reports_are_kept() {
    let known = [report("sched-a", "t1", "done")];
    let incoming = [
        report("sched-a", "t1", "done"),
        report("sched-a", "t2", "question"),
        report("sched-a", "t2", "question"),
        report("sched-a", "t3", "check"),
        report("other", "t4", "done"),
        report("schedule:a", "t5", "done"),
    ];
    assert_eq!(
        new_scheduled_reports(&known, &incoming),
        [report("sched-a", "t2", "question")]
    );
}

#[test]
fn a_starting_task_counts_as_alive_under_its_worker_id() {
    assert_eq!(
        alive_with_starting(&alive(&["sched-a"]), &alive(&["b"])),
        alive(&["sched-a", "sched-b"])
    );
    assert!(alive_with_starting(&[], &[]).is_empty());
}

#[test]
fn a_slow_start_does_not_vanish_while_it_is_starting() {
    let tasks = [running("a", "2026-01-01T09:00:00Z")];
    let alive = alive_with_starting(&[], &alive(&["a"]));
    let plan = plan_worker_tick(&tasks, &[], &alive, utc("2026-01-01T09:05:00Z"), TICK);
    assert!(plan.ended.is_empty());
}

#[test]
fn only_a_running_record_with_that_id_is_still_running() {
    let tasks = [
        running("a", "2026-01-01T09:00:00Z"),
        worker("b", "2026-01-01T09:00:00Z", TaskStatus::Pending),
    ];
    assert!(is_still_running(&tasks, "a"));
    assert!(!is_still_running(&tasks, "b"));
    assert!(!is_still_running(&tasks, "c"));
    assert!(!is_still_running(&[], "a"));
}

#[test]
fn the_sched_prefix_marks_a_scheduled_worker_id() {
    assert!(is_scheduled_worker_id("sched-a"));
    assert!(!is_scheduled_worker_id("GUZ-1"));
    assert!(!is_scheduled_worker_id("scheduled"));
}

#[test]
fn a_reply_to_a_scheduled_worker_is_refused_while_its_record_exists() {
    let tasks = [worker("a", "2026-01-01T09:00:00Z", TaskStatus::Pending)];
    let refusal = reply_refusal("sched-a", &tasks).unwrap();
    assert!(refusal.contains("`a`"));
    assert!(refusal.contains("/schedule cancel a"));
    assert_eq!(reply_refusal("sched-b", &tasks), None);
    assert_eq!(reply_refusal("sched-a", &[]), None);
    assert_eq!(reply_refusal("a", &tasks), None);
}
