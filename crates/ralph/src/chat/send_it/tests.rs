use super::*;
use crate::chat::socket::{respond, Shared};
use ralph_core::chat_memory::Trigger;
use ralph_core::chat_socket::SocketRequest;
use ralph_core::watch::WatchOutcome;
use ralph_server::chat::memory::Store;
use ralph_server::watches::Watches;
use ralph_server::worker_mcp::Inbox;
use ralph_server::workers::Workers;
use serde_json::json;
use std::sync::Mutex;

const LIMITS: Limits = Limits {
    max_turns: 3,
    max_usd: 1.0,
};

fn plan_args() -> Value {
    json!({
        "goal": "ship GUZ-1",
        "steps": [{"do": "run the checks", "criterion": {"kind": "check_passes", "command": "cargo xtask lint"}}]
    })
}

fn go(send_it: &SendIt, shown: Option<&str>, typed: bool) -> (String, Option<Ticked>) {
    send_it.go(
        shown,
        typed,
        &Seen {
            last: &HashMap::new(),
            running: &[],
        },
    )
}

fn proposed(dir: &Path) -> (SendIt, String) {
    let send_it = SendIt::new(dir, LIMITS);
    send_it.propose(&plan_args()).unwrap();
    let hash = send_it.status().shown.unwrap();
    (send_it, hash)
}

#[test]
fn propose_stores_the_plan_under_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let send_it = SendIt::new(dir.path(), LIMITS);
    let stored = send_it.propose(&plan_args()).unwrap();
    let text = fs::read_to_string(dir.path().join("send-it/run.toml")).unwrap();
    let file = read_run(&text, LIMITS).unwrap();
    assert_eq!(file.approved, None);
    let hash = plan_hash(&file.plan);
    assert!(
        stored.starts_with(&format!("Stored run plan {hash}.")),
        "{stored}"
    );
}

#[test]
fn propose_rejects_a_step_without_a_criterion_and_stores_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let send_it = SendIt::new(dir.path(), LIMITS);
    let args = json!({"goal": "g", "steps": [{"do": "x"}]});
    assert_eq!(
        send_it.propose(&args),
        Err("Step 1 has no exit criterion. Give each step a check ralph can measure.".into())
    );
    assert!(!dir.path().join("send-it").exists());
}

#[test]
fn propose_refuses_while_an_approved_plan_exists() {
    let dir = tempfile::tempdir().unwrap();
    let send_it = SendIt::new(dir.path(), LIMITS);
    let plan = parse_run(&plan_args(), LIMITS).unwrap();
    let hash = plan_hash(&plan);
    let draft = RunFile {
        plan,
        approved: None,
        stopped: None,
    };
    send_it.save(&draft).unwrap();
    assert!(send_it.propose(&plan_args()).is_ok());
    send_it
        .save(&RunFile {
            approved: Some(hash.clone()),
            ..draft
        })
        .unwrap();
    assert_eq!(
        send_it.propose(&plan_args()),
        Err(format!(
            "Plan {hash} is approved and active. Stop it with /send-it off first."
        ))
    );
}

#[test]
fn an_approved_run_blocks_a_new_proposal_until_off() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, hash) = proposed(dir.path());
    assert_eq!(
        go(&send_it, Some(&hash), true).0,
        format!("Plan {hash} approved. Running it.")
    );
    let refused = send_it.propose(&plan_args()).unwrap_err();
    assert!(refused.contains("approved and active"), "{refused}");
    assert_eq!(send_it.off(), "send-it off: the run plan is deleted.");
    assert!(send_it.propose(&plan_args()).is_ok());
}

#[test]
fn save_writes_the_stored_plan_as_a_template_and_overwrites_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let send_it = SendIt::new(dir.path(), LIMITS);
    assert_eq!(send_it.save_template("ship", None), NO_RUN);
    proposed(dir.path());
    let saved = send_it.save_template("ship", Some("GUZ-1"));
    assert!(saved.starts_with("Template ship saved: GUZ-1 became {ticket}."));
    let saved = || fs::read_to_string(dir.path().join("send-it/templates/ship.toml")).unwrap();
    assert!(saved().contains("goal = \"ship {ticket}\""));
    let replaced = send_it.save_template("ship", None);
    assert!(replaced.starts_with("Template ship replaced. No ticket given"));
    assert!(saved().contains("goal = \"ship GUZ-1\""));
    assert!(send_it.save_template("../ship", None).contains("dashes"));
    assert!(!dir.path().join("send-it/ship.toml").exists());
}

#[test]
fn templates_lists_the_saved_names_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let send_it = SendIt::new(dir.path(), LIMITS);
    assert!(send_it.templates().starts_with("No templates."));
    proposed(dir.path());
    send_it.save_template("ship", None);
    send_it.save_template("review", None);
    fs::write(dir.path().join("send-it/templates/notes.txt"), "x").unwrap();
    assert_eq!(
        send_it.templates(),
        "Templates: review, ship. Use one with /send-it <ticket> <name>."
    );
    assert!(send_it.template_use("GUZ-7 ship").is_some());
    assert_eq!(send_it.template_use("GUZ-7 notes"), None);
}

#[test]
fn using_a_template_stores_an_unapproved_plan_and_shows_its_hash() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, hash) = proposed(dir.path());
    send_it.save_template("ship", Some("GUZ-1"));
    go(&send_it, Some(&hash), true);
    let refused = send_it.use_template("GUZ-2", "ship");
    assert!(refused.text.contains("approved and active"));
    assert_eq!(refused.shown, None);
    send_it.off();
    let screen = send_it.use_template("GUZ-2", "ship");
    let file = send_it.load().unwrap().unwrap();
    assert_eq!(file.plan.goal, "ship GUZ-2");
    assert_eq!(file.approved, None);
    assert_eq!(screen, send_it.status());
    let missing = send_it.use_template("GUZ-2", "gone");
    assert!(missing.text.starts_with("Could not read"));
}

#[test]
fn go_needs_the_prompt_and_the_hash_on_screen() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, hash) = proposed(dir.path());
    assert!(go(&send_it, Some(&hash), false)
        .0
        .contains("type it at the prompt"));
    assert!(go(&send_it, None, true).0.contains("/send-it status"));
    assert!(send_it
        .status()
        .text
        .ends_with("not approved. Type /send-it go to approve this plan."));
    go(&send_it, Some(&hash), true);
    assert!(send_it
        .status()
        .text
        .ends_with(&format!("Plan {hash}: approved.")));
}

#[test]
fn clearing_after_an_edit_drops_the_approval_and_shows_the_plan() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, hash) = proposed(dir.path());
    go(&send_it, Some(&hash), true);
    let screen = send_it.clear_approval();
    assert_eq!(screen.shown, Some(hash));
    assert!(screen
        .text
        .ends_with("not approved. Type /send-it go to approve this plan."));
    fs::write(&send_it.path, "goal = ").unwrap();
    let broken = send_it.clear_approval();
    assert_eq!(broken.shown, None);
    assert!(
        broken.text.ends_with("Fix it with /send-it edit."),
        "{}",
        broken.text
    );
}

#[test]
fn without_a_run_every_command_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let send_it = SendIt::new(dir.path(), LIMITS);
    let none = Screen {
        text: NO_RUN.to_string(),
        shown: None,
    };
    assert_eq!(send_it.status(), none);
    assert_eq!(send_it.clear_approval(), none);
    assert_eq!(go(&send_it, Some("x"), true), (NO_RUN.to_string(), None));
    assert_eq!(send_it.off(), NO_RUN);
    assert!(NO_RUN.starts_with("No send-it run."));
}

#[test]
fn a_turn_that_stores_no_new_plan_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let send_it = SendIt::new(dir.path(), LIMITS);
    assert_eq!(send_it.proposed(None).text, NOT_PROPOSED);
    let (send_it, hash) = proposed(dir.path());
    assert_eq!(send_it.proposed(Some(hash.clone())).text, NOT_PROPOSED);
    assert_eq!(send_it.proposed(None).shown, Some(hash));
}

const STEP_1: &str = "[send-it 1/1] run the checks · exit: `cargo xtask lint` exits 0";

fn started(dir: &Path) -> (SendIt, Active) {
    let (send_it, hash) = proposed(dir);
    let ticked = go(&send_it, Some(&hash), true).1.unwrap();
    let active = Active {
        driving: ticked.driving,
        watched: None,
    };
    (send_it, active)
}

fn ended() -> Ended<'static> {
    Ended {
        file: None,
        replied: true,
        asked: false,
        typed: false,
        queued: &[],
        cost_usd: 0.1,
        step_result: None,
        drained: &[],
        unread: &[],
        answered: &[],
        watched: None,
        refused: None,
    }
}

fn judging(dir: &Path) -> (SendIt, Active) {
    let send_it = SendIt::new(dir, LIMITS);
    let mut args = plan_args();
    args["steps"] =
        json!([{"do": "review", "criterion": {"kind": "judge", "question": "all fixed?"}}]);
    send_it.propose(&args).unwrap();
    let hash = send_it.status().shown.unwrap();
    let started = go(&send_it, Some(&hash), true).1.unwrap();
    let ticked = send_it.tick(
        &Active {
            driving: started.driving,
            watched: None,
        },
        ended(),
    );
    assert_eq!(ticked.next, Next::Judge);
    let active = Active {
        driving: ticked.driving,
        watched: None,
    };
    (send_it, active)
}

#[test]
fn the_next_tick_takes_the_recorded_step_result_once() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = judging(dir.path());
    send_it.record(StepResult {
        met: true,
        evidence: "all 3 fixed".to_string(),
    });
    let ticked = send_it.tick(&active, ended());
    assert_eq!(ticked.next, Next::Done);
    assert_eq!(
        ticked.met.as_deref(),
        Some("send-it: step 1/1 met (judged: all fixed?): all 3 fixed")
    );
    let Next::Send(line) = send_it.tick(&active, ended()).next else {
        panic!("a second tick must not see the old result")
    };
    assert!(line.contains("judge: not met: "), "{line}");
}

fn watches(dir: &std::path::Path) -> Watches {
    Watches::new(
        std::sync::Arc::new(Inbox::new(dir.join("inbox"))),
        dir,
        std::time::Duration::from_secs(60),
    )
}

#[test]
fn the_socket_records_a_step_result_only_in_a_send_it_turn() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = judging(dir.path());
    let shared = Shared {
        store: Store::new(dir.path().to_path_buf()),
        workers: Workers::new("/s".into(), "/bin/ralph".into(), "auto".into(), None),
        send_it: send_it.clone(),
        watches: watches(dir.path()),
    };
    let call = |args: Value| {
        respond(
            &shared,
            &SocketRequest {
                tool: "step_result".to_string(),
                args,
            },
        )
    };
    let met = json!({"met": true, "evidence": "all 3 fixed"});
    shared.store.set_pending_turn(Trigger::User).unwrap();
    assert!(call(met.clone()).is_error);
    shared.store.set_pending_turn(Trigger::SendIt).unwrap();
    assert!(call(json!({"met": true})).is_error);
    let recorded = call(met);
    assert!(!recorded.is_error, "{}", recorded.text);
    assert_eq!(send_it.tick(&active, ended()).next, Next::Done);
}

#[test]
fn only_an_approved_plan_lends_its_allowed_list() {
    let dir = tempfile::tempdir().unwrap();
    let send_it = SendIt::new(dir.path(), LIMITS);
    assert_eq!(send_it.allowed(), None);
    let mut args = plan_args();
    args["allowed"] = json!([{"kind": "reply"}, {"kind": "watch"}]);
    send_it.propose(&args).unwrap();
    assert_eq!(send_it.allowed(), None);
    let hash = send_it.status().shown.unwrap();
    go(&send_it, Some(&hash), true);
    assert_eq!(send_it.allowed(), Some(vec![Action::Reply, Action::Watch]));
}

#[test]
fn the_next_tick_takes_the_answered_workers_once() {
    let dir = tempfile::tempdir().unwrap();
    let send_it = SendIt::new(dir.path(), LIMITS);
    let mut args = plan_args();
    args["allowed"] = json!([{"kind": "reply"}]);
    send_it.propose(&args).unwrap();
    let hash = send_it.status().shown.unwrap();
    let active = Active {
        driving: go(&send_it, Some(&hash), true).1.unwrap().driving,
        watched: None,
    };
    let flagged = [ralph_core::worker::Report {
        worker: "E1".to_string(),
        at: "2026-01-01T00:00:00Z".to_string(),
        status: "question".to_string(),
        text: "q".to_string(),
    }];
    let with_question = || Ended {
        drained: &flagged,
        ..ended()
    };
    send_it.answer("E1".to_string());
    assert_eq!(
        send_it.tick(&active, with_question()).next,
        Next::Check("cargo xtask lint".to_string())
    );
    assert_eq!(
        send_it.tick(&active, with_question()).next,
        Next::Stop(StopReason::WorkerFlagged)
    );
}

#[test]
fn a_refusal_from_the_socket_stops_the_next_tick_once_and_keeps_the_approval() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = started(dir.path());
    send_it.clone().refuse("spawn_worker codex".to_string());
    assert_eq!(
        send_it.tick(&active, ended()).next,
        Next::Stop(StopReason::Refused {
            tool: "spawn_worker codex".to_string()
        })
    );
    assert_eq!(
        send_it.tick(&active, ended()).next,
        Next::Check("cargo xtask lint".to_string())
    );
    assert!(send_it.load().unwrap().unwrap().is_approved());
}

#[test]
fn a_worker_turn_reply_the_run_does_not_allow_is_refused_and_stops_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let shared = Shared {
        store: Store::new(dir.path().to_path_buf()),
        workers: Workers::new("/s".into(), "/bin/ralph".into(), "auto".into(), None),
        send_it: SendIt::new(dir.path(), LIMITS),
        watches: watches(dir.path()),
    };
    let mut args = plan_args();
    args["allowed"] = json!([{"kind": "watch"}]);
    shared.send_it.propose(&args).unwrap();
    let hash = shared.send_it.status().shown.unwrap();
    let active = Active {
        driving: go(&shared.send_it, Some(&hash), true).1.unwrap().driving,
        watched: None,
    };
    shared.store.set_pending_turn(Trigger::Worker).unwrap();
    let replied = respond(
        &shared,
        &SocketRequest {
            tool: "reply".to_string(),
            args: json!({"id": "E1", "text": "yes, merge it"}),
        },
    );
    assert!(replied.is_error);
    assert_eq!(
        replied.text,
        "The approved run does not allow `reply`. The run stops; ralph tells the person."
    );
    assert_eq!(
        shared.send_it.tick(&active, ended()).next,
        Next::Stop(StopReason::Refused {
            tool: "reply".to_string()
        })
    );
}

#[test]
fn the_socket_runs_a_listed_tool_and_a_refused_one_stops_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let shared = Shared {
        store: Store::new(dir.path().to_path_buf()),
        workers: Workers::new("/s".into(), "/bin/ralph".into(), "auto".into(), None),
        send_it: SendIt::new(dir.path(), LIMITS),
        watches: watches(dir.path()),
    };
    let mut args = plan_args();
    args["allowed"] = json!([{"kind": "watch"}]);
    shared.send_it.propose(&args).unwrap();
    let hash = shared.send_it.status().shown.unwrap();
    let driving = go(&shared.send_it, Some(&hash), true).1.unwrap().driving;
    let active = Active {
        driving,
        watched: None,
    };
    shared.store.set_pending_turn(Trigger::SendIt).unwrap();
    let call = |tool: &str, args: Value| {
        respond(
            &shared,
            &SocketRequest {
                tool: tool.to_string(),
                args,
            },
        )
    };
    let watched = call(
        "watch",
        json!({"id": "ci", "command": "c", "follow_up": "f"}),
    );
    assert!(!watched.is_error, "{}", watched.text);
    let spawned = call(
        "spawn_worker",
        json!({"id": "A", "task": "t", "harness": "codex"}),
    );
    assert!(spawned.is_error);
    assert!(
        spawned.text.contains("`spawn_worker codex`"),
        "{}",
        spawned.text
    );
    assert_eq!(
        shared.send_it.tick(&active, ended()).next,
        Next::Stop(StopReason::Refused {
            tool: "spawn_worker codex".to_string()
        })
    );
}

#[test]
fn go_starts_only_the_approved_plan_at_step_one() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, hash) = proposed(dir.path());
    assert!(go(&send_it, None, true).1.is_none());
    let ticked = go(&send_it, Some(&hash), true).1.unwrap();
    let Next::Send(brief) = ticked.next else {
        panic!("{:?}", ticked.next)
    };
    assert!(brief.ends_with(STEP_1), "{brief}");
    assert!(brief.starts_with("send-it run "), "{brief}");
}

#[test]
fn release_drops_the_run_s_approval_and_keeps_the_plan_for_save_and_reuse() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = started(dir.path());
    let hash = active.driving.hash.clone();
    assert_eq!(send_it.release("000000000000", Some(1)), Ok(None));
    assert!(send_it.load().unwrap().unwrap().is_approved());
    send_it.release(&hash, None).unwrap();
    let file = send_it.load().unwrap().unwrap();
    assert_eq!(file.approved, None);
    assert_eq!(plan_hash(&file.plan), hash);
    assert!(send_it
        .status()
        .text
        .ends_with("not approved. Type /send-it go to approve this plan."));
    assert!(send_it
        .save_template("ship", None)
        .starts_with("Template ship saved"));
    assert!(send_it.use_template("GUZ-2", "ship").shown.is_some());
    assert!(send_it.propose(&plan_args()).is_ok());
    send_it.release(&hash, None).unwrap();
    send_it.off();
    assert_eq!(send_it.release(&hash, Some(1)), Ok(None));
}

#[test]
fn a_stop_keeps_the_plan_until_off_and_go_resumes_at_the_stopped_step() {
    let dir = tempfile::tempdir().unwrap();
    let send_it = SendIt::new(dir.path(), LIMITS);
    let args = json!({
        "goal": "ship GUZ-1",
        "steps": [
            {"do": "spawn the worker", "criterion": {"kind": "worker_done", "worker": "GUZ-1"}},
            {"do": "run the checks", "criterion": {"kind": "check_passes", "command": "cargo xtask lint"}}
        ]
    });
    send_it.propose(&args).unwrap();
    let hash = send_it.status().shown.unwrap();
    let first = go(&send_it, Some(&hash), true).1.unwrap();
    let stopped = send_it.release(&hash, Some(2)).unwrap().unwrap();
    assert_eq!(stopped.approved, None);
    assert_eq!(send_it.load().unwrap(), Some(stopped));
    let screen = send_it.status();
    assert!(
        screen
            .text
            .ends_with("stopped at step 2/2. Type /send-it go to resume from step 2."),
        "{}",
        screen.text
    );
    let resumed = go(&send_it, screen.shown.as_deref(), true).1.unwrap();
    assert_eq!(first.driving.progress.step, 0);
    assert_eq!(resumed.driving.progress.step, 1);
    assert_eq!(resumed.next, Next::Check("cargo xtask lint".to_string()));
    send_it.release(&hash, None).unwrap();
    assert_eq!(send_it.load().unwrap().unwrap().stopped, None);
    assert_eq!(send_it.off(), "send-it off: the run plan is deleted.");
    assert_eq!(send_it.status().text, NO_RUN);
}

#[test]
fn go_on_a_stopped_worker_step_skips_it_once_the_worker_reported_done() {
    let dir = tempfile::tempdir().unwrap();
    let send_it = SendIt::new(dir.path(), LIMITS);
    let args = json!({
        "goal": "ship GUZ-1",
        "steps": [
            {"do": "spawn the worker", "criterion": {"kind": "worker_done", "worker": "GUZ-1"}},
            {"do": "run the checks", "criterion": {"kind": "check_passes", "command": "cargo xtask lint"}}
        ]
    });
    send_it.propose(&args).unwrap();
    let hash = send_it.status().shown.unwrap();
    go(&send_it, Some(&hash), true);
    send_it.release(&hash, Some(1)).unwrap();
    let last = HashMap::from([("GUZ-1".to_string(), "done".to_string())]);
    let seen = Seen {
        last: &last,
        running: &[],
    };
    let resumed = send_it.go(Some(&hash), true, &seen).1.unwrap();
    assert_eq!(
        resumed.met.as_deref(),
        Some("send-it: step 1/2 met (worker GUZ-1 reports done)")
    );
    let Next::Send(brief) = resumed.next else {
        panic!("{:?}", resumed.next)
    };
    assert!(
        brief.ends_with("[send-it 2/2] run the checks · exit: `cargo xtask lint` exits 0"),
        "{brief}"
    );
}

#[test]
fn every_tick_reads_the_plan_file_again() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = started(dir.path());
    assert_eq!(
        send_it.tick(&active, ended()).next,
        Next::Check("cargo xtask lint".to_string())
    );
    send_it.clear_approval();
    assert_eq!(
        send_it.tick(&active, ended()).next,
        Next::Stop(StopReason::PlanChanged)
    );
    send_it.off();
    assert_eq!(
        send_it.tick(&active, ended()).next,
        Next::Stop(StopReason::Off)
    );
}

#[test]
fn a_tick_takes_the_outcome_the_watch_left_in_its_slot() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = started(dir.path());
    let waiting = send_it.tick(&active, ended()).driving;
    for (outcome, next) in [
        (WatchOutcome::Exited(0), Next::Done),
        (
            WatchOutcome::Exited(1),
            Next::Stop(StopReason::CheckFailed { step: 1 }),
        ),
    ] {
        let active = Active {
            driving: waiting.clone(),
            watched: Some(OutcomeSlot::new(Mutex::new(Some(outcome)))),
        };
        assert_eq!(send_it.tick(&active, ended()).next, next);
    }
}
