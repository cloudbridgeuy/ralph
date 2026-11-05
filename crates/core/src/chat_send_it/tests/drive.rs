use super::*;

fn replying(allowed: bool) -> RunFile {
    let mut plan = plan();
    if allowed {
        plan.allowed.push(Action::Reply);
    }
    approved(plan)
}

fn state(file: &RunFile, step: usize, awaiting: Awaiting) -> Driving {
    Driving {
        hash: plan_hash(&file.plan),
        ..driving(step, awaiting, Run::default())
    }
}

fn held(file: &RunFile) -> Driving {
    state(file, 0, Awaiting::Report("E1".to_string()))
}

fn answered_ids(ids: &[&str]) -> Vec<String> {
    ids.iter().map(ToString::to_string).collect()
}

const FLAGGED: Next = Next::Stop(StopReason::WorkerFlagged);

#[test]
fn an_answered_question_the_plan_allows_keeps_the_held_run_holding() {
    let file = replying(true);
    let drained = [worker_report("E1", "question")];
    let answered = answered_ids(&["E1"]);
    let ticked = tick(
        &held(&file),
        &Ended {
            drained: &drained,
            answered: &answered,
            ..ended(&file)
        },
    );
    assert_eq!(ticked.next, Next::Hold("E1".to_string()));
}

#[test]
fn an_unanswered_question_stops_the_run() {
    let file = replying(true);
    let drained = [worker_report("E1", "question")];
    let ticked = tick(
        &held(&file),
        &Ended {
            drained: &drained,
            ..ended(&file)
        },
    );
    assert_eq!(ticked.next, FLAGGED);
}

#[test]
fn an_answer_the_plan_does_not_allow_still_stops_the_run() {
    let file = replying(false);
    let drained = [worker_report("E1", "question")];
    let answered = answered_ids(&["E1"]);
    let ticked = tick(
        &held(&file),
        &Ended {
            drained: &drained,
            answered: &answered,
            ..ended(&file)
        },
    );
    assert_eq!(ticked.next, FLAGGED);
}

#[test]
fn one_unanswered_flagged_report_among_answered_ones_stops_the_run() {
    let file = replying(true);
    let drained = [
        worker_report("E1", "question"),
        worker_report("X9", "blocked"),
    ];
    let answered = answered_ids(&["E1"]);
    let ticked = tick(
        &held(&file),
        &Ended {
            drained: &drained,
            answered: &answered,
            ..ended(&file)
        },
    );
    assert_eq!(ticked.next, FLAGGED);
}

#[test]
fn an_unread_question_stops_the_run_even_when_that_worker_was_answered() {
    let file = replying(true);
    let unread = [worker_report("E1", "question")];
    let answered = answered_ids(&["E1"]);
    let ticked = tick(
        &held(&file),
        &Ended {
            unread: &unread,
            answered: &answered,
            ..ended(&file)
        },
    );
    assert_eq!(ticked.next, FLAGGED);
}

#[test]
fn an_answered_question_does_not_stop_a_judge_step() {
    let file = replying(true);
    let drained = [worker_report("E1", "question")];
    let answered = answered_ids(&["E1"]);
    let result = StepResult {
        met: true,
        evidence: "all fixed".to_string(),
    };
    let ticked = tick(
        &state(&file, 3, Awaiting::Verdict),
        &Ended {
            drained: &drained,
            answered: &answered,
            step_result: Some(&result),
            ..ended(&file)
        },
    );
    assert_eq!(ticked.next, Next::Done);
}
