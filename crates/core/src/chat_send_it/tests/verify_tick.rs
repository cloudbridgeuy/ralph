use super::*;

const WATCH_ID: &str = "watch:send-it-1";

fn verifying_file() -> RunFile {
    let mut json = plan_json();
    json["steps"][0]["criterion"]["verify"] = serde_json::json!("cargo test");
    approved(parse_run(&json, limits()).unwrap())
}

fn observing(file: &RunFile) -> Driving {
    Driving {
        hash: plan_hash(&file.plan),
        progress: Progress {
            verify: VerifyMode::Observe,
            verifying: true,
            ..at(0)
        },
        ..driving(0, awaiting_report(WATCH_ID), Run::default())
    }
}

#[test]
fn a_verify_failure_reaching_tick_gives_the_would_block_notice() {
    let file = verifying_file();
    let output = Report {
        text: "Watch done.\n\nOutput:\nassertion failed".to_string(),
        ..worker_report(WATCH_ID, "done")
    };
    let ticked = tick(
        &observing(&file),
        &Ended {
            watched: Some(&WatchOutcome::Exited(2)),
            drained: &[output],
            ..ended(&file)
        },
    );
    assert_eq!(ticked.driving.progress.step, 1);
    let notice = ticked.notice.unwrap();
    assert!(
        notice.starts_with("send-it: step 1 would block: verify failed: exited with code 2"),
        "{notice}"
    );
    assert!(ticked.met.unwrap().contains("step 1/4 met"));
}

#[test]
fn a_passing_verify_reaching_tick_gives_no_notice() {
    let file = verifying_file();
    let ticked = tick(
        &observing(&file),
        &Ended {
            watched: Some(&WatchOutcome::Exited(0)),
            ..ended(&file)
        },
    );
    assert_eq!(ticked.notice, None);
}

#[test]
fn the_notice_shows_on_the_last_step_where_the_met_line_is_hidden() {
    let mut json = plan_json();
    json["steps"] = serde_json::json!([
        {"do": "collect evidence", "criterion": {"kind": "worker_done", "worker": "E1", "verify": "cargo test"}}
    ]);
    let file = approved(parse_run(&json, limits()).unwrap());
    let ticked = tick(
        &observing(&file),
        &Ended {
            watched: Some(&WatchOutcome::Exited(2)),
            ..ended(&file)
        },
    );
    assert_eq!(ticked.next, Next::Done);
    let shown: Vec<&str> = ticked.shown_lines().collect();
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert!(shown[0].contains("step 1 would block"), "{shown:?}");
}

#[test]
fn shown_lines_lists_the_notice_before_the_met_line_mid_run() {
    let file = verifying_file();
    let ticked = tick(
        &observing(&file),
        &Ended {
            watched: Some(&WatchOutcome::Exited(2)),
            ..ended(&file)
        },
    );
    let shown: Vec<&str> = ticked.shown_lines().collect();
    assert_eq!(shown.len(), 2, "{shown:?}");
    assert!(shown[0].contains("would block"));
    assert!(shown[1].contains("met"));
}
