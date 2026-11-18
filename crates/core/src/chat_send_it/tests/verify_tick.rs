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
        ..driving(0, Awaiting::Report(WATCH_ID.to_string()), Run::default())
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
    let met = ticked.met.unwrap();
    assert!(
        met.starts_with("send-it: step 1 would block: verify failed: exited with code 2"),
        "{met}"
    );
    assert!(met.contains("step 1/4 met"), "{met}");
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
    assert!(!ticked.met.unwrap().contains("would block"));
}
