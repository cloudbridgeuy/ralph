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
fn render_run_shows_the_warning_after_the_not_allowed_line() {
    let text = render_run(&worker_plan(serde_json::json!([{"kind": "merge"}])));
    let lines: Vec<&str> = text.lines().collect();
    let denied = lines
        .iter()
        .position(|line| line.starts_with("Not allowed:"));
    let warning = lines.iter().position(|line| *line == WARNING);
    assert_eq!(warning, denied.map(|index| index + 1), "{text}");
}

#[test]
fn render_run_with_reply_has_no_warning_line() {
    let text = render_run(&worker_plan(serde_json::json!([{"kind": "reply"}])));
    assert!(!text.contains("Warning:"), "{text}");
}

const JUDGE_WARNING: &str = "Warning: every step uses a judge. A judge costs one model turn per round. Prefer check_passes or watch_passes: ralph runs them for $0.";

fn judge_only_plan(count: usize) -> RunPlan {
    let steps: Vec<serde_json::Value> = (0..count)
        .map(|index| {
            serde_json::json!({"do": format!("step {index}"), "criterion": {"kind": "judge", "question": "is it done?"}})
        })
        .collect();
    let mut json = plan_json();
    json["steps"] = serde_json::Value::Array(steps);
    json["allowed"] = serde_json::json!([]);
    parse_run(&json, limits()).unwrap()
}

#[test]
fn a_judge_only_plan_warns_once() {
    assert_eq!(
        plan_warnings(&judge_only_plan(1)),
        vec![JUDGE_WARNING.to_string()]
    );
    assert_eq!(
        plan_warnings(&judge_only_plan(3)),
        vec![JUDGE_WARNING.to_string()]
    );
}

#[test]
fn a_mixed_plan_has_no_judge_warning() {
    let plan = worker_plan(serde_json::json!([{"kind": "reply"}]));
    assert!(!plan_warnings(&plan)
        .iter()
        .any(|line| line.contains("judge")));
}
