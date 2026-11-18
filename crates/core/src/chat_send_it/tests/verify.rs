use super::*;

fn verify_plan(criterion: serde_json::Value) -> Result<RunPlan, String> {
    let mut json = plan_json();
    json["steps"] = serde_json::json!([{"do": "collect evidence", "criterion": criterion}]);
    parse_run(&json, limits())
}

fn worker_done_with(verify: &str) -> serde_json::Value {
    serde_json::json!({"kind": "worker_done", "worker": "E1", "verify": verify})
}

#[test]
fn worker_done_accepts_a_verify_command() {
    let plan = verify_plan(worker_done_with("  cargo test  ")).unwrap();
    assert_eq!(
        plan.steps[0].criterion,
        Criterion::WorkerDone {
            worker: "E1".to_string(),
            verify: Some("cargo test".to_string()),
        }
    );
}

#[test]
fn a_blank_verify_is_refused() {
    for blank in ["", "   "] {
        let error = verify_plan(worker_done_with(blank)).unwrap_err();
        assert!(
            error.contains("Step 1 has an empty verify command"),
            "{error}"
        );
    }
}

#[test]
fn verify_on_another_kind_is_refused() {
    for criterion in [
        serde_json::json!({"kind": "check_passes", "command": "true", "verify": "true"}),
        serde_json::json!({"kind": "watch_passes", "command": "true", "verify": "true"}),
        serde_json::json!({"kind": "judge", "question": "ok?", "verify": "true"}),
    ] {
        let error = verify_plan(criterion).unwrap_err();
        assert!(error.contains("verify"), "{error}");
    }
}

#[test]
fn a_stray_verify_names_the_step() {
    let mut json = plan_json();
    json["steps"] = serde_json::json!([
        {"do": "first", "criterion": {"kind": "judge", "question": "ok?"}},
        {"do": "second", "criterion": {"kind": "check_passes", "command": "true", "verify": "true"}}
    ]);
    let error = parse_run(&json, limits()).unwrap_err();
    assert_eq!(error, "Step 2: verify is only for worker_done");
}

#[test]
fn unrelated_keys_and_a_null_verify_are_accepted() {
    let extra = serde_json::json!({"kind": "worker_done", "worker": "E1", "command": ""});
    assert!(verify_plan(extra).is_ok());
    let null = serde_json::json!({"kind": "check_passes", "command": "true", "verify": null});
    assert!(verify_plan(null).is_ok());
}

#[test]
fn an_old_run_toml_without_verify_loads_and_keeps_its_hash() {
    let file = RunFile {
        plan: plan(),
        approved: None,
        stopped: None,
    };
    let text = write_run(&file).unwrap();
    assert!(!text.contains("verify"), "{text}");
    let loaded = read_run(&text, limits()).unwrap();
    assert_eq!(loaded.plan, plan());
    assert_eq!(plan_hash(&loaded.plan), plan_hash(&plan()));
}

#[test]
fn a_verify_command_survives_a_run_toml_roundtrip() {
    let plan = verify_plan(worker_done_with("cargo test")).unwrap();
    let file = RunFile {
        plan: plan.clone(),
        approved: None,
        stopped: None,
    };
    let loaded = read_run(&write_run(&file).unwrap(), limits()).unwrap();
    assert_eq!(loaded.plan, plan);
}

#[test]
fn the_propose_run_tool_schema_lists_verify() {
    let tool = propose_run_tool();
    let properties = &tool["inputSchema"]["properties"]["steps"]["items"]["properties"]
        ["criterion"]["properties"];
    assert_eq!(properties["verify"]["type"], "string");
}

#[test]
fn the_plan_view_shows_the_verify_command() {
    let plan = verify_plan(worker_done_with("cargo test")).unwrap();
    let text = render_run(&plan);
    assert!(
        text.contains("exit: worker E1 reports done, then `cargo test` exits 0"),
        "{text}"
    );
}
