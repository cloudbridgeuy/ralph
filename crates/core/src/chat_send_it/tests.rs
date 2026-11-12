use super::*;
use crate::inbox_alert::UnknownFocus;
use crate::program_status::{Kind, Report as ProgramReport, State};
use crate::worker::Report;

mod drive;
mod plan;
mod template;

fn limits() -> Limits {
    Limits {
        max_turns: 3,
        max_usd: 1.0,
    }
}

const RESTART: &str = "To run it again, show the plan with /send-it status, then type /send-it go.";

#[test]
fn a_stop_that_is_not_off_says_the_run_stopped_and_how_to_restart_it() {
    let run = Run::default();
    for reason in [
        StopReason::TurnCap,
        StopReason::CostCap,
        StopReason::Unmet { step: 2 },
        StopReason::WorkerFlagged,
        StopReason::JudgeRounds {
            step: 2,
            evidence: "tests fail".to_string(),
        },
        StopReason::CheckFailed { step: 2 },
        StopReason::PlanChanged,
        StopReason::NoReply,
        StopReason::NeedsYou,
        StopReason::TookOver,
        StopReason::Refused {
            tool: "watch".to_string(),
        },
    ] {
        let notice = reason.notice(&run, None);
        assert!(notice.starts_with("send-it stopped: "), "{notice}");
        assert!(notice.ends_with(RESTART), "{notice}");
        assert!(!notice.contains("send-it off"), "{notice}");
        assert!(!notice.contains("Turn it on again"), "{notice}");
    }
}

#[test]
fn a_refused_stop_names_the_tool() {
    let refused = StopReason::Refused {
        tool: "spawn_worker codex".to_string(),
    };
    assert_eq!(
        refused.notice(&Run::default(), None),
        format!("send-it stopped: the plan does not allow `spawn_worker codex`. {RESTART}")
    );
}

#[test]
fn an_off_stop_says_the_plan_is_deleted() {
    assert_eq!(
        StopReason::Off.notice(&Run::default(), None),
        "send-it off: the run stopped and its plan is deleted."
    );
}

fn report(status: &str) -> Report {
    Report {
        worker: "GUZ-1".to_string(),
        at: "2026-01-01T00:00:00Z".to_string(),
        status: status.to_string(),
        text: "text".to_string(),
    }
}

#[test]
fn only_question_and_blocked_reports_flag_a_worker() {
    assert!(worker_flagged(&[report("done"), report("question")]));
    assert!(worker_flagged(&[report("blocked")]));
    assert!(!worker_flagged(&[report("done"), report("progress")]));
    assert!(!worker_flagged(&[]));
}

#[test]
fn a_blocked_alert_carries_the_notice_and_follows_the_config() {
    let config = AlertConfig {
        sound: true,
        notification: true,
        unknown_focus: UnknownFocus::Alert,
        terminal_status: false,
    };
    assert_eq!(
        blocked_alert("send-it stopped", &config),
        Some(Alert {
            sound: true,
            notification: Some("send-it stopped".to_string()),
        })
    );
    let quiet = AlertConfig {
        sound: false,
        notification: false,
        ..config
    };
    assert_eq!(blocked_alert("send-it stopped", &quiet), None);
    let sound_only = AlertConfig {
        notification: false,
        ..config
    };
    assert_eq!(
        blocked_alert("x", &sound_only),
        Some(Alert {
            sound: true,
            notification: None
        })
    );
}

#[test]
fn a_blocked_alert_is_left_to_the_terminal_when_it_sends_status() {
    let rex = AlertConfig {
        sound: true,
        notification: true,
        unknown_focus: UnknownFocus::Alert,
        terminal_status: true,
    };
    assert_eq!(blocked_alert("send-it stopped", &rex), None);
}

#[test]
fn the_blocked_frame_report_is_a_question_with_the_notice() {
    assert_eq!(
        blocked_report("send-it stopped"),
        ProgramReport {
            state: State::Blocked(Kind::Question),
            msg: Some("send-it stopped".to_string()),
        }
    );
}

fn plan_json() -> serde_json::Value {
    serde_json::json!({
        "goal": "ship GUZ-1",
        "steps": [
            {"do": "collect evidence", "criterion": {"kind": "worker_done", "worker": "E1"}},
            {"do": "wait for CI", "criterion": {"kind": "watch_passes", "command": "gh pr checks --watch"}},
            {"do": "run the checks", "criterion": {"kind": "check_passes", "command": "cargo xtask lint"}},
            {"do": "address review", "criterion": {"kind": "judge", "question": "are all findings resolved?"}}
        ],
        "allowed": [{"kind": "spawn", "harness": "codex"}, {"kind": "merge"}, {"kind": "paid_run", "max_usd": 2.5}],
        "grants": [{"kind": "turn_cap", "turns": 30}],
        "caps": {"usd": 4.0}
    })
}

fn plan() -> RunPlan {
    parse_run(&plan_json(), limits()).unwrap()
}

#[test]
fn a_run_plan_parses_with_typed_steps_actions_and_grants() {
    let plan = plan();
    assert_eq!(plan.goal, "ship GUZ-1");
    assert_eq!(
        plan.steps[0],
        Step {
            task: "collect evidence".to_string(),
            criterion: Criterion::WorkerDone {
                worker: "E1".to_string()
            },
        }
    );
    assert_eq!(
        plan.allowed,
        vec![
            Action::Spawn {
                harness: HarnessId::Codex
            },
            Action::Merge,
            Action::PaidRun { max_usd: 2.5 }
        ]
    );
    assert_eq!(plan.grants, vec![Grant::TurnCap { turns: 30 }]);
}

#[test]
fn missing_caps_fall_back_to_the_defaults() {
    let plan = plan();
    assert_eq!(
        plan.limits,
        Limits {
            max_turns: 3,
            max_usd: 4.0
        }
    );
    assert_eq!(plan.judge_rounds, DEFAULT_JUDGE_ROUNDS);
}

#[test]
fn a_step_without_a_criterion_is_rejected_by_number() {
    let mut args = plan_json();
    args["steps"][1]
        .as_object_mut()
        .unwrap()
        .remove("criterion");
    let error = parse_run(&args, limits()).unwrap_err();
    assert!(error.contains("Step 2 has no exit criterion"), "{error}");
}

#[test]
fn an_empty_goal_no_steps_or_an_empty_step_are_rejected() {
    for (field, value, expected) in [
        ("goal", serde_json::json!("  "), "no goal"),
        ("steps", serde_json::json!([]), "no steps"),
        (
            "steps",
            serde_json::json!([{"do": " ", "criterion": {"kind": "judge", "question": "q"}}]),
            "Step 1 is empty",
        ),
    ] {
        let mut args = plan_json();
        args[field] = value;
        let error = parse_run(&args, limits()).unwrap_err();
        assert!(error.contains(expected), "{field}: {error}");
    }
}

#[test]
fn zero_caps_and_unknown_kinds_are_rejected() {
    for caps in [
        serde_json::json!({"turns": 0}),
        serde_json::json!({"usd": 0.0}),
        serde_json::json!({"judge_rounds": 0}),
    ] {
        let mut args = plan_json();
        args["caps"] = caps.clone();
        let error = parse_run(&args, limits()).unwrap_err();
        assert!(error.contains("caps must be positive"), "{caps}: {error}");
    }
    let mut args = plan_json();
    args["allowed"] = serde_json::json!([{"kind": "deploy"}]);
    let error = parse_run(&args, limits()).unwrap_err();
    assert!(error.contains("deploy"), "{error}");
}

#[test]
fn the_restatement_numbers_steps_and_keeps_grants_in_their_own_block() {
    assert_eq!(
        render_run(&plan()),
        "Goal: ship GUZ-1\n\
         Steps:\n  \
           1. collect evidence\n     \
              exit: worker E1 reports done\n  \
           2. wait for CI\n     \
              exit: watch `gh pr checks --watch` exits 0\n  \
           3. run the checks\n     \
              exit: `cargo xtask lint` exits 0\n  \
           4. address review\n     \
              exit: judged: are all findings resolved?\n\
         Allowed: spawn codex workers, merge, paid runs up to $2.50\n\
         Caps: 3 turns, $4.00, 2 judge rounds\n\
         You pre-approve:\n  \
           - raise the turn cap to 30"
    );
}

#[test]
fn a_restatement_without_grants_or_actions_says_so() {
    let mut plan = plan();
    plan.allowed.clear();
    plan.grants.clear();
    let text = render_run(&plan);
    assert!(text.contains("\nAllowed: none\n"), "{text}");
    assert!(text.ends_with("You pre-approve:\n  - nothing"), "{text}");
}

fn at(step: usize) -> Progress {
    Progress {
        step,
        ..Progress::default()
    }
}

fn start(file: &RunFile, known: &[(&str, &str)], running: &[&str]) -> Option<Ticked> {
    let last: HashMap<String, String> = known
        .iter()
        .map(|(id, status)| (id.to_string(), status.to_string()))
        .collect();
    let running: Vec<String> = running.iter().map(ToString::to_string).collect();
    start_run(
        file,
        &Seen {
            last: &last,
            running: &running,
        },
    )
}

fn worker_report(worker: &str, status: &str) -> Report {
    Report {
        worker: worker.to_string(),
        status: status.to_string(),
        ..report(status)
    }
}

fn spent(turns: u32, spent_usd: f64) -> Run {
    Run { turns, spent_usd }
}

const STEP_1: &str = "[send-it 1/4] collect evidence · exit: worker E1 reports done";
const STEP_4: &str = "[send-it 4/4] address review · exit: judged: are all findings resolved?";

#[test]
fn start_sends_the_first_step_with_its_criterion() {
    assert_eq!(
        advance(&plan(), Progress::default(), Event::Start),
        (at(0), Next::Send(STEP_1.to_string()))
    );
}

#[test]
fn a_turn_end_waits_on_the_step_criterion() {
    let run = spent(1, 0.5);
    for (step, next) in [
        (0, Next::Hold("E1".to_string())),
        (1, Next::Watch("gh pr checks --watch".to_string())),
        (2, Next::Check("cargo xtask lint".to_string())),
        (3, Next::Judge),
    ] {
        assert_eq!(
            advance(&plan(), at(step), Event::TurnEnded(&run)),
            (at(step), next)
        );
    }
}

#[test]
fn caps_stop_at_a_turn_end_and_grants_lift_them() {
    let plan = plan();
    for (run, next) in [
        (spent(3, 0.5), Next::Hold("E1".to_string())),
        (spent(30, 0.5), Next::Stop(StopReason::TurnCap)),
        (spent(1, 4.0), Next::Stop(StopReason::CostCap)),
    ] {
        assert_eq!(advance(&plan, at(0), Event::TurnEnded(&run)).1, next);
    }
    let mut granted = plan.clone();
    granted.grants = vec![Grant::CostCap { usd: 10.0 }];
    assert_eq!(
        advance(&granted, at(0), Event::TurnEnded(&spent(1, 4.0))).1,
        Next::Hold("E1".to_string())
    );
    assert_eq!(
        advance(&granted, at(0), Event::TurnEnded(&spent(3, 1.0))).1,
        Next::Stop(StopReason::TurnCap)
    );
}

#[test]
fn reports_move_on_wait_or_stop() {
    let plan = plan();
    for (report, expected) in [
        (
            worker_report("E1", "done"),
            (at(1), Next::Watch("gh pr checks --watch".to_string())),
        ),
        (worker_report("X9", "done"), (at(0), Next::Wait)),
        (
            worker_report("E1", "exited"),
            (at(0), Next::Stop(StopReason::Unmet { step: 1 })),
        ),
    ] {
        assert_eq!(advance(&plan, at(0), Event::Report(&report)), expected);
    }
}

#[test]
fn each_watch_outcome_settles_a_check_as_met_or_failed() {
    let plan = plan();
    let passed = WatchOutcome::Exited(0);
    assert_eq!(
        advance(&plan, at(1), Event::Watched(&passed)),
        (at(2), Next::Check("cargo xtask lint".to_string()))
    );
    assert_eq!(
        advance(&plan, at(2), Event::Watched(&passed)),
        (at(3), Next::Send(STEP_4.to_string()))
    );
    for failed in [
        WatchOutcome::Exited(1),
        WatchOutcome::Signaled(9),
        WatchOutcome::Stopped,
        WatchOutcome::TimedOut(std::time::Duration::from_secs(30)),
        WatchOutcome::WaitFailed("gone".to_string()),
    ] {
        assert_eq!(
            advance(&plan, at(2), Event::Watched(&failed)),
            (at(2), Next::Stop(StopReason::CheckFailed { step: 3 })),
            "{failed:?}"
        );
    }
    assert_eq!(
        advance(&plan, at(0), Event::Watched(&passed)),
        (at(0), Next::Wait)
    );
}

#[test]
fn a_check_runs_as_a_plain_watch_named_for_its_step() {
    assert_eq!(check_watch_id(2), "watch:send-it-3");
    assert_eq!(
        check_spec(2, "true"),
        crate::watch::WatchSpec {
            id: "watch:send-it-3".to_string(),
            command: "true".to_string(),
            follow_up: None,
            kind: crate::watch::WatchKind::Normal,
            timeout: None,
            limit: None,
        }
    );
}

fn judged(met: bool, evidence: &str) -> StepResult {
    StepResult {
        met,
        evidence: evidence.to_string(),
    }
}

#[test]
fn verdicts_finish_or_retry_with_the_evidence_and_no_call_is_a_failed_round() {
    let plan = plan();
    let met = judged(true, "all 3 findings fixed");
    let unmet = judged(false, "finding 2 is open");
    assert_eq!(
        advance(&plan, at(3), Event::Verdict(Some(&met))),
        (at(3), Next::Done)
    );
    assert_eq!(
        advance(&plan, at(0), Event::Verdict(Some(&met))),
        (at(0), Next::Wait)
    );
    let retried = Progress {
        judge_rounds: 1,
        ..at(3)
    };
    for (result, evidence) in [(Some(&unmet), "finding 2 is open"), (None, NO_STEP_RESULT)] {
        let retry = Next::Send(format!("{STEP_4}\njudge: not met: {evidence}"));
        assert_eq!(
            advance(&plan, at(3), Event::Verdict(result)),
            (retried, retry)
        );
        let stop = StopReason::JudgeRounds {
            step: 4,
            evidence: evidence.to_string(),
        };
        assert_eq!(
            advance(&plan, retried, Event::Verdict(result)),
            (retried, Next::Stop(stop))
        );
    }
}

#[test]
fn new_stop_notices_name_the_step() {
    let run = Run::default();
    assert_eq!(
        StopReason::Unmet { step: 2 }.notice(&run, None),
        format!("send-it stopped: step 2 missed its exit criterion. {RESTART}")
    );
    assert!(StopReason::WorkerFlagged
        .notice(&run, None)
        .contains("a worker asked a question"));
    assert_eq!(
        StopReason::JudgeRounds {
            step: 4,
            evidence: "finding 2 is open.".to_string()
        }
        .notice(&run, None),
        format!("send-it stopped: step 4 was still not met after the last judge round: finding 2 is open. {RESTART}")
    );
    for (reason, why) in [
        (
            StopReason::CheckFailed { step: 3 },
            "step 3 check did not exit 0",
        ),
        (StopReason::PlanChanged, "the run plan changed"),
        (StopReason::NoReply, "a turn ended without a reply"),
        (StopReason::NeedsYou, "a schedule waits for your approval"),
        (StopReason::TookOver, "you took over"),
    ] {
        let notice = reason.notice(&run, None);
        assert!(
            notice.starts_with(&format!("send-it stopped: {why}")),
            "{notice}"
        );
    }
}

fn approved(plan: RunPlan) -> RunFile {
    RunFile {
        approved: Some(plan_hash(&plan)),
        plan,
        stopped: None,
    }
}

fn driving(step: usize, awaiting: Awaiting, run: Run) -> Driving {
    Driving {
        hash: plan_hash(&plan()),
        progress: at(step),
        run,
        awaiting,
    }
}

fn ended(file: &RunFile) -> Ended<'_> {
    Ended {
        file: Some(file),
        replied: true,
        asked: false,
        typed: false,
        queued: &[],
        cost_usd: 0.5,
        step_result: None,
        drained: &[],
        unread: &[],
        answered: &[],
        watched: None,
        refused: None,
    }
}

#[test]
fn only_an_approved_file_starts_a_run_at_step_one() {
    let unapproved = RunFile {
        plan: plan(),
        approved: None,
        stopped: None,
    };
    assert_eq!(start(&unapproved, &[], &[]), None);
    let started = start(&approved(plan()), &[("E1", "done")], &["E1"]).unwrap();
    assert_eq!(started.met, None);
    assert_eq!(started.driving, driving(0, Awaiting::Turn, Run::default()));
    let next = started.next;
    let Next::Send(brief) = next else {
        panic!("{next:?}")
    };
    let hash = plan_hash(&plan());
    assert!(
        brief.starts_with(&format!("send-it run {hash} is approved.")),
        "{brief}"
    );
    assert!(brief.contains(&render_run(&plan())), "{brief}");
    assert!(brief.contains("only the allowed actions"), "{brief}");
    assert!(brief.ends_with(STEP_1), "{brief}");
}

#[test]
fn a_step_turn_counts_its_cost_and_waits_on_the_criterion() {
    let file = approved(plan());
    let ticked = tick(&driving(0, Awaiting::Turn, spent(1, 0.25)), &ended(&file));
    assert_eq!(
        ticked,
        Ticked {
            driving: driving(0, Awaiting::Report("E1".to_string()), spent(2, 0.75)),
            next: Next::Hold("E1".to_string()),
            met: None,
        }
    );
    let check = tick(&driving(2, Awaiting::Turn, Run::default()), &ended(&file));
    assert_eq!(check.next, Next::Check("cargo xtask lint".to_string()));
    assert_eq!(
        check.driving.awaiting,
        Awaiting::Report("watch:send-it-3".to_string())
    );
    let judge = tick(&driving(3, Awaiting::Turn, Run::default()), &ended(&file));
    assert_eq!(
        (judge.next, judge.driving.awaiting),
        (Next::Judge, Awaiting::Verdict)
    );
}

#[test]
fn a_missing_unapproved_or_changed_plan_stops_the_run_first() {
    let base = approved(plan());
    let mut edited = plan();
    edited.goal = "ship GUZ-2".to_string();
    let edited = approved(edited);
    let unapproved = RunFile {
        plan: plan(),
        approved: None,
        stopped: None,
    };
    let state = driving(0, Awaiting::Turn, Run::default());
    let flagged = [worker_report("E1", "question")];
    for (file, reason) in [
        (None, StopReason::Off),
        (Some(&unapproved), StopReason::PlanChanged),
        (Some(&edited), StopReason::PlanChanged),
    ] {
        let ended = Ended {
            file,
            replied: false,
            unread: &flagged,
            refused: Some("watch"),
            ..ended(&base)
        };
        assert_eq!(tick(&state, &ended).next, Next::Stop(reason));
    }
}

#[test]
fn a_failed_typed_asked_or_flagged_turn_stops_the_run() {
    let file = approved(plan());
    let state = driving(0, Awaiting::Turn, Run::default());
    let stop = |change: fn(&mut Ended)| {
        let mut ended = ended(&file);
        change(&mut ended);
        tick(&state, &ended).next
    };
    assert_eq!(stop(|e| e.replied = false), Next::Stop(StopReason::NoReply));
    assert_eq!(stop(|e| e.typed = true), Next::Stop(StopReason::TookOver));
    assert_eq!(stop(|e| e.asked = true), Next::Stop(StopReason::NeedsYou));
    let flagged = [worker_report("X9", "blocked")];
    let ended = Ended {
        unread: &flagged,
        ..ended(&file)
    };
    assert_eq!(
        tick(&state, &ended).next,
        Next::Stop(StopReason::WorkerFlagged)
    );
}

#[test]
fn a_refused_tool_stops_the_run_before_any_other_turn_stop() {
    let file = approved(plan());
    let state = driving(0, Awaiting::Turn, Run::default());
    let flagged = [worker_report("X9", "blocked")];
    let ended = Ended {
        replied: false,
        typed: true,
        asked: true,
        unread: &flagged,
        refused: Some("spawn_worker codex"),
        ..ended(&file)
    };
    assert_eq!(
        tick(&state, &ended).next,
        Next::Stop(StopReason::Refused {
            tool: "spawn_worker codex".to_string()
        })
    );
}

#[test]
fn a_drained_done_report_meets_the_step_and_others_keep_holding() {
    let file = approved(plan());
    let state = driving(0, Awaiting::Report("E1".to_string()), Run::default());
    let done = [worker_report("X9", "done"), worker_report("E1", "done")];
    let ticked = tick(
        &state,
        &Ended {
            drained: &done,
            ..ended(&file)
        },
    );
    assert_eq!(ticked.next, Next::Watch("gh pr checks --watch".to_string()));
    assert_eq!(
        ticked.met.as_deref(),
        Some("send-it: step 1/4 met (worker E1 reports done)")
    );
    assert_eq!(
        (ticked.driving.progress, ticked.driving.awaiting),
        (at(1), Awaiting::Report("watch:send-it-2".to_string()))
    );
    let other = [worker_report("X9", "done")];
    let held = tick(
        &state,
        &Ended {
            drained: &other,
            ..ended(&file)
        },
    );
    assert_eq!((held.next, held.met), (Next::Hold("E1".to_string()), None));
}

#[test]
fn the_watched_outcome_settles_a_check_step() {
    let file = approved(plan());
    let state = driving(
        2,
        Awaiting::Report("watch:send-it-3".to_string()),
        Run::default(),
    );
    let passed = WatchOutcome::Exited(0);
    let failed = WatchOutcome::Exited(2);
    for (watched, next) in [
        (Some(&passed), Next::Send(STEP_4.to_string())),
        (
            Some(&failed),
            Next::Stop(StopReason::CheckFailed { step: 3 }),
        ),
        (None, Next::Check("cargo xtask lint".to_string())),
    ] {
        let ended = Ended {
            watched,
            ..ended(&file)
        };
        assert_eq!(tick(&state, &ended).next, next);
    }
}

#[test]
fn a_report_turn_that_settles_its_step_adds_cost_but_no_turn() {
    let file = approved(plan());
    let state = driving(
        2,
        Awaiting::Report("watch:send-it-3".to_string()),
        spent(1, 0.25),
    );
    let passed = WatchOutcome::Exited(0);
    let ticked = tick(
        &state,
        &Ended {
            watched: Some(&passed),
            ..ended(&file)
        },
    );
    assert_eq!(ticked.next, Next::Send(STEP_4.to_string()));
    assert_eq!(ticked.driving.run, spent(1, 0.75));
}

#[test]
fn a_settling_report_turn_one_below_the_turn_cap_still_sends() {
    let file = approved(plan());
    let done = [worker_report("E1", "done")];
    let ticked = tick(
        &driving(0, Awaiting::Report("E1".to_string()), spent(29, 0.0)),
        &Ended {
            drained: &done,
            ..ended(&file)
        },
    );
    assert_eq!(ticked.next, Next::Watch("gh pr checks --watch".to_string()));
    assert_eq!(ticked.driving.run, spent(29, 0.5));
}

#[test]
fn a_report_turn_that_settles_nothing_counts() {
    let file = approved(plan());
    let ticked = tick(
        &driving(
            2,
            Awaiting::Report("watch:send-it-3".to_string()),
            spent(1, 0.25),
        ),
        &ended(&file),
    );
    assert_eq!(ticked.next, Next::Check("cargo xtask lint".to_string()));
    assert_eq!(ticked.driving.run, spent(2, 0.75));
}

#[test]
fn caps_stop_before_a_send_but_not_before_the_finish() {
    let file = approved(plan());
    let done = [worker_report("E1", "done")];
    let capped = tick(
        &driving(0, Awaiting::Report("E1".to_string()), spent(30, 0.0)),
        &Ended {
            drained: &done,
            ..ended(&file)
        },
    );
    assert_eq!(capped.next, Next::Stop(StopReason::TurnCap));
    let met = judged(true, "all 3 findings fixed");
    let finished = tick(
        &driving(3, Awaiting::Verdict, spent(29, 3.9)),
        &Ended {
            step_result: Some(&met),
            ..ended(&file)
        },
    );
    assert_eq!(finished.next, Next::Done);
    assert_eq!(
        finished.met.as_deref(),
        Some("send-it: step 4/4 met (judged: are all findings resolved?): all 3 findings fixed")
    );
}

#[test]
fn an_unmet_or_missing_step_result_sends_the_step_again() {
    let file = approved(plan());
    let unmet = judged(false, "finding 2 is open");
    for step_result in [Some(&unmet), None] {
        let ticked = tick(
            &driving(3, Awaiting::Verdict, Run::default()),
            &Ended {
                step_result,
                ..ended(&file)
            },
        );
        let retry = format!("{STEP_4}\njudge: not met: ");
        assert!(
            matches!(&ticked.next, Next::Send(line) if line.starts_with(&retry)),
            "{:?}",
            ticked.next
        );
        assert_eq!(ticked.driving.progress.judge_rounds, 1);
    }
}

#[test]
fn a_step_result_needs_a_bool_met_and_one_line_of_evidence() {
    use serde_json::json;
    let parsed = parse_step_result(&json!({"met": true, "evidence": "  all 3 fixed  "}));
    assert_eq!(parsed, Ok(judged(true, "all 3 fixed")));
    let too_long = "x".repeat(STEP_RESULT_MAX_CHARS + 1);
    for args in [
        json!({"evidence": "x"}),
        json!({"met": "yes", "evidence": "x"}),
        json!({"met": true}),
        json!({"met": true, "evidence": "  "}),
        json!({"met": true, "evidence": "a\nb"}),
        json!({"met": false, "evidence": too_long}),
    ] {
        assert!(parse_step_result(&args).is_err(), "{args}");
    }
}

#[test]
fn the_step_result_tool_requires_met_and_evidence() {
    let tool = step_result_tool();
    assert_eq!(tool["name"], STEP_RESULT);
    let schema = &tool["inputSchema"];
    assert_eq!(schema["required"], serde_json::json!(["met", "evidence"]));
    assert_eq!(schema["properties"]["met"]["type"], "boolean");
}

#[test]
fn the_judge_prompt_asks_the_step_question_for_one_step_result_call() {
    let prompt = judge_prompt(&plan(), at(3)).unwrap();
    assert!(prompt.contains("[send-it 4/4]"), "{prompt}");
    assert!(prompt.contains("are all findings resolved?"), "{prompt}");
    assert!(prompt.contains("Call `step_result` once"), "{prompt}");
    assert!(!prompt.contains("verdict:"), "{prompt}");
    assert_eq!(judge_prompt(&plan(), at(0)), None);
}

#[test]
fn the_done_notice_counts_steps_turns_and_cost() {
    assert_eq!(
        done_notice(&plan(), &spent(7, 1.25)),
        format!("send-it done: 4/4 steps in 7 turns, $1.25. {RESTART}")
    );
}

fn stopped_at(step: usize) -> Option<Stopped> {
    Some(Stopped {
        step,
        hash: plan_hash(&plan()),
    })
}

#[test]
fn stop_file_drops_only_the_run_s_own_approval_and_keeps_the_plan() {
    let file = approved(plan());
    let hash = plan_hash(&plan());
    assert_eq!(
        stop_file(&file, &hash, None),
        Some(RunFile {
            plan: plan(),
            approved: None,
            stopped: None,
        })
    );
    assert_eq!(stop_file(&file, "000000000000", Some(2)), None);
    let unapproved = RunFile {
        approved: None,
        ..file
    };
    assert_eq!(stop_file(&unapproved, &hash, Some(2)), None);
}

#[test]
fn a_stop_keeps_the_stopped_step_and_a_finished_run_clears_it() {
    let hash = plan_hash(&plan());
    let stopped = stop_file(&approved(plan()), &hash, Some(2)).unwrap();
    assert_eq!(
        stopped,
        RunFile {
            plan: plan(),
            approved: None,
            stopped: stopped_at(2),
        }
    );
    let resumed = approve(&stopped, Some(&hash), true).unwrap();
    assert_eq!(resumed.stopped, stopped_at(2));
    assert_eq!(stop_file(&resumed, &hash, None).unwrap().stopped, None);
}

#[test]
fn go_resumes_at_the_stopped_step_only_for_the_same_plan() {
    let resumed = RunFile {
        stopped: stopped_at(2),
        ..approved(plan())
    };
    let started = start(&resumed, &[], &[]).unwrap();
    assert_eq!(started.driving.progress.step, 1);
    assert_eq!(
        started.next,
        Next::Watch("gh pr checks --watch".to_string())
    );
    let other = RunFile {
        stopped: Some(Stopped {
            step: 2,
            hash: "000000000000".to_string(),
        }),
        ..approved(plan())
    };
    assert_eq!(start(&other, &[], &[]).unwrap().driving.progress, at(0));
    let past_the_end = RunFile {
        stopped: stopped_at(9),
        ..approved(plan())
    };
    assert_eq!(
        start(&past_the_end, &[], &[]).unwrap().driving.progress,
        at(0)
    );
}

#[test]
fn a_kept_stop_names_the_step_and_how_to_resume() {
    let kept = RunFile {
        plan: plan(),
        approved: None,
        stopped: stopped_at(2),
    };
    assert_eq!(
        StopReason::WorkerFlagged.notice(&Run::default(), Some(&kept)),
        "send-it stopped at step 2/4: a worker asked a question or is blocked. The plan is kept. Fix the cause, show it with /send-it status, then type /send-it go to resume from step 2. /send-it off deletes it."
    );
    assert_eq!(
        StopReason::Off.notice(&Run::default(), Some(&kept)),
        "send-it off: the run stopped and its plan is deleted."
    );
    let unstopped = RunFile {
        stopped: None,
        ..kept
    };
    assert_eq!(
        StopReason::TookOver.notice(&Run::default(), Some(&unstopped)),
        format!("send-it stopped: you took over. {RESTART}")
    );
}

#[test]
fn the_running_line_shows_progress_against_the_caps() {
    assert_eq!(
        running_line(&plan(), &driving(1, Awaiting::Turn, spent(3, 0.5))),
        "Running: step 2/4, 3 of 30 turns, $0.50 of $4.00."
    );
}

#[test]
fn a_queued_send_it_off_stops_the_run_as_off_and_other_queued_text_as_took_over() {
    let file = approved(plan());
    let state = driving(0, Awaiting::Turn, Run::default());
    let stop = |lines: &[&str], typed: bool| {
        let queued: Vec<String> = lines.iter().map(|line| line.to_string()).collect();
        let ended = Ended {
            typed,
            queued: &queued,
            ..ended(&file)
        };
        tick(&state, &ended).next
    };
    for lines in [
        &["/send-it off"][..],
        &["  /send-it off  "],
        &["hello", "/send-it off"],
    ] {
        assert_eq!(stop(lines, false), Next::Stop(StopReason::Off), "{lines:?}");
    }
    assert_eq!(stop(&["/send-it off"], true), Next::Stop(StopReason::Off));
    for lines in [&["hello"][..], &["/send-it status"], &["/send-it offline"]] {
        assert_eq!(
            stop(lines, false),
            Next::Stop(StopReason::TookOver),
            "{lines:?}"
        );
    }
}
