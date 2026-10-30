use super::*;
use crate::chat_status::{ScheduleState, WatchState, WorkerState};
use crate::chat_thread::{SessionCost, ThreadCell};

pub(super) fn status() -> Status {
    Status {
        unread: 2,
        workers: vec![
            WorkerState {
                id: "GUZ-416".into(),
                state: "running".into(),
            },
            WorkerState {
                id: "GUZ-417".into(),
                state: "question".into(),
            },
        ],
        watches: Vec::new(),
        schedule: Vec::new(),
        thread: None,
        napping: false,
        queue: None,
        hold: None,
    }
}

pub(super) fn thread() -> ThreadCell {
    ThreadCell {
        turn: 12,
        max_turns: 30,
        ctx_percent: Some(62),
        cache_minutes: Some(41),
        write_heavy: false,
        reset: None,
        shift: Some(9),
        cost: Some(SessionCost {
            total: 4.123,
            delta: Some(0.314),
            missing: false,
        }),
    }
}

pub(super) fn with_thread() -> Status {
    Status {
        thread: Some(thread()),
        ..status()
    }
}

pub(super) fn with_watches() -> Status {
    Status {
        watches: vec![
            WatchState {
                id: "w1".into(),
                left: Some("12m".into()),
            },
            WatchState {
                id: "w2".into(),
                left: None,
            },
        ],
        ..with_thread()
    }
}

pub(super) fn with_schedule(count: usize) -> Status {
    Status {
        schedule: (1..=count)
            .map(|n| ScheduleState {
                id: format!("job{n}"),
                time: format!("0{n}:30"),
            })
            .collect(),
        ..with_watches()
    }
}

pub(super) fn context() -> Context {
    Context {
        store: ".ralph/chat".into(),
    }
}

pub(super) fn text(spans: &[Span]) -> String {
    spans.iter().map(|span| span.text.as_str()).collect()
}

pub(super) fn line(status: &Status, width: usize) -> String {
    text(&bar(status, &context(), width, Separator::Block))
}

pub(super) fn placed_texts(status: &Status, width: usize) -> Vec<String> {
    let all = segments(status, &context());
    layout(&all, width, Separator::Block)
        .iter()
        .map(|p| all[p.segment].variants[p.variant].clone())
        .collect()
}
