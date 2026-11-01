use super::*;

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
