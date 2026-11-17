use crate::chat_thread::{AutoMode, ThreadMeta, ThreadName, THREAD_REPORT_PREFIX};
use crate::worker::Report;

pub const THREAD_CLOSED_STATUS: &str = "thread-closed";

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
    Report {
        worker: format!("{THREAD_REPORT_PREFIX}{name}"),
        at: at.to_string(),
        status: THREAD_CLOSED_STATUS.to_string(),
        text: thread_closed_notice(name, auto),
    }
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
        assert_eq!(report.status, "thread-closed");
        assert_eq!(report.text, "side closed \u{b7} Enter to drop or keep it");
    }
}
