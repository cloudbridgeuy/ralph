use super::*;

#[test]
fn pending_lines_are_the_pending_prefixed_lines_of_a_reply() {
    let reply = "Done with step 2.\nPending: push main (13 commits ahead)\n  Pending:   run lint on main  \nNext step: /hold";
    assert_eq!(
        pending_lines(reply),
        ["push main (13 commits ahead)", "run lint on main"]
    );
}

#[test]
fn pending_none_and_empty_pending_lines_are_skipped() {
    assert!(pending_lines("Pending: none\nPending: Nothing\nPending:\nPending:   ").is_empty());
    assert!(pending_lines("no marker here\nthe Pending: mid-line one").is_empty());
}

#[test]
fn a_long_pending_line_is_cut_to_200_characters() {
    let long = "x".repeat(300);
    let lines = pending_lines(&format!("Pending: {long}"));
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].chars().count(), 200);
    assert!(lines[0].ends_with("..."), "{}", lines[0]);
}

fn ticked_with(pending: &[&str]) -> Ticked {
    let file = approved(plan());
    let mut ticked = start(&file, &[], &[]).unwrap();
    ticked.driving.tally.pending = pending.iter().map(ToString::to_string).collect();
    ticked
}

#[test]
fn a_reply_adds_its_pending_lines_once_in_order() {
    let ticked = with_reply(ticked_with(&["a"]), "Pending: b\nPending: a\nPending: c");
    assert_eq!(ticked.driving.tally.pending, ["a", "b", "c"]);
}

#[test]
fn pending_keeps_at_most_five_items() {
    let reply = (1..=8)
        .map(|n| format!("Pending: item {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let ticked = with_reply(ticked_with(&[]), &reply);
    assert_eq!(ticked.driving.tally.pending.len(), 5);
    assert_eq!(ticked.driving.tally.pending[4], "item 5");
}

#[test]
fn a_judged_met_step_keeps_its_evidence_and_a_later_plain_met_step_drops_it() {
    let file = approved(plan());
    let met = judged(true, "all 3 findings fixed");
    let judged_met = tick(
        &driving(3, Awaiting::Verdict, spent(2, 0.5)),
        &Ended {
            step_result: Some(&met),
            ..ended(&file)
        },
    );
    assert_eq!(
        judged_met.driving.tally.last_met.as_deref(),
        Some("all 3 findings fixed")
    );
    let report = [worker_report("E1", "done")];
    let plain_met = tick(
        &driving(0, Awaiting::Report("E1".to_string()), spent(1, 0.1)),
        &Ended {
            drained: &report,
            ..ended(&file)
        },
    );
    assert_eq!(plain_met.driving.tally.last_met, None);
}

#[test]
fn a_flagged_stop_names_the_worker_and_its_first_report_line() {
    let file = approved(plan());
    let flagged = [Report {
        text: "\n  Keep the old flag name or rename it?\nmore detail".to_string(),
        ..worker_report("GUZ-1606", "question")
    }];
    let ticked = tick(
        &driving(0, Awaiting::Turn, Run::default()),
        &Ended {
            unread: &flagged,
            ..ended(&file)
        },
    );
    assert_eq!(
        ticked.next,
        Next::Stop(StopReason::WorkerFlagged {
            worker: "GUZ-1606".to_string(),
            line: "Keep the old flag name or rename it?".to_string(),
        })
    );
}

#[test]
fn the_run_brief_asks_for_pending_lines_and_no_next_step() {
    let started = start(&approved(plan()), &[], &[]).unwrap();
    let Next::Send(text) = started.next else {
        panic!("a first worker step is sent")
    };
    assert!(text.contains("`Pending: <one line>`"), "{text}");
    assert!(text.contains("no `Next step:` line"), "{text}");
}

#[test]
fn every_stop_but_off_prefills_send_it_go() {
    let stops = [
        StopReason::TurnCap,
        StopReason::CostCap,
        StopReason::Unmet { step: 2 },
        flagged_stop(),
        StopReason::JudgeRounds {
            step: 1,
            evidence: "no".to_string(),
        },
        StopReason::CheckFailed {
            step: 3,
            detail: None,
        },
        StopReason::NoReply,
        StopReason::TookOver,
    ];
    for stop in stops {
        assert_eq!(
            run_end_prefill(&Next::Stop(stop.clone())).as_deref(),
            Some("/send-it go"),
            "{stop:?}"
        );
    }
}

#[test]
fn done_and_off_prefill_nothing() {
    assert_eq!(run_end_prefill(&Next::Done), None);
    assert_eq!(run_end_prefill(&Next::Stop(StopReason::Off)), None);
}

#[test]
fn a_changed_plan_prefills_send_it_status() {
    assert_eq!(
        run_end_prefill(&Next::Stop(StopReason::PlanChanged)).as_deref(),
        Some("/send-it status")
    );
}
