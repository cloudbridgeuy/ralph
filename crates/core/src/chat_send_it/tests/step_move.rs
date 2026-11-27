use super::*;
use crate::worker::{ReportStatus, Sender};

fn moved(worker: &str, status: &str) -> Next {
    advance(
        &plan(),
        Progress::default(),
        Event::Report(&worker_report(worker, status)),
    )
    .1
}

#[test]
fn awaited_worker_blocked_stops_with_needs_user_cause() {
    let next = moved("E1", "blocked");
    assert!(
        matches!(next, Next::Stop(StopReason::WorkerFlagged { .. })),
        "{next:?}"
    );
}

#[test]
fn other_worker_question_stops_with_needs_user_cause() {
    let next = moved("E2", "question");
    assert!(
        matches!(next, Next::Stop(StopReason::WorkerFlagged { .. })),
        "{next:?}"
    );
}

#[test]
fn other_worker_done_holds() {
    assert_eq!(moved("E2", "done"), Next::Wait);
}

#[test]
fn awaited_worker_check_fails_step() {
    assert_eq!(
        moved("E1", "check"),
        Next::Stop(StopReason::Unmet { step: 1 })
    );
}

fn expected(kind: ReportKind, from: &str, awaited: bool) -> StepMove {
    match kind {
        ReportKind::Worker(ReportStatus::Blocked | ReportStatus::Question) => {
            StepMove::Stop(StopCause::NeedsUser {
                worker: Sender::parse(from).unwrap(),
            })
        }
        ReportKind::Worker(ReportStatus::Done) if awaited => StepMove::Complete,
        ReportKind::Worker(ReportStatus::Done) => StepMove::Hold,
        ReportKind::Check
        | ReportKind::Warning
        | ReportKind::Watch
        | ReportKind::Merged
        | ReportKind::View
        | ReportKind::ThreadClosed
        | ReportKind::External
            if awaited =>
        {
            StepMove::Fail
        }
        ReportKind::Check
        | ReportKind::Warning
        | ReportKind::Watch
        | ReportKind::Merged
        | ReportKind::View
        | ReportKind::ThreadClosed
        | ReportKind::External => StepMove::Hold,
    }
}

#[test]
fn step_move_covers_every_report_kind() {
    let e1 = WorkerId::parse("E1").unwrap();
    for kind in ReportKind::ALL {
        for (from, awaited, from_awaited) in [
            ("E1", Some(&e1), true),
            ("E2", Some(&e1), false),
            ("E1", None, false),
            ("watch:nightly", None, false),
        ] {
            let report = worker_report(from, kind.as_str());
            assert_eq!(
                step_move(&report, awaited),
                expected(kind, from, from_awaited),
                "{kind} from {from}"
            );
        }
    }
}
