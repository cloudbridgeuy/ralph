use crate::chat_status::truncate_to_width;
use crate::chat_thread::{AutoMode, ThreadMeta, ThreadName, THREAD_REPORT_PREFIX};
use crate::worker::{Report, ReportKind};

pub struct ThreadClosedNotice {
    pub merge: &'static str,
    pub drop: &'static str,
}

pub const THREAD_CLOSED_NOTICE: ThreadClosedNotice = ThreadClosedNotice {
    merge: " closed \u{b7} Enter to merge or keep it",
    drop: " closed \u{b7} Enter to drop or keep it",
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseEvent {
    Closed,
    Switched,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosePlan {
    Nothing,
    MarkOnly,
    MarkAndReport,
}

pub fn close_plan(event: CloseEvent, meta: &ThreadMeta) -> ClosePlan {
    if meta.auto.is_none() || meta.merged.is_some() || meta.parked.is_some() {
        return ClosePlan::Nothing;
    }
    match event {
        CloseEvent::Closed => ClosePlan::MarkAndReport,
        CloseEvent::Switched => ClosePlan::MarkOnly,
        CloseEvent::Other => ClosePlan::Nothing,
    }
}

pub fn thread_closed_notice(name: &ThreadName, auto: AutoMode) -> String {
    let suffix = match auto {
        AutoMode::Merge => THREAD_CLOSED_NOTICE.merge,
        AutoMode::Drop => THREAD_CLOSED_NOTICE.drop,
    };
    format!("{name}{suffix}")
}

pub fn thread_closed_report(name: &ThreadName, auto: AutoMode, at: &str) -> Report {
    Report::thread_closed(
        &format!("{THREAD_REPORT_PREFIX}{name}"),
        at,
        thread_closed_notice(name, auto),
    )
}

pub fn with_auto(meta: &ThreadMeta, auto: Option<AutoMode>) -> Option<ThreadMeta> {
    auto.filter(|auto| meta.auto != Some(*auto))
        .map(|auto| ThreadMeta {
            auto: Some(auto),
            ..meta.clone()
        })
}

pub fn clear_closed(meta: &ThreadMeta) -> Option<ThreadMeta> {
    meta.closed.then(|| ThreadMeta {
        closed: false,
        ..meta.clone()
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingClose {
    pub name: ThreadName,
    pub auto: AutoMode,
}

pub fn pending_closes(threads: &[(ThreadName, ThreadMeta)]) -> Vec<PendingClose> {
    threads
        .iter()
        .filter(|(_, meta)| meta.closed && meta.merged.is_none() && meta.parked.is_none())
        .filter_map(|(name, meta)| {
            meta.auto.map(|auto| PendingClose {
                name: name.clone(),
                auto,
            })
        })
        .collect()
}

pub fn close_footer(closes: &[PendingClose], width: usize) -> Option<String> {
    let first = closes.first()?;
    let notice = thread_closed_notice(&first.name, first.auto);
    let line = match closes.len() {
        1 => notice,
        count => format!("{count} threads closed \u{b7} {notice}"),
    };
    Some(truncate_to_width(&line, width))
}

pub fn fresh_closes(pending: &[PendingClose], shown: &[ThreadName]) -> Vec<PendingClose> {
    pending
        .iter()
        .filter(|close| !shown.contains(&close.name))
        .cloned()
        .collect()
}

pub fn close_names(pending: &[PendingClose]) -> Vec<ThreadName> {
    pending.iter().map(|close| close.name.clone()).collect()
}

fn verb(auto: AutoMode) -> &'static str {
    match auto {
        AutoMode::Merge => "merge",
        AutoMode::Drop => "drop",
    }
}

pub fn close_question(name: &ThreadName, auto: AutoMode) -> String {
    format!("{} {name}? [y/n]", verb(auto))
}

pub fn close_answer(line: Option<&str>) -> bool {
    line.is_some_and(|line| matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
}

pub fn is_close_report(report: &Report) -> bool {
    report.status == ReportKind::ThreadClosed && report.worker.starts_with(THREAD_REPORT_PREFIX)
}

pub fn split_close_reports(reports: Vec<Report>) -> (Vec<Report>, Vec<Report>) {
    reports.into_iter().partition(is_close_report)
}

pub fn close_refusal(name: &ThreadName, auto: AutoMode, reason: &str) -> String {
    format!(
        "{reason}\nthread {name} is kept; `ralph thread {} {name}` retries it.",
        verb(auto)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta_with(auto: Option<AutoMode>, closed: bool) -> ThreadMeta {
        ThreadMeta {
            base: 1,
            created: "t".to_string(),
            merged: None,
            parked: None,
            auto,
            model: None,
            closed,
        }
    }

    #[test]
    fn with_auto_sets_or_replaces_the_choice_and_ignores_none() {
        let fresh = meta_with(None, false);
        assert_eq!(
            with_auto(&fresh, Some(AutoMode::Merge)),
            Some(meta_with(Some(AutoMode::Merge), false))
        );
        let stored = meta_with(Some(AutoMode::Merge), false);
        assert_eq!(
            with_auto(&stored, Some(AutoMode::Drop)),
            Some(meta_with(Some(AutoMode::Drop), false))
        );
        assert_eq!(with_auto(&stored, Some(AutoMode::Merge)), None);
        assert_eq!(with_auto(&stored, None), None);
    }

    #[test]
    fn clear_closed_keeps_auto_and_skips_an_open_thread() {
        assert_eq!(
            clear_closed(&meta_with(Some(AutoMode::Drop), true)),
            Some(meta_with(Some(AutoMode::Drop), false))
        );
        assert_eq!(clear_closed(&meta_with(None, false)), None);
    }

    fn name() -> ThreadName {
        ThreadName::parse("side").unwrap()
    }

    fn live(auto: Option<AutoMode>) -> ThreadMeta {
        meta_with(auto, false)
    }

    #[test]
    fn close_plan_table() {
        let merge = live(Some(AutoMode::Merge));
        assert_eq!(
            close_plan(CloseEvent::Closed, &merge),
            ClosePlan::MarkAndReport
        );
        assert_eq!(
            close_plan(CloseEvent::Switched, &merge),
            ClosePlan::MarkOnly
        );
        assert_eq!(close_plan(CloseEvent::Other, &merge), ClosePlan::Nothing);
        for event in [CloseEvent::Closed, CloseEvent::Switched] {
            assert_eq!(close_plan(event, &live(None)), ClosePlan::Nothing);
            let merged = ThreadMeta {
                merged: Some(crate::chat_thread::Merged {
                    at: "t".to_string(),
                    digest: "d".to_string(),
                    landed: None,
                }),
                ..merge.clone()
            };
            assert_eq!(close_plan(event, &merged), ClosePlan::Nothing);
            let parked = ThreadMeta {
                parked: Some(crate::chat_thread::Parked {
                    at: "t".to_string(),
                }),
                ..merge.clone()
            };
            assert_eq!(close_plan(event, &parked), ClosePlan::Nothing);
        }
    }

    #[test]
    fn notice_text_is_exact_for_both_modes() {
        assert_eq!(
            thread_closed_notice(&name(), AutoMode::Merge),
            "side closed \u{b7} Enter to merge or keep it"
        );
        assert_eq!(
            thread_closed_notice(&name(), AutoMode::Drop),
            "side closed \u{b7} Enter to drop or keep it"
        );
    }

    #[test]
    fn report_carries_the_notice_under_a_per_thread_worker() {
        let report = thread_closed_report(&name(), AutoMode::Drop, "2026-01-01T00:00:00Z");
        assert_eq!(report.worker, "thread:side");
        assert_eq!(report.at, "2026-01-01T00:00:00Z");
        assert_eq!(report.status.as_str(), "thread-closed");
        assert_eq!(report.text, "side closed \u{b7} Enter to drop or keep it");
    }

    fn pending(name: &str, auto: AutoMode) -> PendingClose {
        PendingClose {
            name: ThreadName::parse(name).unwrap(),
            auto,
        }
    }

    #[test]
    fn close_footer_is_the_notice_for_one_pending_close_in_both_modes() {
        assert_eq!(
            close_footer(&[pending("auth", AutoMode::Merge)], 200).unwrap(),
            "auth closed \u{b7} Enter to merge or keep it"
        );
        assert_eq!(
            close_footer(&[pending("auth", AutoMode::Drop)], 200).unwrap(),
            "auth closed \u{b7} Enter to drop or keep it"
        );
    }

    #[test]
    fn close_footer_counts_several_and_names_the_first() {
        let closes = [
            pending("auth", AutoMode::Merge),
            pending("db", AutoMode::Drop),
        ];
        assert_eq!(
            close_footer(&closes, 200).unwrap(),
            "2 threads closed \u{b7} auth closed \u{b7} Enter to merge or keep it"
        );
    }

    #[test]
    fn fresh_closes_lists_an_unshown_close_once() {
        let pending = [pending("auth", AutoMode::Merge)];
        let fresh = fresh_closes(&pending, &[]);
        assert_eq!(fresh, pending);
        assert!(fresh_closes(&pending, &close_names(&fresh)).is_empty());
    }

    #[test]
    fn fresh_closes_keeps_only_the_new_names() {
        let pending = [
            pending("auth", AutoMode::Merge),
            pending("db", AutoMode::Drop),
        ];
        let shown = [ThreadName::parse("auth").unwrap()];
        assert_eq!(
            close_footer(&fresh_closes(&pending, &shown), 200).unwrap(),
            "db closed \u{b7} Enter to drop or keep it"
        );
    }

    #[test]
    fn fresh_closes_of_two_threads_is_the_counted_footer() {
        let pending = [
            pending("auth", AutoMode::Merge),
            pending("db", AutoMode::Drop),
        ];
        assert_eq!(
            close_footer(&fresh_closes(&pending, &[]), 200).unwrap(),
            "2 threads closed \u{b7} auth closed \u{b7} Enter to merge or keep it"
        );
    }

    #[test]
    fn a_name_no_longer_pending_is_forgotten_and_announces_again() {
        let both = [
            pending("auth", AutoMode::Merge),
            pending("db", AutoMode::Merge),
        ];
        let settled = [pending("db", AutoMode::Merge)];
        let shown = close_names(&settled);
        assert_eq!(shown, [ThreadName::parse("db").unwrap()]);
        assert_eq!(
            fresh_closes(&both, &shown),
            [pending("auth", AutoMode::Merge)]
        );
    }

    #[test]
    fn close_footer_is_absent_with_nothing_pending_and_fits_the_width() {
        assert_eq!(close_footer(&[], 200), None);
        let one = [pending("auth", AutoMode::Merge)];
        assert!((1..60).all(|w| close_footer(&one, w).unwrap().chars().count() <= w));
    }

    fn closed_meta(auto: Option<AutoMode>) -> ThreadMeta {
        ThreadMeta {
            closed: true,
            ..meta_with(auto, false)
        }
    }

    #[test]
    fn pending_closes_table() {
        let side = name();
        let other = ThreadName::parse("other").unwrap();
        let merged = ThreadMeta {
            merged: Some(crate::chat_thread::Merged {
                at: "t".to_string(),
                digest: "d".to_string(),
                landed: None,
            }),
            ..closed_meta(Some(AutoMode::Merge))
        };
        let parked = ThreadMeta {
            parked: Some(crate::chat_thread::Parked {
                at: "t".to_string(),
            }),
            ..closed_meta(Some(AutoMode::Merge))
        };
        let threads = vec![
            (side.clone(), closed_meta(Some(AutoMode::Drop))),
            (other.clone(), closed_meta(Some(AutoMode::Merge))),
            (
                ThreadName::parse("open").unwrap(),
                live(Some(AutoMode::Merge)),
            ),
            (ThreadName::parse("plain").unwrap(), closed_meta(None)),
            (ThreadName::parse("done").unwrap(), merged),
            (ThreadName::parse("away").unwrap(), parked),
        ];
        assert_eq!(
            pending_closes(&threads),
            vec![
                PendingClose {
                    name: side,
                    auto: AutoMode::Drop
                },
                PendingClose {
                    name: other,
                    auto: AutoMode::Merge
                },
            ]
        );
        assert!(pending_closes(&[]).is_empty());
    }

    #[test]
    fn close_question_is_exact_for_both_modes() {
        assert_eq!(
            close_question(&name(), AutoMode::Merge),
            "merge side? [y/n]"
        );
        assert_eq!(close_question(&name(), AutoMode::Drop), "drop side? [y/n]");
    }

    #[test]
    fn only_an_explicit_yes_is_yes() {
        for yes in ["y", "yes", " Y\n", "YES"] {
            assert!(close_answer(Some(yes)), "{yes}");
        }
        for no in ["n", "no", "", "\n", "maybe", "yy"] {
            assert!(!close_answer(Some(no)), "{no}");
        }
        assert!(!close_answer(None));
    }

    #[test]
    fn close_reports_split_from_digests_and_workers() {
        let closed = thread_closed_report(&name(), AutoMode::Merge, "t");
        let digest = Report::merged("thread:side", "t", "x".to_string());
        let worker = Report::from_worker(
            "w1",
            "t",
            crate::worker::ReportStatus::Done,
            "x".to_string(),
        );
        assert!(is_close_report(&closed));
        assert!(!is_close_report(&digest));
        let (closes, rest) =
            split_close_reports(vec![digest.clone(), closed.clone(), worker.clone()]);
        assert_eq!(closes, vec![closed]);
        assert_eq!(rest, vec![digest, worker]);
    }

    #[test]
    fn a_refusal_names_the_reason_and_the_manual_command() {
        let merge = close_refusal(&name(), AutoMode::Merge, "thread side is in use");
        assert!(merge.contains("thread side is in use"), "{merge}");
        assert!(merge.contains("ralph thread merge side"), "{merge}");
        let drop = close_refusal(&name(), AutoMode::Drop, "no thread side");
        assert!(drop.contains("ralph thread drop side"), "{drop}");
    }
}
