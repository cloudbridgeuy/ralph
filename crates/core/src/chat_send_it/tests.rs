use super::*;
use crate::chat_status::WorkerState;
use crate::inbox_alert::UnknownFocus;
use crate::program_status::{Kind, Report as ProgramReport, State};
use crate::worker::Report;

fn limits() -> Limits {
    Limits {
        max_turns: 3,
        max_usd: 1.0,
    }
}

fn facts(text: &str, step: Option<Step>) -> Facts<'_> {
    Facts {
        text,
        step,
        worker_flagged: false,
        schedule_pending: false,
    }
}

fn decide_with(run: &Run, text: &str, step: Option<Step>) -> Decision {
    decide(run, &limits(), &facts(text, step))
}

#[test]
fn step_names_round_trip() {
    for step in Step::ALL {
        assert_eq!(Step::parse(step.as_str()), Some(step));
    }
    assert_eq!(Step::parse("nope"), None);
    assert_eq!(Step::FixRound.as_str(), "fix_round");
    assert_eq!(Step::DoneCleanup.as_str(), "done_cleanup");
}

#[test]
fn vetoes_push_and_merge_words() {
    for text in [
        "pushed",
        "PRs",
        "PR",
        "force-push it",
        "open a pull request",
        "Abandon GUZ-1",
        "merge it",
        "pull requests are ready",
    ] {
        assert!(vetoed_text(text), "{text}");
    }
}

#[test]
fn vetoes_approval_words() {
    for text in [
        "approve",
        "Approved",
        "approves it",
        "approving now",
        "needs approval",
        "LGTM",
        "ok, lgtm!",
    ] {
        assert!(vetoed_text(text), "{text}");
    }
}

#[test]
fn approval_lookalikes_are_not_vetoed() {
    for text in ["approvals", "disapprove", "approx", "lgtmx"] {
        assert!(!vetoed_text(text), "{text}");
    }
}

#[test]
fn allows_lookalike_words() {
    for text in [
        "pushover",
        "emerge",
        "prefix the line",
        "submerge",
        "pull the logs",
    ] {
        assert!(!vetoed_text(text), "{text}");
    }
}

#[test]
fn veto_beats_a_routine_step() {
    let run = Run::new("GUZ-1");
    assert_eq!(
        decide_with(
            &run,
            "go fast-forward main to abc and push",
            Some(Step::FixRound)
        ),
        Decision::Manual(ManualReason::VetoWord)
    );
}

#[test]
fn fast_forward_alone_is_not_vetoed() {
    assert!(!vetoed_text("fast-forward main"));
}

#[test]
fn slash_text_is_manual() {
    let run = Run::new("GUZ-1");
    assert_eq!(
        decide_with(&run, "/hold w1", Some(Step::FixRound)),
        Decision::Manual(ManualReason::SlashCommand)
    );
}

#[test]
fn missing_step_is_manual() {
    let run = Run::new("GUZ-1");
    assert_eq!(
        decide_with(&run, "go on", None),
        Decision::Manual(ManualReason::NoStep)
    );
}

#[test]
fn worker_flag_and_schedule_are_manual() {
    let run = Run::new("GUZ-1");
    let mut flagged = facts("go on", Some(Step::Checks));
    flagged.worker_flagged = true;
    assert_eq!(
        decide(&run, &limits(), &flagged),
        Decision::Manual(ManualReason::WorkerFlagged)
    );
    let mut pending = facts("go on", Some(Step::Checks));
    pending.schedule_pending = true;
    assert_eq!(
        decide(&run, &limits(), &pending),
        Decision::Manual(ManualReason::SchedulePending)
    );
}

#[test]
fn caps_stop_the_run() {
    let mut run = Run::new("GUZ-1");
    for index in 0..3 {
        run.record(Step::FixRound, &format!("go {index}"), None);
    }
    assert_eq!(
        decide_with(&run, "go on", Some(Step::FixRound)),
        Decision::Stop(StopReason::TurnCap)
    );
    let mut spent = Run::new("GUZ-1");
    spent.record(Step::FixRound, "go", Some(1.0));
    assert_eq!(
        decide_with(&spent, "go on", Some(Step::FixRound)),
        Decision::Stop(StopReason::CostCap)
    );
}

#[test]
fn repeat_of_same_step_and_text_stops() {
    let mut run = Run::new("GUZ-1");
    run.record(Step::FixRound, "  Run  the checks ", None);
    assert_eq!(
        decide_with(&run, "run the CHECKS", Some(Step::FixRound)),
        Decision::Stop(StopReason::Repeated)
    );
    assert_eq!(
        decide_with(&run, "run the checks", Some(Step::Checks)),
        Decision::Countdown
    );
}

#[test]
fn clean_fix_round_counts_down() {
    let run = Run::new("GUZ-1");
    assert_eq!(
        decide_with(&run, "start the next fix round", Some(Step::FixRound)),
        Decision::Countdown
    );
}

#[test]
fn record_accumulates_and_ignores_missing_cost() {
    let mut run = Run::new("GUZ-1");
    run.record(Step::Checks, "Go   ON", Some(0.5));
    run.record(Step::Reinstall, "again", None);
    run.record(Step::Reinstall, "more", Some(0.25));
    assert_eq!(run.turns, 3);
    assert!((run.spent_usd - 0.75).abs() < 1e-9);
    assert_eq!(run.sent[0], (Step::Checks, "go on".to_string()));
}

#[test]
fn notices_point_back_to_the_ticket() {
    let run = Run::new("GUZ-9");
    for notice in [
        ManualReason::NoStep.notice(&run),
        ManualReason::SlashCommand.notice(&run),
        ManualReason::VetoWord.notice(&run),
        ManualReason::WorkerFlagged.notice(&run),
        ManualReason::SchedulePending.notice(&run),
        StopReason::TurnCap.notice(&run),
        StopReason::CostCap.notice(&run),
        StopReason::Repeated.notice(&run),
    ] {
        assert!(notice.ends_with("/send-it GUZ-9"), "{notice}");
    }
}

#[test]
fn send_it_args_parse_to_arm_off_or_status() {
    assert_eq!(SendIt::parse(""), Some(SendIt::Status));
    assert_eq!(SendIt::parse("  "), Some(SendIt::Status));
    assert_eq!(SendIt::parse(" off "), Some(SendIt::Off));
    assert_eq!(
        SendIt::parse("GUZ-1162"),
        Some(SendIt::Arm("GUZ-1162".to_string()))
    );
    assert_eq!(SendIt::parse("GUZ-1 now"), None);
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
fn the_status_text_names_ticket_turns_spend_and_caps() {
    let mut run = Run::new("GUZ-9");
    run.record(Step::Checks, "run the checks", Some(0.5));
    let text = status_text(Some(&run), &limits_for_text(), 10);
    assert_eq!(
        text,
        "send-it armed for GUZ-9: 1 of 15 turns, $0.50 of $5.00. Countdown: 10s."
    );
}

#[test]
fn the_status_text_when_off_says_so() {
    assert_eq!(
        status_text(None, &limits_for_text(), 10),
        "send-it is off. Arm it with /send-it <ticket>."
    );
}

#[test]
fn the_cancel_and_terminal_notices_end_with_the_resume_hint() {
    let run = Run::new("GUZ-9");
    assert_eq!(
        cancel_notice(&run),
        "send-it cancelled by a key press. Resume with /send-it GUZ-9"
    );
    assert!(no_terminal_notice(&run).ends_with("Resume with /send-it GUZ-9"));
    assert_eq!(
        closed_notice(&run),
        "send-it off: worker GUZ-9 is no longer listed."
    );
}

fn status_with(workers: &[&str]) -> Status {
    Status {
        workers: workers
            .iter()
            .map(|id| WorkerState {
                id: (*id).to_string(),
                state: "idle".to_string(),
            })
            .collect(),
        ..Status::default()
    }
}

#[test]
fn a_ticket_matching_a_hold_target_is_tracked() {
    assert!(tracks_worker("GUZ-1", &status_with(&["GUZ-1"])));
    assert!(!tracks_worker("GUZ-2", &status_with(&["GUZ-1"])));
}

#[test]
fn a_tracked_ticket_closes_when_it_is_no_longer_listed() {
    assert!(worker_closed(true, "GUZ-1", &status_with(&[])));
    assert!(!worker_closed(true, "GUZ-1", &status_with(&["GUZ-1"])));
    assert!(!worker_closed(false, "GUZ-1", &status_with(&[])));
}

#[test]
fn a_blocked_alert_carries_the_notice_and_follows_the_config() {
    let config = AlertConfig {
        sound: true,
        notification: true,
        unknown_focus: UnknownFocus::Alert,
    };
    assert_eq!(
        blocked_alert("send-it paused", &config),
        Some(Alert {
            sound: true,
            notification: Some("send-it paused".to_string()),
        })
    );
    let quiet = AlertConfig {
        sound: false,
        notification: false,
        ..config
    };
    assert_eq!(blocked_alert("send-it paused", &quiet), None);
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
fn the_blocked_frame_report_is_a_question_with_the_notice() {
    assert_eq!(
        blocked_report("send-it paused"),
        ProgramReport {
            state: State::Blocked(Kind::Question),
            msg: Some("send-it paused".to_string()),
        }
    );
}
