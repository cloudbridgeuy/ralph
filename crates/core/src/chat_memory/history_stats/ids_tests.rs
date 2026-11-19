use super::ids::{call_span, normalize_scope, overlaps, parse_span, view_low, Span};
use super::test_rows::{call, history};
use serde_json::json;

fn span(scope: Option<&str>, lo: u64, hi: u64) -> Span {
    Span {
        scope: scope.map(str::to_string),
        lo,
        hi,
    }
}

#[test]
fn spans_parse_single_ranges_and_scopes() {
    assert_eq!(parse_span("5", None), Some(span(None, 5, 5)));
    assert_eq!(parse_span("4-7", None), Some(span(None, 4, 7)));
    assert_eq!(parse_span("auth:4-7", None), Some(span(Some("auth"), 4, 7)));
    assert_eq!(parse_span("main:3", None), Some(span(None, 3, 3)));
    assert_eq!(
        parse_span("3", Some("auth")),
        Some(span(Some("auth"), 3, 3))
    );
    assert_eq!(parse_span("main:3", Some("auth")), Some(span(None, 3, 3)));
    assert_eq!(parse_span("x", None), None);
    assert_eq!(parse_span("7-4", None), None);
    assert_eq!(parse_span("", None), None);
}

#[test]
fn scopes_normalize_main_and_own_to_bare() {
    assert_eq!(normalize_scope("main"), None);
    assert_eq!(normalize_scope("own"), None);
    assert_eq!(normalize_scope(""), None);
    assert_eq!(normalize_scope("auth"), Some("auth".to_string()));
}

#[test]
fn view_low_ignores_other_scopes() {
    assert_eq!(view_low("4-7", None), Some(4));
    assert_eq!(view_low("auth:2", Some("auth")), Some(2));
    assert_eq!(view_low("auth:2", None), None);
    assert_eq!(view_low("9", Some("auth")), Some(9));
    assert_eq!(view_low("junk", None), None);
}

#[test]
fn spans_overlap_only_in_the_same_scope() {
    assert!(overlaps(&span(None, 4, 7), &span(None, 7, 9)));
    assert!(!overlaps(&span(None, 4, 7), &span(None, 8, 9)));
    assert!(!overlaps(&span(None, 4, 7), &span(Some("a"), 4, 7)));
}

#[test]
fn call_spans_come_from_zoom_and_read_args() {
    let parsed = history(&[
        call(json!({"tool": "zoom", "args": {"id": "4-7"}})),
        call(json!({"tool": "read", "args": {"from": 2, "to": 6, "mode": "raw"}})),
        call(json!({"tool": "read", "args": {"id": "3-4"}})),
        call(json!({"tool": "zoom", "scope": "auth", "args": {"id": "5"}})),
        call(json!({"tool": "search", "args": {"query": "x"}})),
    ]);
    let spans: Vec<_> = parsed.calls.iter().map(call_span).collect();
    assert_eq!(spans[0], Some(span(None, 4, 7)));
    assert_eq!(spans[1], Some(span(None, 2, 6)));
    assert_eq!(spans[2], Some(span(None, 3, 4)));
    assert_eq!(spans[3], Some(span(Some("auth"), 5, 5)));
    assert_eq!(spans[4], None);
}
