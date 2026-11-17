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
        &driving(0, Awaiting::Report("E1".to_string()), spent(1, 0.1)),
        &Ended {
            drained: &[worker_report("E1", "done")],
            ..ended(&file)
        },
    );
    assert_eq!(mid.shown_met(), mid.met.as_deref());
    assert!(mid.shown_met().is_some());
}
