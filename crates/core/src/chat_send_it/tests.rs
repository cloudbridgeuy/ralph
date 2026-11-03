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

fn facts(text: &str) -> Facts<'_> {
    Facts {
        text,
        worker_flagged: false,
        schedule_pending: false,
    }
}

fn decide_with(run: &Run, text: &str) -> Decision {
    decide(run, &limits(), &facts(text))
}

const FIX_ROUND: &str = "go 1172: full fix, blocked_alert follows the Rex gate, failing test first, new commit, four checks, reinstall";

#[test]
fn a_real_fix_round_counts_down_with_no_step_tag() {
    let run = Run::default();
    assert_eq!(decide_with(&run, FIX_ROUND), Decision::Countdown);
}

#[test]
fn real_decision_replies_wait_on_the_person() {
    let run = Run::default();
    for (text, word) in [
        ("go merge 1172", "merge"),
        (
            "go: reset local main to origin/main dropping our 27b9664, keep worker/GUZ-1176, run the checks, reinstall, rerun the case B repro",
            "reset",
        ),
        (
            "go: mark 1175 Done after CI, close the worker, delete /tmp/guz1175",
            "mark",
        ),
        ("go: close the 1202 worker", "close"),
        ("I quit the A4 test chat; delete the rest", "i"),
        (
            "go 792 live check 1-6 in tmux on /tmp/guz792, Haiku for the test chat (stop if first call isn't Haiku), ≤6 turns, Ctrl-C via send-keys, stop on first FAIL, keep raw logs",
            "haiku",
        ),
        ("go 1191 plan-first", "plan"),
        ("open a pull request", "pull request"),
        ("ok, LGTM!", "lgtm"),
        ("Done", "done"),
    ] {
        assert_eq!(
            decide_with(&run, text),
            Decision::Manual(ManualReason::Decision(word)),
            "{text}"
        );
    }
}

#[test]
fn a_reply_with_no_routine_action_waits_on_the_person() {
    let run = Run::default();
    assert_eq!(
        decide_with(
            &run,
            "go 1166 live check steps 1-5 on /tmp/guz1166 only, restore the chmod, stop on first FAIL, keep raw logs"),
        Decision::Manual(ManualReason::NotRoutine)
    );
}

#[test]
fn routine_words_count_as_whole_tokens_only() {
    for text in [
        "run the checks",
        "fix the typo",
        "rebase on main",
        "retry the tests",
        "cargo fmt and clippy",
        "lint it",
        "reinstall",
    ] {
        assert_eq!(text_reason(text), None, "{text}");
    }
    for text in [
        "prefix the line",
        "checksum it",
        "contest",
        "pushover",
        "emerge",
    ] {
        assert_eq!(text_reason(text), Some(ManualReason::NotRoutine), "{text}");
    }
}

#[test]
fn decision_words_match_in_any_case_and_form() {
    for (text, word) in [
        ("fix it and PUSHED", "pushed"),
        ("fix, then PRs", "prs"),
        ("fix it, approving now", "approving"),
        ("fix it, needs approval", "approval"),
        ("fix it then force-push", "force"),
        ("fix it, then delete /tmp/guz1175", "delete"),
        ("fix it, pull requests are ready", "pull request"),
    ] {
        assert_eq!(
            text_reason(text),
            Some(ManualReason::Decision(word)),
            "{text}"
        );
    }
    assert_eq!(text_reason("fix the approvals list"), None);
    assert_eq!(text_reason("fix it and pull the logs"), None);
}

#[test]
fn slash_text_is_manual() {
    let run = Run::default();
    assert_eq!(
        decide_with(&run, "/hold w1"),
        Decision::Manual(ManualReason::SlashCommand)
    );
    assert_eq!(
        decide_with(&run, "/hold w1 w2"),
        Decision::Manual(ManualReason::SlashCommand)
    );
}

#[test]
fn worker_flag_and_schedule_are_manual() {
    let run = Run::default();
    let mut flagged = facts(FIX_ROUND);
    flagged.worker_flagged = true;
    assert_eq!(
        decide(&run, &limits(), &flagged),
        Decision::Manual(ManualReason::WorkerFlagged)
    );
    let mut pending = facts(FIX_ROUND);
    pending.schedule_pending = true;
    assert_eq!(
        decide(&run, &limits(), &pending),
        Decision::Manual(ManualReason::SchedulePending)
    );
}

#[test]
fn caps_stop_the_run() {
    let mut run = Run::default();
    for index in 0..3 {
        run.record(&format!("go {index}"), None);
    }
    assert_eq!(
        decide_with(&run, "fix it"),
        Decision::Stop(StopReason::TurnCap)
    );
    let mut spent = Run::default();
    spent.record("go", Some(1.0));
    assert_eq!(
        decide_with(&spent, "fix it"),
        Decision::Stop(StopReason::CostCap)
    );
}

#[test]
fn a_repeated_text_stops() {
    let mut run = Run::default();
    run.record("  Run  the checks ", None);
    assert_eq!(
        decide_with(&run, "run the CHECKS"),
        Decision::Stop(StopReason::Repeated)
    );
    assert_eq!(decide_with(&run, "run the tests"), Decision::Countdown);
}

#[test]
fn clean_fix_round_counts_down() {
    let run = Run::default();
    assert_eq!(
        decide_with(&run, "start the next fix round"),
        Decision::Countdown
    );
}

#[test]
fn record_accumulates_and_ignores_missing_cost() {
    let mut run = Run::default();
    run.record("Go   ON", Some(0.5));
    run.record("again", None);
    run.record("more", Some(0.25));
    assert_eq!(run.turns, 3);
    assert!((run.spent_usd - 0.75).abs() < 1e-9);
    assert_eq!(run.sent[0], "go on".to_string());
}

#[test]
fn a_manual_reply_waits_and_names_why() {
    for (reason, why) in [
        (ManualReason::SlashCommand, "a slash command"),
        (ManualReason::Decision("merge"), "a decision (merge)"),
        (ManualReason::NotRoutine, "no routine action"),
        (ManualReason::WorkerFlagged, "a worker question or block"),
        (ManualReason::SchedulePending, "a pending schedule"),
    ] {
        assert_eq!(reason.notice(), format!("send-it waits on you: {why}."));
    }
}

#[test]
fn stop_notices_say_how_to_turn_it_on_again() {
    let run = Run::default();
    for notice in [
        StopReason::TurnCap.notice(&run),
        StopReason::CostCap.notice(&run),
        StopReason::Repeated.notice(&run),
    ] {
        assert!(notice.starts_with("send-it off: "), "{notice}");
        assert!(
            notice.ends_with("Turn it on again with /send-it."),
            "{notice}"
        );
    }
}

#[test]
fn send_it_args_parse_to_on_or_off() {
    assert_eq!(SendIt::parse(""), Some(SendIt::On));
    assert_eq!(SendIt::parse("  "), Some(SendIt::On));
    assert_eq!(SendIt::parse(" off "), Some(SendIt::Off));
    assert_eq!(SendIt::parse("OFF"), Some(SendIt::Off));
    assert_eq!(SendIt::parse(" Off "), Some(SendIt::Off));
    assert_eq!(SendIt::parse("GUZ-1162"), None);
    assert_eq!(SendIt::parse("off now"), None);
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
fn remaining_seconds_round_up_and_reach_zero() {
    assert_eq!(remaining_seconds(0, 10), 10);
    assert_eq!(remaining_seconds(1, 10), 10);
    assert_eq!(remaining_seconds(999, 10), 10);
    assert_eq!(remaining_seconds(1_000, 10), 9);
    assert_eq!(remaining_seconds(9_001, 10), 1);
    assert_eq!(remaining_seconds(10_000, 10), 0);
    assert_eq!(remaining_seconds(99_000, 10), 0);
    assert_eq!(remaining_seconds(0, 0), 0);
}

#[test]
fn the_frame_rewrites_one_line_and_names_the_text_and_the_cancel() {
    let frame = countdown_frame(7, "run the checks", 80, false);
    assert_eq!(
        frame,
        "\r\x1b[2Ksend-it in 7s: run the checks  (any key cancels)"
    );
}

#[test]
fn the_frame_has_no_escape_other_than_the_clear_when_color_is_off() {
    let frame = countdown_frame(3, "go", 80, false);
    assert_eq!(frame.matches('\x1b').count(), 1);
}

#[test]
fn the_frame_is_dim_when_color_is_on() {
    let frame = countdown_frame(3, "go", 80, true);
    assert!(frame.starts_with("\r\x1b[2K\x1b[2m"));
    assert!(frame.ends_with("\x1b[0m"));
}

#[test]
fn a_long_text_is_cut_to_fit_one_row() {
    let text = "x".repeat(300);
    let frame = countdown_frame(10, &text, 60, false);
    let visible = frame.trim_start_matches("\r\x1b[2K");
    assert!(visible.chars().count() < 60, "{visible}");
    assert!(visible.contains('…'));
    assert!(visible.ends_with("(any key cancels)"));
}

#[test]
fn a_narrow_row_still_keeps_the_cancel_hint() {
    let frame = countdown_frame(10, "abcdefghij", 10, false);
    assert!(frame.ends_with("(any key cancels)"));
}

#[test]
fn the_auto_line_carries_the_send_it_prefix() {
    assert_eq!(auto_line("run the checks"), "[send-it] run the checks");
}

fn limits_for_text() -> Limits {
    Limits {
        max_turns: 15,
        max_usd: 5.0,
    }
}

#[test]
fn the_status_text_names_turns_spend_and_caps() {
    let mut run = Run::default();
    run.record("run the checks", Some(0.5));
    let text = status_text(Some(&run), &limits_for_text(), 10);
    assert_eq!(
        text,
        "send-it on: 1 of 15 turns, $0.50 of $5.00. Countdown: 10s."
    );
}

#[test]
fn the_status_text_when_off_says_so() {
    assert_eq!(
        status_text(None, &limits_for_text(), 10),
        "send-it is off. Turn it on with /send-it."
    );
}

#[test]
fn the_cancel_and_terminal_notices_say_how_to_turn_it_on_again() {
    assert_eq!(
        cancel_notice(),
        "send-it cancelled by a key press. Turn it on again with /send-it."
    );
    assert_eq!(
        no_terminal_notice(),
        "send-it off: the countdown needs a terminal. Turn it on again with /send-it."
    );
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
