use super::render::strip;
use super::*;

const WIDE: usize = 1000;

fn no_judge_plan() -> RunPlan {
    let mut json = plan_json();
    json["steps"] = serde_json::json!([
        {"do": "run the checks", "criterion": {"kind": "check_passes", "command": "cargo xtask lint"}}
    ]);
    parse_run(&json, limits()).unwrap()
}

fn caps_line(plan: &RunPlan) -> String {
    render_run(plan)
        .lines()
        .find(|line| line.starts_with("Caps:"))
        .unwrap()
        .to_string()
}

#[test]
fn a_plan_without_a_judge_step_omits_the_judge_rounds() {
    assert_eq!(caps_line(&no_judge_plan()), "Caps: 3 turns, $4.00");
}

#[test]
fn a_plan_with_a_judge_step_shows_the_judge_rounds() {
    assert_eq!(caps_line(&plan()), "Caps: 3 turns, $4.00, 2 judge rounds");
}

#[test]
fn a_hidden_judge_rounds_cap_still_changes_the_plan_hash() {
    let mut other = no_judge_plan();
    other.judge_rounds = 5;
    assert_ne!(plan_hash(&other), plan_hash(&no_judge_plan()));
    assert_eq!(caps_line(&other), caps_line(&no_judge_plan()));
}

#[test]
fn a_plan_without_grants_omits_the_pre_approve_block() {
    let mut plan = plan();
    plan.grants.clear();
    let text = render_run(&plan);
    assert!(!text.contains("pre-approve"), "{text}");
    assert!(!text.contains("nothing"), "{text}");
    assert!(text.ends_with("2 judge rounds"), "{text}");
}

#[test]
fn a_plan_with_grants_keeps_the_pre_approve_block() {
    let text = render_run(&plan());
    assert!(
        text.ends_with("You pre-approve:\n  - raise the turn cap to 30"),
        "{text}"
    );
}

#[test]
fn the_styled_status_dims_the_plan_hash() {
    let file = RunFile {
        plan: plan(),
        approved: None,
        stopped: None,
    };
    let hash = plan_hash(&file.plan);
    let styled = render_status_styled(&file, WIDE, TRUST);
    let last = styled.lines().last().unwrap();
    assert!(last.contains(&format!("\x1b[2m{hash}\x1b[0m")), "{last:?}");
    assert_eq!(strip(&styled), render_status(&file, WIDE, TRUST));
    assert!(!render_status(&file, WIDE, TRUST).contains('\x1b'));
}
