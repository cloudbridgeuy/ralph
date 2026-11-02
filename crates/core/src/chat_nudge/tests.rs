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
fn no_id_prints_the_usage() {
    assert_eq!(parse_nudge_args(""), NudgeCommand::Usage);
    assert_eq!(parse_nudge_args("   "), NudgeCommand::Usage);
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
    assert_eq!(route("/nudge"), Route::Nudge(NudgeCommand::Usage));
    assert_eq!(route("/nudge GUZ-1 hi"), Route::Nudge(send("GUZ-1", "hi")));
    assert_eq!(route("/nudgex"), Route::Send("/nudgex".to_string()));
}

#[test]
fn only_the_usage_runs_mid_turn() {
    assert!(Route::Nudge(NudgeCommand::Usage).runs_mid_turn());
    assert!(!Route::Nudge(send("GUZ-1", "hi")).runs_mid_turn());
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
