use super::*;
use crate::chat_status::{route_input, EmptySubmit, Route, WorkerState};
use std::collections::BTreeSet;

fn route(line: &str) -> Route {
    route_input(line, 0, &EmptySubmit::default(), &BTreeSet::new())
}

fn direct(id: &str, text: &str) -> Direct {
    Direct {
        id: id.to_string(),
        text: text.to_string(),
    }
}

fn ids(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| id.to_string()).collect()
}

fn status() -> Status {
    Status {
        workers: ["GUZ-2", "GUZ-1", "w1"]
            .iter()
            .map(|id| WorkerState {
                id: id.to_string(),
                state: "running".to_string(),
            })
            .collect(),
        ..Status::default()
    }
}

fn values(line: &str) -> Vec<String> {
    direct_completions(line, line.len(), &status())
        .map(|completion| completion.values)
        .unwrap_or_default()
}

#[test]
fn parses_leading_id_and_text() {
    assert_eq!(
        route("@GUZ-1 run tests"),
        Route::Direct(direct("GUZ-1", "run tests"))
    );
    assert_eq!(
        route("  @GUZ-1   hi  "),
        Route::Direct(direct("GUZ-1", "hi"))
    );
    assert_eq!(route("@GUZ-1"), Route::Direct(direct("GUZ-1", "")));
}

#[test]
fn keeps_multiline_text_verbatim() {
    assert_eq!(
        route("@w1 first\n  second\n\nthird"),
        Route::Direct(direct("w1", "first\n  second\n\nthird"))
    );
    assert_eq!(route("@w1\nline"), Route::Direct(direct("w1", "line")));
}

#[test]
fn mid_line_at_and_email_route_to_send() {
    for line in ["hello @GUZ-1 there", "me@example.com", "email me@x.io now"] {
        assert_eq!(route(line), Route::Send(line.to_string()));
    }
}

#[test]
fn double_at_sends_to_model_minus_one_at() {
    assert_eq!(route("@@GUZ-1 hi"), Route::Send("@GUZ-1 hi".to_string()));
    assert_eq!(route("@@@x"), Route::Send("@@x".to_string()));
}

#[test]
fn bare_at_routes_to_send() {
    assert_eq!(route("@"), Route::Send("@".to_string()));
    assert_eq!(route("@ text"), Route::Send("@ text".to_string()));
}

#[test]
fn empty_text_is_refused() {
    let refusal = resolve_direct(direct("w1", ""), &ids(&["w1"]), |_| false).unwrap_err();
    assert!(refusal.starts_with("@w1: "), "{refusal}");
    assert!(refusal.contains("text"), "{refusal}");
}

#[test]
fn thread_report_id_is_refused() {
    let refusal =
        resolve_direct(direct("thread:x", "hi"), &ids(&["thread:x"]), |_| false).unwrap_err();
    assert!(refusal.starts_with("@thread:x: "), "{refusal}");
    assert!(refusal.contains("digest"), "{refusal}");
}

#[test]
fn unknown_id_lists_known_ids() {
    let refusal = resolve_direct(direct("nope", "hi"), &ids(&["b", "a"]), |_| false).unwrap_err();
    assert!(refusal.starts_with("@nope: "), "{refusal}");
    assert!(refusal.contains("a, b"), "{refusal}");
    assert!(refusal.contains("@@"), "{refusal}");
    let none = resolve_direct(direct("nope", "hi"), &[], |_| false).unwrap_err();
    assert!(none.contains("no workers"), "{none}");
}

#[test]
fn blocked_worker_is_refused() {
    let refusal = resolve_direct(direct("w1", "hi"), &ids(&["w1"]), |id| id == "w1").unwrap_err();
    assert!(refusal.starts_with("@w1: "), "{refusal}");
    assert!(refusal.contains("permission"), "{refusal}");
    assert_eq!(
        resolve_direct(direct("w1", "hi"), &ids(&["w1"]), |_| false),
        Ok(direct("w1", "hi"))
    );
}

#[test]
fn completes_ids_after_at() {
    assert_eq!(values("@"), ["GUZ-1", "GUZ-2", "w1"]);
    assert_eq!(values("@GUZ"), ["GUZ-1", "GUZ-2"]);
    assert!(values("@GUZ-1 ").is_empty());
    assert!(values("@@GUZ").is_empty());
    assert!(values("hi @GUZ").is_empty());
    let completion = direct_completions("@GU", 3, &status()).unwrap();
    assert_eq!(completion.span, 1..3);
}

#[test]
fn direct_route_waits_for_turn_end() {
    assert!(!Route::Direct(direct("w1", "hi")).runs_mid_turn());
}

#[test]
fn memory_line_shows_text_and_result() {
    assert_eq!(
        memory_line(&direct("w1", "a\nb"), "Sent to worker w1; busy."),
        "@w1 a\nb\nResult: Sent to worker w1; busy."
    );
}
