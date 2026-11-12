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

fn stopped_on(step: usize) -> RunFile {
    RunFile {
        stopped: stopped_at(step),
        ..approved(plan())
    }
}

#[test]
fn a_resumed_worker_step_whose_worker_reported_done_moves_to_the_next_step() {
    let started = start(&stopped_on(1), &[("E1", "done")], &[]).unwrap();
    assert_eq!(
        started.driving,
        driving(
            1,
            Awaiting::Report("watch:send-it-2".to_string()),
            Run::default()
        )
    );
    assert_eq!(
        started.next,
        Next::Watch("gh pr checks --watch".to_string())
    );
    assert_eq!(
        started.met.as_deref(),
        Some("send-it: step 1/4 met (worker E1 reports done)")
    );
}

#[test]
fn a_resumed_worker_step_with_a_known_or_running_worker_holds_and_does_not_resend() {
    let question: &[(&str, &str)] = &[("E1", "question")];
    let idle: &[(&str, &str)] = &[("E1", "idle")];
    for (known, running) in [(question, &[][..]), (idle, &[][..]), (&[][..], &["E1"][..])] {
        let started = start(&stopped_on(1), known, running).unwrap();
        assert_eq!(started.next, Next::Hold("E1".to_string()));
        assert_eq!(
            started.driving,
            driving(0, Awaiting::Report("E1".to_string()), Run::default())
        );
        assert_eq!(started.met, None);
    }
}

#[test]
fn a_resumed_worker_step_with_an_unknown_worker_sends_the_step() {
    let started = start(&stopped_on(1), &[("X9", "done")], &["X9"]).unwrap();
    let Next::Send(brief) = &started.next else {
        panic!("{:?}", started.next)
    };
    assert!(brief.ends_with(STEP_1), "{brief}");
    assert_eq!(started.driving.awaiting, Awaiting::Turn);
}

#[test]
fn a_resumed_check_step_runs_the_check_first_and_a_failure_stops_the_run() {
    let file = stopped_on(3);
    let started = start(&file, &[], &[]).unwrap();
    assert_eq!(started.next, Next::Check("cargo xtask lint".to_string()));
    assert_eq!(
        started.driving.awaiting,
        Awaiting::Report("watch:send-it-3".to_string())
    );
    let failed = WatchOutcome::Exited(1);
    let stopped = tick(
        &started.driving,
        &Ended {
            watched: Some(&failed),
            ..ended(&file)
        },
    );
    assert_eq!(stopped.next, Next::Stop(StopReason::check_failed(3)));
}

#[test]
fn a_resumed_check_step_that_passes_is_met() {
    let file = stopped_on(3);
    let started = start(&file, &[], &[]).unwrap();
    let passed = WatchOutcome::Exited(0);
    let ticked = tick(
        &started.driving,
        &Ended {
            watched: Some(&passed),
            ..ended(&file)
        },
    );
    assert_eq!(ticked.next, Next::Send(STEP_4.to_string()));
    assert_eq!(ticked.driving.progress, at(3));
}

#[test]
fn a_check_or_watch_step_starts_by_running_the_command_with_no_step_turn() {
    let plan = plan();
    assert_eq!(
        advance(&plan, at(1), Event::Start),
        (at(1), Next::Watch("gh pr checks --watch".to_string()))
    );
    assert_eq!(
        advance(&plan, at(2), Event::Start),
        (at(2), Next::Check("cargo xtask lint".to_string()))
    );
}

#[test]
fn a_run_whose_first_step_is_a_check_starts_by_running_it() {
    let mut plan = plan();
    plan.steps.remove(0);
    let started = start(&approved(plan), &[], &[]).unwrap();
    assert_eq!(
        started.next,
        Next::Watch("gh pr checks --watch".to_string())
    );
    assert_eq!(
        started.driving.awaiting,
        Awaiting::Report("watch:send-it-1".to_string())
    );
    assert_eq!(started.met, None);
}

#[test]
fn a_resumed_judge_step_judges_first_and_a_miss_is_not_a_judge_round() {
    let file = stopped_on(4);
    let started = start(&file, &[], &[]).unwrap();
    assert_eq!(started.next, Next::Judge);
    assert_eq!(started.driving.awaiting, Awaiting::Verdict);
    let unmet = StepResult {
        met: false,
        evidence: "finding 2 is open".to_string(),
    };
    let probed = tick(
        &started.driving,
        &Ended {
            step_result: Some(&unmet),
            ..ended(&file)
        },
    );
    assert_eq!(
        probed.next,
        Next::Send(format!("{STEP_4}\njudge: not met: finding 2 is open"))
    );
    assert_eq!(probed.driving.progress, at(3));
}

#[test]
fn only_the_generated_check_shape_is_a_check_watch_id() {
    assert!(is_check_watch_id("watch:send-it-3"));
    assert!(is_check_watch_id("watch:send-it-12"));
    assert!(!is_check_watch_id("watch:send-it-deploy"));
    assert!(!is_check_watch_id("watch:send-it-"));
    assert!(!is_check_watch_id("watch:send-it-3x"));
}

fn checking(file: &RunFile, step: usize, run: Run) -> Driving {
    Driving {
        run,
        ..state(file, step, Awaiting::Report(check_watch_id(step)))
    }
}

#[test]
fn a_passing_check_settles_into_the_next_step_with_a_met_line_and_no_turn() {
    let file = approved(plan());
    let run = spent(1, 0.25);
    let settled = settle_check(
        &checking(&file, 2, run.clone()),
        Some(&file),
        &WatchOutcome::Exited(0),
        &[],
    );
    assert_eq!(settled.next, Next::Send(STEP_4.to_string()));
    assert_eq!(settled.driving.progress, at(3));
    assert_eq!(settled.driving.awaiting, Awaiting::Turn);
    assert_eq!(settled.driving.run, run);
    assert_eq!(
        settled.met.as_deref(),
        Some("send-it: step 3/4 met (`cargo xtask lint` exits 0)")
    );
}

#[test]
fn a_passing_last_check_finishes_the_run() {
    let mut plan = plan();
    plan.steps.truncate(3);
    let file = approved(plan);
    let settled = settle_check(
        &checking(&file, 2, Run::default()),
        Some(&file),
        &WatchOutcome::Exited(0),
        &[],
    );
    assert_eq!(settled.next, Next::Done);
    assert_eq!(
        settled.met.as_deref(),
        Some("send-it: step 3/3 met (`cargo xtask lint` exits 0)")
    );
}

#[test]
fn a_passing_watch_chains_into_a_following_check_without_a_turn() {
    let file = approved(plan());
    let settled = settle_check(
        &checking(&file, 1, Run::default()),
        Some(&file),
        &WatchOutcome::Exited(0),
        &[],
    );
    assert_eq!(settled.next, Next::Check("cargo xtask lint".to_string()));
    assert_eq!(
        settled.driving.awaiting,
        Awaiting::Report("watch:send-it-3".to_string())
    );
    assert_eq!(settled.driving.run, Run::default());
}

#[test]
fn any_other_outcome_stops_the_run_with_check_failed_and_no_turn() {
    let file = approved(plan());
    let run = spent(1, 0.25);
    for outcome in [
        WatchOutcome::Exited(1),
        WatchOutcome::Signaled(9),
        WatchOutcome::Stopped,
        WatchOutcome::WaitFailed("gone".to_string()),
    ] {
        let settled = settle_check(&checking(&file, 2, run.clone()), Some(&file), &outcome, &[]);
        let detail = Some(check_detail(&outcome, None));
        assert_eq!(
            settled.next,
            Next::Stop(StopReason::CheckFailed { step: 3, detail }),
            "{outcome:?}"
        );
        assert_eq!(settled.driving.run, run);
        assert_eq!(settled.met, None);
    }
}

#[test]
fn a_failed_resume_probe_stops_with_check_failed() {
    let file = stopped_on(3);
    let started = start(&file, &[], &[]).unwrap();
    let settled = settle_check(&started.driving, Some(&file), &WatchOutcome::Exited(1), &[]);
    let detail = Some("exited with code 1".to_string());
    assert_eq!(
        settled.next,
        Next::Stop(StopReason::CheckFailed { step: 3, detail })
    );
}

#[test]
fn a_changed_unapproved_or_deleted_plan_stops_a_settling_check() {
    let file = approved(plan());
    let mut edited = plan();
    edited.goal = "ship GUZ-2".to_string();
    let unapproved = RunFile {
        approved: None,
        ..approved(plan())
    };
    for current in [Some(&approved(edited)), Some(&unapproved)] {
        let settled = settle_check(
            &checking(&file, 2, Run::default()),
            current,
            &WatchOutcome::Exited(0),
            &[],
        );
        assert_eq!(settled.next, Next::Stop(StopReason::PlanChanged));
        assert_eq!(settled.driving.progress, at(2));
    }
    let off = settle_check(
        &checking(&file, 2, Run::default()),
        None,
        &WatchOutcome::Exited(0),
        &[],
    );
    assert_eq!(off.next, Next::Stop(StopReason::Off));
}

#[test]
fn a_settled_check_at_the_turn_cap_stops_before_sending_the_next_step() {
    let file = approved(plan());
    let settled = settle_check(
        &checking(&file, 2, spent(30, 0.5)),
        Some(&file),
        &WatchOutcome::Exited(0),
        &[],
    );
    assert_eq!(settled.next, Next::Stop(StopReason::TurnCap));
}

#[test]
fn only_a_run_waiting_on_a_generated_check_watch_awaits_a_check() {
    let file = approved(plan());
    assert!(awaits_check(&checking(&file, 2, Run::default())));
    assert!(!awaits_check(&held(&file)));
    assert!(!awaits_check(&state(&file, 2, Awaiting::Turn)));
    assert!(!awaits_check(&state(&file, 3, Awaiting::Verdict)));
}

fn watch_report(id: &str, text: &str) -> Report {
    Report {
        worker: id.to_string(),
        text: text.to_string(),
        ..report("watch")
    }
}

fn output_report(output: &str) -> Report {
    watch_report(
        "watch:send-it-3",
        &format!(
            "Watch `watch:send-it-3` exited with code 1.\nCommand: cargo xtask lint\n\nOutput:\n{output}"
        ),
    )
}

#[test]
fn a_check_failure_detail_gives_the_exit_code_and_the_last_output_lines() {
    let report = output_report("one\ntwo\nthree\nfour\nfive");
    assert_eq!(
        check_detail(&WatchOutcome::Exited(1), Some(&report)),
        "exited with code 1; last output: three / four / five"
    );
}

#[test]
fn a_check_failure_detail_skips_the_follow_up_blank_lines_and_a_missing_output() {
    let followed = watch_report(
        "watch:send-it-3",
        "Watch `x` exited with code 2.\nCommand: c\n\nOutput:\nboom\n\n\nFollow-up: tell me",
    );
    assert_eq!(
        check_detail(&WatchOutcome::Exited(2), Some(&followed)),
        "exited with code 2; last output: boom"
    );
    let silent = output_report("(no output)");
    assert_eq!(
        check_detail(&WatchOutcome::Exited(1), Some(&silent)),
        "exited with code 1"
    );
    assert_eq!(
        check_detail(&WatchOutcome::Signaled(9), None),
        "was killed by signal 9"
    );
    let unrelated = watch_report("watch:send-it-3", "no output section here");
    assert_eq!(
        check_detail(&WatchOutcome::Exited(1), Some(&unrelated)),
        "exited with code 1"
    );
}

#[test]
fn a_long_output_tail_keeps_only_its_last_characters() {
    let report = output_report(&format!("{}END", "x".repeat(500)));
    let detail = check_detail(&WatchOutcome::Exited(1), Some(&report));
    let tail = detail
        .strip_prefix("exited with code 1; last output: ")
        .unwrap();
    assert!(tail.starts_with("..."), "{tail}");
    assert!(tail.ends_with("xxEND"), "{tail}");
    assert_eq!(tail.chars().count(), 203);
}

#[test]
fn a_check_failed_notice_carries_its_detail_in_brackets_and_stays_the_same_without_one() {
    let run = Run::default();
    let plain = StopReason::check_failed(3);
    assert_eq!(
        plain.notice(&run, None),
        format!("send-it stopped: step 3 check did not exit 0. {RESTART}")
    );
    let detailed = StopReason::CheckFailed {
        step: 3,
        detail: Some("exited with code 1; last output: oops.".to_string()),
    };
    assert_eq!(
        detailed.notice(&run, None),
        format!(
            "send-it stopped: step 3 check did not exit 0 (exited with code 1; last output: oops). {RESTART}"
        )
    );
}

#[test]
fn a_settled_check_failure_carries_the_exit_code_and_output_from_its_report() {
    let file = approved(plan());
    let reports = [
        watch_report("watch:send-it-1", "other"),
        output_report("lint error: bad"),
    ];
    let settled = settle_check(
        &checking(&file, 2, Run::default()),
        Some(&file),
        &WatchOutcome::Exited(1),
        &reports,
    );
    assert_eq!(
        settled.next,
        Next::Stop(StopReason::CheckFailed {
            step: 3,
            detail: Some("exited with code 1; last output: lint error: bad".to_string())
        })
    );
    let without = settle_check(
        &checking(&file, 2, Run::default()),
        Some(&file),
        &WatchOutcome::Exited(1),
        &[],
    );
    assert_eq!(
        without.next,
        Next::Stop(StopReason::CheckFailed {
            step: 3,
            detail: Some("exited with code 1".to_string())
        })
    );
}
