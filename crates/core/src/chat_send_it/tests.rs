use super::*;
use crate::inbox_alert::UnknownFocus;
use crate::program_status::{Kind, Report as ProgramReport, State};
use crate::worker::Report;

fn limits() -> Limits {
    Limits {
        max_turns: 3,
        max_usd: 1.0,
    }
}

#[test]
fn stop_notices_say_how_to_turn_it_on_again() {
    let run = Run::default();
    for notice in [
        StopReason::TurnCap.notice(&run),
        StopReason::CostCap.notice(&run),
    ] {
        assert!(notice.starts_with("send-it off: "), "{notice}");
        assert!(
            notice.ends_with("Turn it on again with /send-it."),
            "{notice}"
        );
    }
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
    assert!(worker_flagged(&[report("done"), report("question")], &[]));
    assert!(worker_flagged(&[report("blocked")], &[]));
    assert!(!worker_flagged(&[report("done"), report("progress")], &[]));
    assert!(!worker_flagged(&[], &[]));
}

#[test]
fn reports_drained_during_the_turn_also_flag_a_worker() {
    assert!(worker_flagged(&[], &[report("question")]));
    assert!(worker_flagged(&[], &[report("done"), report("blocked")]));
    assert!(worker_flagged(&[report("question")], &[report("done")]));
    assert!(!worker_flagged(&[report("done")], &[report("progress")]));
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
        judge_rounds: 0,
    }
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
    let line_2 = "[send-it 2/4] wait for CI · exit: watch `gh pr checks --watch` exits 0";
    for (report, expected) in [
        (
            worker_report("E1", "done"),
            (at(1), Next::Send(line_2.to_string())),
        ),
        (worker_report("X9", "done"), (at(0), Next::Wait)),
        (
            worker_report("X9", "question"),
            (at(0), Next::Stop(StopReason::WorkerFlagged)),
        ),
        (
            worker_report("E1", "blocked"),
            (at(0), Next::Stop(StopReason::WorkerFlagged)),
        ),
        (
            worker_report("E1", "exited"),
            (at(0), Next::Stop(StopReason::Unmet { step: 1 })),
        ),
    ] {
        assert_eq!(advance(&plan, at(0), Event::Report(&report)), expected);
    }
}

#[test]
fn exit_codes_settle_watch_and_check_steps_only() {
    let plan = plan();
    let line_3 = "[send-it 3/4] run the checks · exit: `cargo xtask lint` exits 0";
    assert_eq!(
        advance(&plan, at(1), Event::Exited(0)),
        (at(2), Next::Send(line_3.to_string()))
    );
    assert_eq!(
        advance(&plan, at(2), Event::Exited(0)),
        (at(3), Next::Send(STEP_4.to_string()))
    );
    assert_eq!(
        advance(&plan, at(2), Event::Exited(1)),
        (at(2), Next::Stop(StopReason::Unmet { step: 3 }))
    );
    assert_eq!(advance(&plan, at(0), Event::Exited(0)), (at(0), Next::Wait));
}

#[test]
fn verdicts_finish_retry_then_stop_after_the_judge_rounds() {
    let plan = plan();
    assert_eq!(
        advance(&plan, at(3), Event::Verdict(true)),
        (at(3), Next::Done)
    );
    let retried = Progress {
        step: 3,
        judge_rounds: 1,
    };
    assert_eq!(
        advance(&plan, at(3), Event::Verdict(false)),
        (retried, Next::Send(STEP_4.to_string()))
    );
    assert_eq!(
        advance(&plan, retried, Event::Verdict(false)),
        (retried, Next::Stop(StopReason::JudgeRounds { step: 4 }))
    );
    assert_eq!(
        advance(&plan, at(0), Event::Verdict(true)),
        (at(0), Next::Wait)
    );
}

#[test]
fn new_stop_notices_name_the_step() {
    let run = Run::default();
    assert_eq!(
        StopReason::Unmet { step: 2 }.notice(&run),
        "send-it off: step 2 missed its exit criterion. Turn it on again with /send-it."
    );
    assert!(StopReason::WorkerFlagged
        .notice(&run)
        .contains("a worker asked a question"));
    assert!(StopReason::JudgeRounds { step: 4 }
        .notice(&run)
        .contains("step 4 was still not met"));
}

#[test]
fn usage_names_the_instructions_form_and_the_default_caps() {
    let text = usage(&Limits {
        max_turns: 15,
        max_usd: 5.0,
    });
    assert!(text.starts_with("Usage: /send-it <instructions>"), "{text}");
    assert!(text.contains("exit criterion"), "{text}");
    assert!(text.contains("Default caps: 15 turns, $5.00."), "{text}");
}

#[test]
fn send_it_keywords_count_only_as_the_whole_argument() {
    assert_eq!(parse_send_it_args(""), SendItCommand::Usage);
    assert_eq!(parse_send_it_args("  "), SendItCommand::Usage);
    assert_eq!(parse_send_it_args(" edit "), SendItCommand::Edit);
    assert_eq!(parse_send_it_args("go"), SendItCommand::Go);
    assert_eq!(parse_send_it_args("status"), SendItCommand::Status);
    assert_eq!(parse_send_it_args("off"), SendItCommand::Off);
    assert_eq!(
        parse_send_it_args(" go ship GUZ-1 "),
        SendItCommand::Propose("go ship GUZ-1".to_string())
    );
}

fn run_file(approved: Option<String>) -> RunFile {
    RunFile {
        plan: plan(),
        approved,
    }
}

#[test]
fn a_run_file_round_trips_through_toml_with_its_approval() {
    for approved in [None, Some(plan_hash(&plan()))] {
        let file = run_file(approved);
        let text = write_run(&file).unwrap();
        assert_eq!(read_run(&text, limits()), Ok(file), "{text}");
    }
    assert!(!write_run(&run_file(None)).unwrap().contains("approved"));
}

const HAND_WRITTEN: &str = r#"
goal = "ship GUZ-1"

[[steps]]
do = "run the checks"
criterion = { kind = "check_passes", command = "cargo xtask lint" }
"#;

#[test]
fn a_hand_written_run_file_takes_the_default_caps() {
    let file = read_run(HAND_WRITTEN, limits()).unwrap();
    assert_eq!(file.plan.limits, limits());
    assert_eq!(file.plan.judge_rounds, DEFAULT_JUDGE_ROUNDS);
    assert_eq!(file.approved, None);
}

#[test]
fn a_bad_run_file_says_what_is_wrong() {
    assert!(read_run("goal = ", limits())
        .unwrap_err()
        .starts_with("run.toml is malformed"));
    let no_criterion = "goal = \"g\"\n[[steps]]\ndo = \"x\"\n";
    assert_eq!(
        read_run(no_criterion, limits()),
        Err("Step 1 has no exit criterion. Give each step a check ralph can measure.".to_string())
    );
}

#[test]
fn the_plan_hash_ignores_layout_and_follows_content() {
    let hand = read_run(HAND_WRITTEN, limits()).unwrap();
    let hash = plan_hash(&hand.plan);
    assert_eq!(hash.len(), 12);
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    let rewritten = read_run(&write_run(&hand).unwrap(), limits()).unwrap();
    assert_eq!(plan_hash(&rewritten.plan), hash);
    let mut changed = hand.plan.clone();
    changed.steps[0].task = "run the tests".to_string();
    assert_ne!(plan_hash(&changed), hash);
}

#[test]
fn go_approves_only_the_shown_plan_typed_at_the_prompt() {
    let file = run_file(None);
    let hash = plan_hash(&file.plan);
    assert!(!file.is_approved());
    assert!(approve(&file, Some(&hash), false)
        .unwrap_err()
        .contains("type it at the prompt"));
    for shown in [None, Some("0123456789ab")] {
        assert!(approve(&file, shown, true)
            .unwrap_err()
            .contains("/send-it status"));
    }
    let approved = approve(&file, Some(&hash), true).unwrap();
    assert_eq!(approved.approved, Some(hash));
    assert!(approved.is_approved());
}

#[test]
fn an_edited_plan_loses_its_approval() {
    let mut file = approve(&run_file(None), Some(&plan_hash(&plan())), true).unwrap();
    assert!(file.is_approved());
    file.plan.steps.pop();
    assert!(!file.is_approved());
}

#[test]
fn status_shows_the_plan_its_hash_and_whether_it_is_approved() {
    let file = run_file(None);
    let hash = plan_hash(&file.plan);
    let status = render_status(&file);
    assert!(status.starts_with(&render_run(&file.plan)), "{status}");
    assert!(status.ends_with(&format!(
        "Plan {hash}: not approved. Type /send-it go to approve this plan."
    )));
    let approved = approve(&file, Some(&hash), true).unwrap();
    assert!(render_status(&approved).ends_with(&format!("Plan {hash}: approved.")));
}

#[test]
fn only_an_approved_run_blocks_a_new_proposal() {
    let draft = run_file(None);
    assert_eq!(propose_refusal(None), None);
    assert_eq!(propose_refusal(Some(&draft)), None);
    let hash = plan_hash(&draft.plan);
    let approved = approve(&draft, Some(&hash), true).unwrap();
    assert_eq!(
        propose_refusal(Some(&approved)),
        Some(format!(
            "Plan {hash} is approved and active. Stop it with /send-it off first."
        ))
    );
}

#[test]
fn the_propose_prompt_asks_for_one_propose_run_call_and_no_work() {
    let prompt = propose_prompt("ship GUZ-1 and merge");
    assert!(prompt.ends_with("\n\nship GUZ-1 and merge"), "{prompt}");
    assert!(prompt.contains("Call `propose_run` once"), "{prompt}");
    assert!(prompt.contains("Do not start the work"), "{prompt}");
}

#[test]
fn the_propose_run_tool_requires_a_criterion_on_every_step() {
    let tool = propose_run_tool();
    assert_eq!(tool["name"], "propose_run");
    let schema = &tool["inputSchema"];
    assert_eq!(schema["required"], serde_json::json!(["goal", "steps"]));
    assert_eq!(
        schema["properties"]["steps"]["items"]["required"],
        serde_json::json!(["do", "criterion"])
    );
}
