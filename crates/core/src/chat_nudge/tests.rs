use super::*;
use crate::chat_status::{route_input, EmptySubmit, Route, ScheduleState, WatchState, WorkerState};
use std::collections::BTreeSet;

fn send(id: &str, text: &str) -> NudgeCommand {
    NudgeCommand::Send {
        id: id.to_string(),
        text: text.to_string(),
    }
}

fn route(line: &str) -> Route {
    route_input(line, 0, &EmptySubmit::default(), &BTreeSet::new())
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
        watches: vec![WatchState {
            id: "watch:ci".to_string(),
            left: None,
        }],
        schedule: vec![ScheduleState {
            id: "nightly".to_string(),
            time: "t".to_string(),
        }],
        ..Status::default()
    }
}

fn values(line: &str) -> Vec<String> {
    nudge_completions(line, line.len(), &status())
        .map(|completion| completion.values)
        .unwrap_or_default()
}

#[test]
fn no_id_parses_as_bare() {
    assert_eq!(parse_nudge_args(""), NudgeCommand::Bare);
    assert_eq!(parse_nudge_args("   "), NudgeCommand::Bare);
}

#[test]
fn an_id_alone_sends_the_progress_request() {
    assert_eq!(parse_nudge_args(" GUZ-1 "), send("GUZ-1", PROGRESS_REQUEST));
}

#[test]
fn text_after_the_id_replaces_the_progress_request() {
    assert_eq!(
        parse_nudge_args(" GUZ-1   run  git status first "),
        send("GUZ-1", "run  git status first")
    );
}

#[test]
fn nudge_routes_locally() {
    assert_eq!(route("/nudge"), Route::Nudge(NudgeCommand::Bare));
    assert_eq!(route("/nudge GUZ-1 hi"), Route::Nudge(send("GUZ-1", "hi")));
    assert_eq!(route("/nudgex"), Route::Send("/nudgex".to_string()));
}

#[test]
fn no_nudge_runs_mid_turn() {
    assert!(!Route::Nudge(NudgeCommand::Bare).runs_mid_turn());
    assert!(!Route::Nudge(send("GUZ-1", "hi")).runs_mid_turn());
}

fn ids(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| id.to_string()).collect()
}

fn resolve(command: NudgeCommand, running: &[&str]) -> Result<(String, String), String> {
    resolve_nudge(command, &ids(running), |_| false)
}

#[test]
fn bare_with_no_workers_says_none_run() {
    assert_eq!(
        resolve(NudgeCommand::Bare, &[]),
        Err("/nudge: no workers running".to_string())
    );
}

#[test]
fn bare_with_one_worker_sends_it_the_progress_request() {
    assert_eq!(
        resolve(NudgeCommand::Bare, &["GUZ-1"]),
        Ok(("GUZ-1".to_string(), PROGRESS_REQUEST.to_string()))
    );
}

#[test]
fn bare_with_several_workers_prints_the_usage_and_the_sorted_ids() {
    assert_eq!(
        resolve(NudgeCommand::Bare, &["GUZ-2", "GUZ-1", "w1"]),
        Err("Usage: /nudge <id> [text]; running: GUZ-1, GUZ-2, w1".to_string())
    );
}

#[test]
fn a_given_id_is_kept_and_never_auto_selected() {
    assert_eq!(
        resolve(send("GUZ-9", "hi"), &["GUZ-1"]),
        Ok(("GUZ-9".to_string(), "hi".to_string()))
    );
    assert_eq!(
        resolve(parse_nudge_args("hi there"), &["GUZ-1"]),
        Ok(("hi".to_string(), "there".to_string()))
    );
    assert_eq!(
        resolve(send("GUZ-9", PROGRESS_REQUEST), &[]),
        Ok(("GUZ-9".to_string(), PROGRESS_REQUEST.to_string()))
    );
}

#[test]
fn a_blocked_worker_is_refused_when_auto_selected_or_named() {
    let blocked = |id: &str| id == "GUZ-1";
    let refusal = Err(blocked_refusal("GUZ-1"));
    assert_eq!(
        resolve_nudge(NudgeCommand::Bare, &ids(&["GUZ-1"]), blocked),
        refusal
    );
    assert_eq!(
        resolve_nudge(send("GUZ-1", "hi"), &ids(&["GUZ-1", "GUZ-2"]), blocked),
        refusal
    );
    assert_eq!(
        resolve_nudge(send("GUZ-2", "hi"), &ids(&["GUZ-1", "GUZ-2"]), blocked),
        Ok(("GUZ-2".to_string(), "hi".to_string()))
    );
}

#[test]
fn completion_offers_only_worker_ids_sorted() {
    assert_eq!(
        nudge_completions("/nudge ", 7, &status()),
        Some(HoldCompletion {
            span: 7..7,
            values: vec!["GUZ-1".to_string(), "GUZ-2".to_string(), "w1".to_string()],
        })
    );
}

#[test]
fn completion_filters_by_the_partial_id() {
    let completion = nudge_completions("/nudge GU", 9, &status()).unwrap();
    assert_eq!(completion.span, 7..9);
    assert_eq!(completion.values, ["GUZ-1", "GUZ-2"]);
}

#[test]
fn completion_stops_after_the_id() {
    assert_eq!(values("/nudge GUZ-1 w"), Vec::<String>::new());
    assert_eq!(nudge_completions("/nudge GUZ-1 w", 14, &status()), None);
    assert_eq!(nudge_completions("/nudgew", 7, &status()), None);
    assert_eq!(nudge_completions("/hold w", 7, &status()), None);
}

#[test]
fn refusal_and_failure_lines_name_the_command() {
    assert_eq!(
        blocked_refusal("GUZ-1"),
        "/nudge: worker GUZ-1 waits on a permission request; answer it by request id."
    );
    assert_eq!(
        failure_line("worker GUZ-1 is not running"),
        "/nudge: worker GUZ-1 is not running"
    );
}

#[test]
fn a_merged_thread_report_cannot_be_nudged() {
    assert_eq!(
        resolve(send("thread:auth", "hi"), &[]),
        Err("thread:auth is a merged thread's digest, not a worker; there is nothing to reply to, nudge or close.".to_string())
    );
}
