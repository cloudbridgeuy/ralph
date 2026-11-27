use super::*;
use unicode_width::UnicodeWidthStr;

fn finished(turns: u32, usd: f64, last_met: Option<&str>, pending: &[&str]) -> Driving {
    Driving {
        run: spent(turns, usd),
        tally: Tally {
            last_met: last_met.map(str::to_string),
            pending: pending.iter().map(ToString::to_string).collect(),
        },
        ..driving(3, Awaiting::Turn, Run::default())
    }
}

fn body(block: &str) -> Vec<&str> {
    block.lines().skip(1).collect()
}

#[test]
fn a_done_block_lists_changes_pending_and_one_next_step() {
    let driving = finished(
        2,
        0.27,
        Some("all 3 findings fixed"),
        &["push main (2 ahead)", "run lint"],
    );
    let block = done_block(&plan(), &driving, 60);
    let header = block.lines().next().unwrap();
    assert!(
        header.starts_with("━━ send-it DONE · 4/4 steps · 2 turns · $0.27 ━"),
        "{header}"
    );
    assert_eq!(header.width(), 60, "{header}");
    assert_eq!(
        body(&block),
        [
            "Changed   ✓ 1-3 (3 steps)",
            "          ✓ 4 address review — all 3 findings fixed",
            "Pending   1. push main (2 ahead) [model]",
            "on you    2. run lint [model]",
            "Next      decide item 1: push main (2 ahead)",
        ]
    );
}

#[test]
fn a_done_block_without_evidence_or_pending_has_nothing_to_do() {
    let block = done_block(&plan(), &finished(1, 0.1, None, &[]), 60);
    assert!(
        block.lines().next().unwrap().contains("1 turn · $0.10"),
        "{block}"
    );
    assert_eq!(
        body(&block),
        [
            "Changed   ✓ 1-4 (4 steps)",
            "Pending   nothing",
            "Next      nothing to do · /send-it off deletes the plan",
        ]
    );
}

#[test]
fn a_done_block_wraps_long_text_under_its_label() {
    let evidence = "the tests pass and the lint is clean and the review is closed";
    let block = done_block(&plan(), &finished(2, 0.5, Some(evidence), &[]), 50);
    for line in block.lines() {
        assert!(line.width() <= 50, "{line}");
    }
    let lines = body(&block);
    assert!(
        lines[1].starts_with("          ✓ 4 address review"),
        "{block}"
    );
    assert!(lines[2].starts_with("          "), "{block}");
    assert!(!lines[2].trim().starts_with('✓'), "{block}");
}

#[test]
fn the_final_met_line_is_not_shown_when_the_run_is_done() {
    let file = approved(plan());
    let met = judged(true, "all 3 findings fixed");
    let finished = tick(
        &driving(3, Awaiting::Verdict, spent(2, 0.5)),
        &Ended {
            step_result: Some(&met),
            ..ended(&file)
        },
    );
    assert_eq!(finished.next, Next::Done);
    assert!(finished.met.is_some());
    assert_eq!(finished.shown_met(), None);
    let mid = tick(
        &driving(0, awaiting_report("E1"), spent(1, 0.1)),
        &Ended {
            drained: &[worker_report("E1", "done")],
            ..ended(&file)
        },
    );
    assert_eq!(mid.shown_met(), mid.met.as_deref());
    assert!(mid.shown_met().is_some());
}

fn kept_at(step: usize) -> RunFile {
    RunFile {
        plan: plan(),
        approved: None,
        stopped: stopped_at(step),
    }
}

fn stopped_block(
    reason: &StopReason,
    step: usize,
    turns: u32,
    usd: f64,
    pending: &[&str],
) -> String {
    stop_block(
        reason,
        Some(&kept_at(step)),
        &finished(turns, usd, None, pending),
        80,
    )
}

fn header_of(block: &str) -> &str {
    block.lines().next().unwrap()
}

#[test]
fn a_flagged_stop_block_names_the_worker_and_how_to_answer_it() {
    let reason = StopReason::WorkerFlagged {
        worker: Sender::parse("E1").unwrap(),
        line: "Keep the old flag name?".to_string(),
    };
    let block = stopped_block(&reason, 2, 4, 0.61, &[]);
    assert!(
        header_of(&block).starts_with(
            "━━ send-it STOPPED at 2/4 · worker E1 needs an answer · 4 turns · $0.61 ━"
        ),
        "{block}"
    );
    assert_eq!(header_of(&block).width(), 80);
    assert_eq!(
        body(&block),
        [
            "Changed   ✓ 1 (1 step)",
            "          ✗ 2 wait for CI — E1: \"Keep the old flag name?\"",
            "Pending   1. answer E1",
            "Next      @E1 <your answer> · then /send-it go resumes at step 2",
        ]
    );
}

#[test]
fn a_judge_rounds_stop_block_shows_the_evidence_once() {
    let reason = StopReason::JudgeRounds {
        step: 4,
        evidence: "finding 2 is open.".to_string(),
    };
    let block = stopped_block(&reason, 4, 7, 1.4, &[]);
    assert!(
        header_of(&block)
            .contains("STOPPED at 4/4 · not met after 2 judge rounds · 7 turns · $1.40"),
        "{block}"
    );
    assert_eq!(
        body(&block),
        [
            "Changed   ✓ 1-3 (3 steps)",
            "          ✗ 4 address review — finding 2 is open",
            "Pending   1. act on the judge evidence or edit step 4",
            "Next      /send-it go · resumes at step 4",
        ]
    );
    assert_eq!(block.matches("finding 2 is open").count(), 1);
}

#[test]
fn a_cap_stop_block_names_the_cap_and_resumes_with_fresh_caps() {
    let block = stopped_block(&StopReason::TurnCap, 3, 15, 3.1, &[]);
    assert!(
        header_of(&block).contains("STOPPED at 3/4 · turn cap reached (15 turns) · $3.10"),
        "{block}"
    );
    assert_eq!(
        body(&block),
        [
            "Changed   ✓ 1-2 (2 steps)",
            "          … 3 run the checks — not finished",
            "Pending   nothing; the cause is the cap",
            "Next      /send-it go · resumes at step 3 with fresh caps",
        ]
    );
    let cost = stopped_block(&StopReason::CostCap, 3, 6, 4.0, &[]);
    assert!(
        header_of(&cost).contains("cost cap reached ($4.00)"),
        "{cost}"
    );
}

#[test]
fn a_stop_block_lists_its_own_item_before_the_model_items() {
    let block = stopped_block(&StopReason::Unmet { step: 2 }, 2, 3, 0.5, &["push main"]);
    assert_eq!(
        body(&block),
        [
            "Changed   ✓ 1 (1 step)",
            "          ✗ 2 wait for CI — exit criterion not met",
            "Pending   1. fix step 2 or edit it",
            "on you    2. push main [model]",
            "Next      /send-it go · resumes at step 2",
        ]
    );
}

#[test]
fn a_took_over_stop_at_step_one_has_no_collapsed_line_and_nothing_pending() {
    let block = stopped_block(&StopReason::TookOver, 1, 1, 0.1, &[]);
    assert_eq!(
        body(&block),
        [
            "Changed   … 1 collect evidence — not finished",
            "Pending   nothing",
            "Next      /send-it go · resumes at step 1",
        ]
    );
}

#[test]
fn a_stop_block_without_a_kept_plan_points_at_status_and_go() {
    let block = stop_block(
        &StopReason::TookOver,
        None,
        &finished(2, 0.2, None, &[]),
        80,
    );
    assert!(
        header_of(&block).starts_with("━━ send-it STOPPED · you took over · 2 turns · $0.20 ━"),
        "{block}"
    );
    assert_eq!(
        body(&block),
        [
            "Pending   nothing",
            "Next      /send-it status, then /send-it go"
        ]
    );
}

#[test]
fn an_off_stop_has_no_block_only_its_notice() {
    let block = stop_block(&StopReason::Off, None, &finished(2, 0.2, None, &[]), 80);
    assert_eq!(
        block,
        "send-it off: the run stopped and its plan is deleted."
    );
}

fn check_failed_at_three(detail: Option<&str>) -> StopReason {
    StopReason::CheckFailed {
        step: 3,
        detail: detail.map(str::to_string),
    }
}

#[test]
fn a_check_failed_block_names_the_command_and_shows_its_output_lines() {
    let reason = check_failed_at_three(Some(
        "exited with code 101; last output: error[E0308]: mismatched types / --> crates/core/src/x.rs:41:9",
    ));
    let block = stopped_block(&reason, 3, 3, 0.48, &[]);
    assert!(
        header_of(&block).starts_with(
            "━━ send-it CHECK FAILED at 3/4 · `cargo xtask lint` exited with code 101 · 3 turns · $0.48 ━"
        ),
        "{block}"
    );
    assert_eq!(
        body(&block),
        [
            "Changed   ✓ 1-2 (2 steps)",
            "          ✗ 3 run the checks",
            "Output    error[E0308]: mismatched types",
            "          --> crates/core/src/x.rs:41:9",
            "Pending   1. fix the check failure",
            "Next      /send-it go · runs the check again at step 3",
        ]
    );
}

#[test]
fn a_check_failed_block_without_detail_has_no_output_row() {
    let block = stopped_block(&check_failed_at_three(None), 3, 3, 0.48, &[]);
    assert!(
        header_of(&block)
            .contains("CHECK FAILED at 3/4 · `cargo xtask lint` did not exit 0 · 3 turns"),
        "{block}"
    );
    assert!(!block.contains("Output"), "{block}");
    assert!(body(&block)[1].ends_with("✗ 3 run the checks"), "{block}");
}

#[test]
fn a_check_that_never_started_has_a_check_failed_header_and_its_error() {
    let reason = StopReason::CheckNotStarted {
        step: 3,
        error: "no such file.".to_string(),
    };
    let block = stopped_block(&reason, 3, 2, 0.3, &[]);
    assert!(
        header_of(&block).contains("CHECK FAILED at 3/4 · `cargo xtask lint` could not start"),
        "{block}"
    );
    assert_eq!(
        body(&block),
        [
            "Changed   ✓ 1-2 (2 steps)",
            "          ✗ 3 run the checks — no such file",
            "Pending   1. fix the check command",
            "Next      /send-it go · runs the check again at step 3",
        ]
    );
}

#[test]
fn a_failed_watch_step_names_the_watch_command() {
    let reason = StopReason::CheckFailed {
        step: 2,
        detail: Some("exited with code 1".to_string()),
    };
    let block = stopped_block(&reason, 2, 2, 0.3, &[]);
    assert!(
        header_of(&block)
            .contains("CHECK FAILED at 2/4 · `gh pr checks --watch` exited with code 1"),
        "{block}"
    );
}
