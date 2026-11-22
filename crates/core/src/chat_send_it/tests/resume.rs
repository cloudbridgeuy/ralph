use super::*;

fn stopped_file() -> RunFile {
    let hash = plan_hash(&plan());
    stop_file(&approved(plan()), &hash, Some(2)).unwrap()
}

#[test]
fn go_resumes_a_stopped_plan_without_it_being_shown() {
    let resumed = signed_go(&stopped_file(), None, true).map(|file| file.approved);
    assert_eq!(resumed, Ok(Some(signed(&plan()))));
}

#[test]
fn go_on_a_stopped_plan_still_needs_the_prompt() {
    let refused = signed_go(&stopped_file(), None, false);
    assert_eq!(
        refused,
        Err("/send-it go counts only when you type it at the prompt.".to_string())
    );
}

#[test]
fn go_on_a_changed_stopped_plan_needs_it_shown() {
    let mut changed = plan();
    changed.goal = "ship GUZ-2".to_string();
    let file = RunFile {
        plan: changed,
        ..stopped_file()
    };
    let hash = plan_hash(&file.plan);
    assert_eq!(
        signed_go(&file, None, true).map(|file| file.approved),
        Err(format!(
            "Plan {hash} is not the plan on screen. Show it with /send-it status, then type /send-it go."
        ))
    );
}

#[test]
fn go_on_a_finished_run_needs_it_shown() {
    let hash = plan_hash(&plan());
    let finished = stop_file(&approved(plan()), &hash, None).unwrap();
    assert!(signed_go(&finished, None, true).is_err());
}
