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
