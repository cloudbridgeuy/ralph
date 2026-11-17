use super::*;

const FIXTURE: &str = include_str!("fixtures/old_rows.jsonl");

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 1e-9
}

fn row(kind: &str, run: &str, process_turn: Option<u64>, cost: Option<f64>) -> UsageLine {
    UsageLine {
        kind: kind.to_string(),
        run: Some(run.to_string()),
        process_turn,
        cost_usd: cost,
        input_tokens: 1,
        ..UsageLine::default()
    }
}

fn old_turn(run: &str, process_turn: u64, total: f64) -> UsageLine {
    row("turn", run, Some(process_turn), Some(total))
}

fn old_ping(run: &str, total: f64) -> UsageLine {
    row("ping", run, None, Some(total))
}

fn new_turn(run: &str, process_turn: u64, own: f64) -> UsageLine {
    UsageLine {
        ledger_v: Some(2),
        ..old_turn(run, process_turn, own)
    }
}

fn new_ping(run: &str, own: f64) -> UsageLine {
    UsageLine {
        ledger_v: Some(2),
        ..old_ping(run, own)
    }
}

fn owns(rows: Vec<UsageLine>) -> Vec<Option<f64>> {
    normalize_costs(rows)
        .iter()
        .map(|row| row.cost_usd)
        .collect()
}

fn assert_owns(actual: Vec<Option<f64>>, expected: &[Option<f64>]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        match (actual, expected) {
            (Some(actual), Some(expected)) => assert!(close(*actual, *expected), "{actual}"),
            (actual, expected) => assert_eq!(actual, expected),
        }
    }
}

fn summed(rows: &[UsageLine]) -> f64 {
    rows.iter().filter_map(|row| row.cost_usd).sum()
}

fn fixture_rows() -> Vec<UsageLine> {
    FIXTURE
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn normalised_fixture_sums_to_the_golden_session_costs() {
    let rows = normalize_costs(fixture_rows());
    let of = |run: &str| -> Vec<UsageLine> {
        rows.iter()
            .filter(|row| {
                row.run.as_deref() == Some(run) && matches!(row.kind.as_str(), "turn" | "ping")
            })
            .cloned()
            .collect()
    };
    let a = of("100-2026-11-01T09:00:00+00:00");
    let b = of("200-2026-11-01T10:00:00+00:00");
    assert!(close(summed(&a), 3.1));
    assert!(close(summed(&b), 1.26));
    let cost = session_cost(&a).unwrap();
    assert!(close(cost.total, 3.1));
    assert!(close(cost.delta.unwrap(), 0.08));
    assert!(!cost.missing);
}

#[test]
fn normalising_leaves_other_kinds_untouched() {
    let rows = normalize_costs(fixture_rows());
    let originals = fixture_rows();
    for (after, before) in rows.iter().zip(&originals) {
        if !matches!(before.kind.as_str(), "turn" | "ping") {
            assert_eq!(after, before);
        }
    }
}

#[test]
fn old_rows_of_one_process_become_their_growth() {
    assert_owns(
        owns(vec![
            old_turn("r", 1, 0.5),
            old_ping("r", 0.6),
            old_turn("r", 2, 1.0),
        ]),
        &[Some(0.5), Some(0.1), Some(0.4)],
    );
}

#[test]
fn a_restart_starts_a_new_running_total() {
    assert_owns(
        owns(vec![
            old_turn("r", 1, 0.5),
            old_turn("r", 2, 1.0),
            old_turn("r", 1, 0.3),
            old_turn("r", 2, 0.4),
        ]),
        &[Some(0.5), Some(0.5), Some(0.3), Some(0.1)],
    );
}

#[test]
fn a_null_cost_row_stays_null_and_keeps_the_running_total() {
    assert_owns(
        owns(vec![
            old_turn("r", 1, 0.5),
            row("turn", "r", Some(2), None),
            old_turn("r", 3, 0.9),
        ]),
        &[Some(0.5), None, Some(0.4)],
    );
}

#[test]
fn a_decreasing_total_keeps_its_negative_own_cost() {
    assert_owns(
        owns(vec![old_turn("r", 1, 1.0), old_turn("r", 2, 0.75)]),
        &[Some(1.0), Some(-0.25)],
    );
}

#[test]
fn the_sum_of_a_process_is_its_last_total() {
    let rows = normalize_costs(vec![
        old_turn("r", 1, 0.5),
        old_ping("r", 0.6),
        old_turn("r", 2, 0.55),
    ]);
    assert!(close(summed(&rows), 0.55));
}

#[test]
fn new_rows_keep_their_own_cost() {
    assert_owns(
        owns(vec![
            new_turn("r", 1, 0.5),
            new_ping("r", 0.1),
            new_turn("r", 2, 0.25),
        ]),
        &[Some(0.5), Some(0.1), Some(0.25)],
    );
}

#[test]
fn a_file_of_old_rows_then_new_rows_sums_both() {
    let rows = normalize_costs(vec![
        old_turn("old", 1, 0.5),
        old_turn("old", 2, 1.0),
        new_turn("new", 1, 0.2),
        new_ping("new", 0.05),
        new_turn("new", 2, 0.3),
    ]);
    assert!(close(summed(&rows), 1.55));
}

#[test]
fn old_then_new_rows_inside_one_process_sum_to_the_total_plus_the_new_costs() {
    let rows = normalize_costs(vec![
        old_turn("r", 1, 0.5),
        old_turn("r", 2, 1.0),
        new_turn("r", 3, 0.25),
        new_ping("r", 0.05),
        old_turn("r", 4, 9.0),
    ]);
    assert!(close(
        rows[0].cost_usd.unwrap() + rows[1].cost_usd.unwrap(),
        1.0
    ));
    assert!(close(rows[4].cost_usd.unwrap(), 9.0 - 1.3));
}

#[test]
fn runs_that_interleave_keep_separate_running_totals() {
    assert_owns(
        owns(vec![
            old_turn("a", 1, 1.0),
            old_turn("b", 1, 5.0),
            old_turn("a", 2, 1.5),
            old_turn("b", 2, 5.25),
        ]),
        &[Some(1.0), Some(5.0), Some(0.5), Some(0.25)],
    );
}

#[test]
fn normalising_twice_changes_nothing() {
    let once = normalize_costs(vec![old_turn("r", 1, 0.5), old_ping("r", 0.6)]);
    assert_eq!(normalize_costs(once.clone()), once);
}

#[test]
fn a_row_without_a_run_forms_its_own_group() {
    let bare = |total| UsageLine {
        run: None,
        ..old_turn("x", 1, total)
    };
    assert_owns(
        owns(vec![bare(1.0), old_turn("r", 1, 2.0)]),
        &[Some(1.0), Some(2.0)],
    );
}

#[test]
fn an_absorbed_tail_normalises_across_chunks() {
    let text = "{\"at\":\"2026-11-01T00:00:00Z\",\"kind\":\"turn\",\"cost_usd\":0.5,\"process_turn\":1,\"run\":\"r\"}\n{\"at\":\"2026-11-01T00:01:00Z\",\"kind\":\"ping\",\"cost_usd\":0.6,\"run\":\"r\"}\n{\"at\":\"2026-11-01T00:02:00Z\",\"kind\":\"turn\",\"cost_usd\":1.0,\"process_turn\":2,\"run\":\"r\"}\n";
    let split = text.find("{\"at\":\"2026-11-01T00:02").unwrap();
    let take = |chunk: &str, tail: UsageTail| {
        let stamp = FileStamp {
            id: None,
            len: (tail.offset + chunk.len() as u64),
        };
        let read = tail_read(&tail, &stamp);
        absorb_usage(tail, &stamp, read, chunk.as_bytes(), "r")
    };
    let empty = UsageTail::starting_at(&FileStamp::default());
    let first = take(&text[..split], empty);
    let both = take(&text[split..], first);
    assert!(close(summed(&both.rows), 1.0));
    assert!(close(both.rows[2].cost_usd.unwrap(), 0.4));
}
