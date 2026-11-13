use super::*;

const WARNING: &str = "Warning: step 1 waits for worker E1, but the plan does not allow `reply`. A question from that worker stops the run.";

fn worker_plan(allowed: serde_json::Value) -> RunPlan {
    let mut json = plan_json();
    json["allowed"] = allowed;
    parse_run(&json, limits()).unwrap()
}

#[test]
fn worker_done_without_reply_warns() {
    let plan = worker_plan(serde_json::json!([{"kind": "merge"}]));
    assert_eq!(plan_warnings(&plan), vec![WARNING.to_string()]);
}

#[test]
fn worker_done_with_reply_has_no_warning() {
    let plan = worker_plan(serde_json::json!([{"kind": "reply"}]));
    assert!(plan_warnings(&plan).is_empty());
}

#[test]
fn a_plan_without_worker_done_has_no_warning() {
    let mut json = plan_json();
    json["steps"] = serde_json::json!([
        {"do": "run the checks", "criterion": {"kind": "check_passes", "command": "cargo xtask lint"}}
    ]);
    let plan = parse_run(&json, limits()).unwrap();
    assert!(plan_warnings(&plan).is_empty());
}

#[test]
fn each_worker_done_step_without_reply_gets_its_own_warning() {
    let mut json = plan_json();
    json["steps"] = serde_json::json!([
        {"do": "first", "criterion": {"kind": "worker_done", "worker": "A"}},
        {"do": "second", "criterion": {"kind": "worker_done", "worker": "B"}}
    ]);
    json["allowed"] = serde_json::json!([]);
    let plan = parse_run(&json, limits()).unwrap();
    let warnings = plan_warnings(&plan);
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings[1].starts_with("Warning: step 2 waits for worker B"));
}

#[test]
fn render_run_shows_the_warning_after_the_allowed_line() {
    let text = render_run(&worker_plan(serde_json::json!([{"kind": "merge"}])));
    let lines: Vec<&str> = text.lines().collect();
    let allowed = lines.iter().position(|line| line.starts_with("Allowed:"));
    let warning = lines.iter().position(|line| *line == WARNING);
    assert_eq!(warning, allowed.map(|index| index + 1), "{text}");
}

#[test]
fn render_run_with_reply_has_no_warning_line() {
    let text = render_run(&worker_plan(serde_json::json!([{"kind": "reply"}])));
    assert!(!text.contains("Warning:"), "{text}");
}
