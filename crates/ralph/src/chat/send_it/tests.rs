use super::*;
use ralph_core::chat_memory::{Role, Trigger};
use ralph_core::chat_send_it::{
    plan_hash, write_run, Action, Awaiting, Limits, Refusal, Seen, StepResult, Trust, VerifyMode,
};
use ralph_core::chat_socket::SocketRequest;
use ralph_core::watch::WatchOutcome;
use ralph_core::{chat_local::notice_record, worker::ClaudeMode};
use ralph_server::chat::memory::Store;
use ralph_server::chat::send_it::{Screen, NOT_PROPOSED, NO_RUN};
use ralph_server::chat::session::{Emit, Event};
use ralph_server::chat::socket::serve::{respond, Shared};
use ralph_server::watches::Watches;
use ralph_server::worker_mcp::Inbox;
use ralph_server::workers::Workers;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub(in crate::chat) mod held;
mod screen;

const LIMITS: Limits = Limits {
    max_turns: 3,
    max_usd: 1.0,
    verify: VerifyMode::Off,
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
            last: &Default::default(),
            running: &[],
        },
    )
}

fn proposed(dir: &Path) -> (SendIt, String) {
    let send_it = quiet(dir);
    send_it.propose(&plan_args()).unwrap();
    let hash = send_it.status(80).shown.unwrap();
    (send_it, hash)
}

#[test]
fn go_stores_a_signed_record_for_the_shown_plan() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, hash) = proposed(dir.path());
    go(&send_it, Some(&hash), true);
    let approval = send_it.load().unwrap().unwrap().approved.unwrap();
    assert_eq!(approval.mac.len(), 64);
    assert!(approval.subject_hash.starts_with(&hash));
    assert_eq!(approval.peer_uid, ralph_server::wire::store::current_uid());
    assert_eq!(approval.nonce.len(), 32);
}

fn run_path(dir: &Path) -> PathBuf {
    dir.join("send-it/run.toml")
}

fn tampered(text: &str) -> String {
    let at = text.find("mac = \"").unwrap() + 7;
    let flipped = if text.as_bytes()[at] == b'0' {
        "1"
    } else {
        "0"
    };
    format!("{}{flipped}{}", &text[..at], &text[at + 1..])
}

#[test]
fn a_hand_written_approved_hash_is_not_an_approval() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, hash) = proposed(dir.path());
    let text = fs::read_to_string(run_path(dir.path())).unwrap();
    fs::write(
        run_path(dir.path()),
        format!("approved = \"{hash}\"\n{text}"),
    )
    .unwrap();
    assert_eq!(send_it.allowed(), None);
    assert_eq!(send_it.refusal(), None);
    assert!(send_it.load().unwrap().unwrap().approved.is_some());
}

#[test]
fn a_record_copied_back_after_off_is_not_an_approval() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, hash) = proposed(dir.path());
    go(&send_it, Some(&hash), true);
    let saved = fs::read_to_string(run_path(dir.path())).unwrap();
    assert!(send_it.allowed().is_some());
    send_it.off();
    send_it.propose(&plan_args()).unwrap();
    fs::write(run_path(dir.path()), &saved).unwrap();
    assert_eq!(send_it.allowed(), None);
}

#[test]
fn a_record_copied_into_a_fresh_start_is_not_an_approval() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, hash) = proposed(dir.path());
    go(&send_it, Some(&hash), true);
    let saved = fs::read_to_string(run_path(dir.path())).unwrap();
    let restarted = quiet(dir.path());
    fs::write(run_path(dir.path()), saved).unwrap();
    assert_eq!(restarted.allowed(), None);
}

#[test]
fn propose_run_arguments_cannot_carry_an_approval() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, hash) = proposed(dir.path());
    go(&send_it, Some(&hash), true);
    let record = send_it.load().unwrap().unwrap().approved.unwrap();
    send_it.off();
    let mut args = plan_args();
    args["approved"] = json!({
        "kind": record.kind, "subject_hash": record.subject_hash, "at": record.at,
        "peer_uid": record.peer_uid, "nonce": record.nonce, "mac": record.mac,
    });
    send_it.propose(&args).unwrap();
    assert!(send_it.load().unwrap().unwrap().approved.is_none());
    assert_eq!(send_it.allowed(), None);
    args["approved"] = json!(hash);
    send_it.propose(&args).unwrap();
    assert_eq!(send_it.allowed(), None);
}

#[test]
fn go_not_typed_at_the_prompt_stores_no_approval() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, hash) = proposed(dir.path());
    let (text, ticked) = go(&send_it, Some(&hash), false);
    assert!(text.contains("type it at the prompt"), "{text}");
    assert!(ticked.is_none());
    assert!(send_it.load().unwrap().unwrap().approved.is_none());
    assert_eq!(send_it.allowed(), None);
}

#[test]
fn a_tampered_record_mid_run_stops_the_run_and_closes_the_gate() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = started(dir.path());
    assert!(send_it.allowed().is_some());
    let text = fs::read_to_string(run_path(dir.path())).unwrap();
    fs::write(run_path(dir.path()), tampered(&text)).unwrap();
    assert_eq!(send_it.allowed(), None);
    assert_eq!(
        tick(&send_it, &active, ended()).next,
        Next::Stop(StopReason::PlanChanged(Some(Refusal::Tampered)))
    );
}

#[test]
fn a_swapped_plan_under_the_old_record_mid_run_stops_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = started(dir.path());
    let mut swapped = send_it.load().unwrap().unwrap();
    swapped.plan.goal = "something else".to_string();
    fs::write(run_path(dir.path()), write_run(&swapped).unwrap()).unwrap();
    assert_eq!(send_it.allowed(), None);
    assert_eq!(
        tick(&send_it, &active, ended()).next,
        Next::Stop(StopReason::PlanChanged(Some(Refusal::WrongPlan)))
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
    let send_it = quiet(dir.path());
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
    let send_it = quiet(dir.path());
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
    let refused = send_it.use_template("GUZ-2", "ship", 80);
    assert!(refused.text.contains("approved and active"));
    assert_eq!(refused.shown, None);
    send_it.off();
    let screen = send_it.use_template("GUZ-2", "ship", 80);
    let file = send_it.load().unwrap().unwrap();
    assert_eq!(file.plan.goal, "ship GUZ-2");
    assert_eq!(file.approved, None);
    assert_eq!(screen, send_it.status(80));
    let missing = send_it.use_template("GUZ-2", "gone", 80);
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
        .status(80)
        .text
        .ends_with(&format!("{hash} · not approved · /send-it go to approve")));
    go(&send_it, Some(&hash), true);
    assert!(send_it
        .status(80)
        .text
        .ends_with(&format!("{hash} · approved")));
}

#[test]
fn the_status_screen_has_a_plain_text_and_a_styled_copy_of_it() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, _) = proposed(dir.path());
    let screen = send_it.status(80);
    assert!(screen.styled.contains('\x1b'), "{:?}", screen.styled);
    assert!(!screen.text.contains('\x1b'), "{:?}", screen.text);
    let mut plain = String::new();
    let mut chars = screen.styled.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            chars.by_ref().find(|next| *next == 'm');
        } else {
            plain.push(c);
        }
    }
    assert_eq!(plain, screen.text);
    let text = Screen::text("hello");
    assert_eq!(text.styled, text.text);
}

#[test]
fn the_screens_after_a_proposal_and_a_template_use_carry_a_styled_copy() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, _) = proposed(dir.path());
    let after_proposal = send_it.proposed(None, 80);
    assert!(after_proposal.styled.contains('\x1b'));
    assert!(!after_proposal.text.contains('\x1b'));
    send_it.save_template("ship", Some("GUZ-1"));
    let after_template = send_it.use_template("GUZ-2", "ship", 80);
    assert!(after_template.styled.contains('\x1b'));
    assert!(!after_template.text.contains('\x1b'));
}

#[test]
fn without_a_run_every_command_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let send_it = quiet(dir.path());
    let none = Screen::text(NO_RUN);
    assert_eq!(send_it.status(80), none);
    assert_eq!(go(&send_it, Some("x"), true), (NO_RUN.to_string(), None));
    assert_eq!(send_it.off(), NO_RUN);
    assert!(NO_RUN.starts_with("No send-it run."));
}

#[test]
fn a_turn_that_stores_no_new_plan_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let send_it = quiet(dir.path());
    assert_eq!(send_it.proposed(None, 80).text, NOT_PROPOSED);
    let (send_it, hash) = proposed(dir.path());
    assert_eq!(send_it.proposed(Some(hash.clone()), 80).text, NOT_PROPOSED);
    assert_eq!(send_it.proposed(None, 80).shown, Some(hash));
}

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
        trust: Trust::NONE,
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
    judging_on(quiet(dir))
}

fn judging_on(send_it: SendIt) -> (SendIt, Active) {
    let mut args = plan_args();
    args["steps"] =
        json!([{"do": "review", "criterion": {"kind": "judge", "question": "all fixed?"}}]);
    send_it.propose(&args).unwrap();
    let hash = send_it.status(80).shown.unwrap();
    let started = go(&send_it, Some(&hash), true).1.unwrap();
    let ticked = tick(
        &send_it,
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
    let ticked = tick(&send_it, &active, ended());
    assert_eq!(ticked.next, Next::Done);
    assert_eq!(
        ticked.met.as_deref(),
        Some("send-it: step 1/1 met: all 3 fixed")
    );
    let Next::Send(line) = tick(&send_it, &active, ended()).next else {
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
        workers: Workers::new("/s".into(), "/bin/ralph".into(), ClaudeMode::Auto, None),
        send_it: send_it.clone(),
        watches: watches(dir.path()),
        inbox: Arc::new(Inbox::new(dir.path().join("inbox"))),
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
    assert_eq!(tick(&send_it, &active, ended()).next, Next::Done);
}

#[test]
fn only_an_approved_plan_lends_its_allowed_list() {
    let dir = tempfile::tempdir().unwrap();
    let send_it = quiet(dir.path());
    assert_eq!(send_it.allowed(), None);
    let mut args = plan_args();
    args["allowed"] = json!([{"kind": "reply"}, {"kind": "watch"}]);
    send_it.propose(&args).unwrap();
    assert_eq!(send_it.allowed(), None);
    let hash = send_it.status(80).shown.unwrap();
    go(&send_it, Some(&hash), true);
    assert_eq!(send_it.allowed(), Some(vec![Action::Reply, Action::Watch]));
}

#[test]
fn the_next_tick_takes_the_answered_workers_once() {
    let dir = tempfile::tempdir().unwrap();
    let send_it = quiet(dir.path());
    let mut args = plan_args();
    args["allowed"] = json!([{"kind": "reply"}]);
    send_it.propose(&args).unwrap();
    let hash = send_it.status(80).shown.unwrap();
    let active = Active {
        driving: go(&send_it, Some(&hash), true).1.unwrap().driving,
        watched: None,
    };
    let flagged = [ralph_core::worker::Report::from_worker(
        &ralph_core::worker::WorkerId::parse("E1").unwrap(),
        "2026-01-01T00:00:00Z".parse().unwrap(),
        ralph_core::worker::ReportStatus::Question,
        "q".to_string(),
    )];
    let with_question = || Ended {
        drained: &flagged,
        ..ended()
    };
    send_it.answer("E1".to_string());
    assert_eq!(
        tick(&send_it, &active, with_question()).next,
        Next::Check("cargo xtask lint".to_string())
    );
    assert!(matches!(
        tick(&send_it, &active, with_question()).next,
        Next::Stop(StopReason::WorkerFlagged { .. })
    ));
}

#[test]
fn a_refusal_from_the_socket_stops_the_next_tick_once_and_keeps_the_approval() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = started(dir.path());
    send_it.clone().refuse("spawn_worker codex".to_string());
    assert_eq!(
        tick(&send_it, &active, ended()).next,
        Next::Stop(StopReason::Refused {
            tool: "spawn_worker codex".to_string()
        })
    );
    assert_eq!(
        tick(&send_it, &active, ended()).next,
        Next::Check("cargo xtask lint".to_string())
    );
    assert!(send_it.allowed().is_some());
}

#[test]
fn a_worker_turn_reply_the_run_does_not_allow_is_refused_and_stops_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let shared = Shared {
        store: Store::new(dir.path().to_path_buf()),
        workers: Workers::new("/s".into(), "/bin/ralph".into(), ClaudeMode::Auto, None),
        send_it: quiet(dir.path()),
        watches: watches(dir.path()),
        inbox: Arc::new(Inbox::new(dir.path().join("inbox"))),
    };
    let mut args = plan_args();
    args["allowed"] = json!([{"kind": "watch"}]);
    shared.send_it.propose(&args).unwrap();
    let hash = shared.send_it.status(80).shown.unwrap();
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
        shared
            .send_it
            .tick(&active.driving, active.watched.as_ref(), ended())
            .next,
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
        workers: Workers::new("/s".into(), "/bin/ralph".into(), ClaudeMode::Auto, None),
        send_it: quiet(dir.path()),
        watches: watches(dir.path()),
        inbox: Arc::new(Inbox::new(dir.path().join("inbox"))),
    };
    let mut args = plan_args();
    args["allowed"] = json!([{"kind": "watch"}]);
    shared.send_it.propose(&args).unwrap();
    let hash = shared.send_it.status(80).shown.unwrap();
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
        shared
            .send_it
            .tick(&active.driving, active.watched.as_ref(), ended())
            .next,
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
    assert_eq!(ticked.next, Next::Check("cargo xtask lint".to_string()));
}

#[test]
fn release_drops_the_run_s_approval_and_keeps_the_plan_for_save_and_reuse() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = started(dir.path());
    let hash = active.driving.hash.clone();
    assert_eq!(send_it.release("000000000000", Some(1)), Ok(None));
    assert!(send_it.allowed().is_some());
    send_it.release(&hash, None).unwrap();
    let file = send_it.load().unwrap().unwrap();
    assert_eq!(file.approved, None);
    assert_eq!(plan_hash(&file.plan), hash);
    assert!(send_it
        .status(80)
        .text
        .ends_with(&format!("{hash} · not approved · /send-it go to approve")));
    assert!(send_it
        .save_template("ship", None)
        .starts_with("Template ship saved"));
    assert!(send_it.use_template("GUZ-2", "ship", 80).shown.is_some());
    assert!(send_it.propose(&plan_args()).is_ok());
    send_it.release(&hash, None).unwrap();
    send_it.off();
    assert_eq!(send_it.release(&hash, Some(1)), Ok(None));
}

#[test]
fn a_stop_keeps_the_plan_until_off_and_go_resumes_at_the_stopped_step() {
    let dir = tempfile::tempdir().unwrap();
    let send_it = quiet(dir.path());
    let args = json!({
        "goal": "ship GUZ-1",
        "steps": [
            {"do": "spawn the worker", "criterion": {"kind": "worker_done", "worker": "GUZ-1"}},
            {"do": "run the checks", "criterion": {"kind": "check_passes", "command": "cargo xtask lint"}}
        ]
    });
    send_it.propose(&args).unwrap();
    let hash = send_it.status(80).shown.unwrap();
    let first = go(&send_it, Some(&hash), true).1.unwrap();
    let stopped = send_it.release(&hash, Some(2)).unwrap().unwrap();
    assert_eq!(stopped.approved, None);
    assert_eq!(send_it.load().unwrap(), Some(stopped));
    let screen = send_it.status(80);
    assert!(
        screen
            .text
            .ends_with("stopped at step 2/2 · /send-it go to resume from step 2"),
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
    assert_eq!(send_it.status(80).text, NO_RUN);
}

#[test]
fn go_on_a_stopped_worker_step_skips_it_once_the_worker_reported_done() {
    let dir = tempfile::tempdir().unwrap();
    let send_it = quiet(dir.path());
    let args = json!({
        "goal": "ship GUZ-1",
        "steps": [
            {"do": "spawn the worker", "criterion": {"kind": "worker_done", "worker": "GUZ-1"}},
            {"do": "run the checks", "criterion": {"kind": "check_passes", "command": "cargo xtask lint"}}
        ]
    });
    send_it.propose(&args).unwrap();
    let hash = send_it.status(80).shown.unwrap();
    go(&send_it, Some(&hash), true);
    send_it.release(&hash, Some(1)).unwrap();
    let done = ralph_core::worker::ReportKind::parse("done").unwrap();
    let last = [(ralph_core::worker::WorkerId::parse("GUZ-1").unwrap(), done)].into();
    let seen = Seen {
        last: &last,
        running: &[],
    };
    let resumed = send_it.go(Some(&hash), true, &seen).1.unwrap();
    assert_eq!(
        resumed.met.as_deref(),
        Some("send-it: step 1/2 met (worker GUZ-1 reports done)")
    );
    assert_eq!(resumed.next, Next::Check("cargo xtask lint".to_string()));
}

#[test]
fn every_tick_reads_the_plan_file_again() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = started(dir.path());
    assert_eq!(
        tick(&send_it, &active, ended()).next,
        Next::Check("cargo xtask lint".to_string())
    );
    send_it.release(&active.driving.hash, None).unwrap();
    assert_eq!(
        tick(&send_it, &active, ended()).next,
        Next::Stop(StopReason::PlanChanged(None))
    );
    send_it.off();
    assert_eq!(
        tick(&send_it, &active, ended()).next,
        Next::Stop(StopReason::Off)
    );
}

#[test]
fn a_tick_takes_the_outcome_the_watch_left_in_its_slot() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = started(dir.path());
    let waiting = tick(&send_it, &active, ended()).driving;
    for (outcome, next) in [
        (WatchOutcome::Exited(0), Next::Done),
        (
            WatchOutcome::Exited(1),
            Next::Stop(StopReason::check_failed(1)),
        ),
    ] {
        let active = Active {
            driving: waiting.clone(),
            watched: Some(OutcomeSlot::new(Mutex::new(Some(outcome)))),
        };
        assert_eq!(tick(&send_it, &active, ended()).next, next);
    }
}

#[test]
fn a_replied_tick_collects_the_pending_lines_of_the_last_assistant_entry() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = judging(dir.path());
    Store::new(dir.path().to_path_buf())
        .append(Role::Assistant, "done.\nPending: push main")
        .unwrap();
    send_it.record(StepResult {
        met: false,
        evidence: "open".to_string(),
    });
    let ticked = tick(&send_it, &active, ended());
    assert_eq!(ticked.driving.tally.pending, ["push main"]);
    let silent = tick(
        &send_it,
        &active,
        Ended {
            replied: false,
            ..ended()
        },
    );
    assert!(silent.driving.tally.pending.is_empty());
}

fn logged_texts(dir: &Path) -> Vec<String> {
    let (entries, _) = Store::new(dir.to_path_buf()).load().unwrap();
    entries.into_iter().map(|entry| entry.text).collect()
}

#[test]
fn a_dispatched_notice_is_logged_once() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = judging(dir.path());
    let notice = "Step 1: the check would block the worker".to_string();
    let ticked = Ticked {
        driving: active.driving,
        next: Next::Wait,
        met: None,
        notice: Some(notice.clone()),
    };
    dispatch(&send_it, ticked, &held::monitor(dir.path()));
    assert_eq!(logged_texts(dir.path()), vec![notice_record(&notice)]);
}

#[test]
fn a_dispatched_met_line_is_logged_once() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = judging(dir.path());
    let ticked = Ticked {
        driving: active.driving,
        next: Next::Wait,
        met: Some("Step 1 met".to_string()),
        notice: None,
    };
    dispatch(&send_it, ticked, &held::monitor(dir.path()));
    assert_eq!(logged_texts(dir.path()), vec![notice_record("Step 1 met")]);
}

#[test]
fn a_dispatched_notice_is_logged_before_its_met_line() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = judging(dir.path());
    let ticked = Ticked {
        driving: active.driving,
        next: Next::Wait,
        met: Some("Step 1 met".to_string()),
        notice: Some("Step 1: the check would block the worker".to_string()),
    };
    dispatch(&send_it, ticked, &held::monitor(dir.path()));
    assert_eq!(
        logged_texts(dir.path()),
        vec![
            notice_record("Step 1: the check would block the worker"),
            notice_record("Step 1 met")
        ]
    );
}

#[test]
fn a_dispatched_done_tick_logs_no_met_line() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = judging(dir.path());
    let ticked = Ticked {
        driving: active.driving,
        next: Next::Done,
        met: Some("Step 1 met".to_string()),
        notice: None,
    };
    dispatch(&send_it, ticked, &held::monitor(dir.path()));
    let logged = logged_texts(dir.path());
    assert_eq!(logged.len(), 1, "{logged:?}");
    assert!(
        logged[0].starts_with(&notice_record("send-it done: 1/1")),
        "{logged:?}"
    );
}

#[test]
fn a_done_tick_without_a_plan_still_logs_its_met_line() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = judging(dir.path());
    send_it.off();
    let ticked = Ticked {
        driving: active.driving,
        next: Next::Done,
        met: Some("Step 1 met".to_string()),
        notice: None,
    };
    dispatch(&send_it, ticked, &held::monitor(dir.path()));
    assert_eq!(logged_texts(dir.path()), vec![notice_record("Step 1 met")]);
}

#[test]
fn a_settled_check_logs_its_notice_once_and_before_its_met_line() {
    let dir = tempfile::tempdir().unwrap();
    let send_it = quiet(dir.path());
    let mut args = plan_args();
    args["steps"] = json!([
        {"do": "collect evidence", "criterion": {"kind": "worker_done", "worker": "E1", "verify": "cargo test"}},
        {"do": "review", "criterion": {"kind": "judge", "question": "ok?"}}
    ]);
    send_it.propose(&args).unwrap();
    let hash = send_it.status(80).shown.unwrap();
    let mut driving = go(&send_it, Some(&hash), true).1.unwrap().driving;
    driving.awaiting = Awaiting::Report("watch:send-it-1".to_string());
    driving.progress.verify = VerifyMode::Observe;
    driving.progress.verifying = true;
    let mut sending = Some(Active {
        driving,
        watched: Some(ralph_server::watches::OutcomeSlot::new(Mutex::new(Some(
            WatchOutcome::Exited(1),
        )))),
    });
    held_item(
        &send_it,
        &mut sending,
        Position::FIRST,
        &held::monitor(dir.path()),
    )
    .unwrap();
    let logged = logged_texts(dir.path());
    let at = |needle: &str| {
        logged
            .iter()
            .enumerate()
            .filter(|(_, text)| text.contains(needle))
            .map(|(index, _)| index)
            .collect::<Vec<_>>()
    };
    assert_eq!(at("would block"), [0], "{logged:?}");
    assert_eq!(at("step 1/2 met"), [1], "{logged:?}");
}

#[test]
fn a_settled_last_check_logs_only_the_done_notice() {
    let dir = tempfile::tempdir().unwrap();
    let send_it = quiet(dir.path());
    send_it.propose(&plan_args()).unwrap();
    let hash = send_it.status(80).shown.unwrap();
    let mut driving = go(&send_it, Some(&hash), true).1.unwrap().driving;
    driving.awaiting = Awaiting::Report("watch:send-it-1".to_string());
    driving.progress.verify = VerifyMode::Observe;
    driving.progress.verifying = true;
    let mut sending = Some(Active {
        driving,
        watched: Some(ralph_server::watches::OutcomeSlot::new(Mutex::new(Some(
            WatchOutcome::Exited(0),
        )))),
    });
    held_item(
        &send_it,
        &mut sending,
        Position::FIRST,
        &held::monitor(dir.path()),
    )
    .unwrap();
    let logged = logged_texts(dir.path());
    assert_eq!(logged.len(), 1, "{logged:?}");
    assert!(
        logged[0].starts_with(&notice_record("send-it done:")),
        "{logged:?}"
    );
}

#[derive(Clone, Default)]
struct Printed(Arc<Mutex<String>>);

impl Printed {
    fn text(&self) -> String {
        self.0.lock().unwrap().clone()
    }

    fn sink(&self) -> Emit {
        let lines = self.0.clone();
        Arc::new(move |event| {
            if let Event::Notice(line) = event {
                let mut lines = lines.lock().unwrap();
                lines.push_str(&line);
                lines.push('\n');
            }
        })
    }
}

fn printing(dir: &Path) -> (SendIt, Active, Printed) {
    let printed = Printed::default();
    let send_it = SendIt::new(dir, LIMITS, printed.sink()).unwrap();
    let (send_it, active) = judging_on(send_it);
    (send_it, active, printed)
}

fn done(active: Active, met: Option<&str>) -> Ticked {
    Ticked {
        driving: active.driving,
        next: Next::Done,
        met: met.map(str::to_string),
        notice: None,
    }
}

#[test]
fn a_done_tick_without_a_plan_prints_and_logs_its_met_line_once() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active, printed) = printing(dir.path());
    send_it.off();
    dispatch(
        &send_it,
        done(active, Some("Step 1 met")),
        &held::monitor(dir.path()),
    );
    assert_eq!(printed.text(), "Step 1 met\n");
    assert_eq!(logged_texts(dir.path()), vec![notice_record("Step 1 met")]);
}

#[test]
fn a_done_tick_without_a_plan_or_a_met_line_stays_silent() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active, printed) = printing(dir.path());
    send_it.off();
    dispatch(&send_it, done(active, None), &held::monitor(dir.path()));
    assert_eq!(printed.text(), "");
    assert!(logged_texts(dir.path()).is_empty());
}

#[test]
fn a_done_tick_with_a_plan_prints_and_logs_only_the_done_notice() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active, printed) = printing(dir.path());
    dispatch(
        &send_it,
        done(active, Some("Step 1 met")),
        &held::monitor(dir.path()),
    );
    let logged = logged_texts(dir.path());
    assert_eq!(logged.len(), 1, "{logged:?}");
    assert!(logged[0].contains("send-it done"), "{logged:?}");
    assert!(!logged[0].contains("Step 1 met"), "{logged:?}");
    assert!(
        printed.text().contains("send-it DONE"),
        "{}",
        printed.text()
    );
    assert!(!printed.text().contains("Step 1 met"), "{}", printed.text());
}

fn quiet(dir: &Path) -> SendIt {
    SendIt::new(dir, LIMITS, Printed::default().sink()).unwrap()
}
