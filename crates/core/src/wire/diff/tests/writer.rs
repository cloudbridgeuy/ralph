use super::*;

#[test]
fn writer_breakpoint_counts_servable_tokens_past_a_moved_breakpoint() {
    let a = call(
        1,
        0,
        10,
        vec![blk("h", Some("5m")), blk("q1", Some("5m"))],
        written(20, 80),
    );
    let b = call(
        2,
        20,
        30,
        vec![
            blk("h", Some("5m")),
            blk("q1", None),
            blk("q2", None),
            blk("q3", Some("5m")),
        ],
        usage(0, 100),
    );
    let explanation = view_of(&a, &b, &[a.clone(), b.clone()]).explanation;
    assert_eq!(explanation.servable_tokens, Some(100));
    assert_eq!(explanation.method, Some(TokenMethod::Anchor { seq: 1 }));
}

#[test]
fn writer_tier_decides_ttl_expiry() {
    let a = call(
        1,
        0,
        10,
        vec![blk("h", Some("5m")), blk("tail", Some("1h"))],
        written(0, 100),
    );
    let b = call(
        2,
        410,
        420,
        vec![blk("h", Some("5m")), blk("tail", Some("5m"))],
        written(0, 100),
    );
    assert!(matches!(verdict_of(&a, &b), Verdict::Unexplained { .. }));
}

fn run_of(len: usize, marked: usize) -> Vec<Value> {
    (0..len)
        .map(|i| blk(&format!("b{i}"), (i == marked).then_some("5m")))
        .collect()
}

#[test]
fn writer_lookback_out_of_range_has_no_shared_breakpoint() {
    let a = call(1, 0, 10, run_of(21, 0), written(0, 60));
    let b = call(2, 20, 30, run_of(21, 20), usage(0, 60));
    assert_eq!(verdict_of(&a, &b), Verdict::NoSharedBreakpoint);
}

#[test]
fn writer_lookback_in_range_uses_the_writer_breakpoint() {
    let a = call(1, 0, 10, run_of(21, 0), written(0, 60));
    let b = call(2, 20, 30, run_of(21, 19), usage(0, 60));
    let explanation = view_of(&a, &b, &[a.clone(), b.clone()]).explanation;
    assert_eq!(explanation.servable_tokens, Some(60));
    assert_eq!(explanation.method, Some(TokenMethod::Anchor { seq: 1 }));
}

#[test]
fn writer_lookback_reaches_past_the_divergence() {
    let a = call(1, 0, 10, run_of(5, 1), written(0, 50));
    let mut b_blocks = run_of(5, 4);
    b_blocks[2] = blk("changed", None);
    let b = call(2, 20, 30, b_blocks, usage(0, 50));
    let explanation = view_of(&a, &b, &[a.clone(), b.clone()]).explanation;
    assert_eq!(explanation.servable_tokens, Some(50));
    assert_eq!(explanation.method, Some(TokenMethod::Anchor { seq: 1 }));
}
