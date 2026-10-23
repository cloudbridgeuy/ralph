use super::*;
use crate::chat_status::{WatchState, WorkerState};

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
        memory: Memory {
            msgs: 132,
            tokens: 4100,
        },
        turn: None,
        napping: false,
        queue: None,
    }
}

pub(super) fn turn() -> TurnCache {
    TurnCache {
        input: 100,
        read: 12_300,
        write: 400,
        cost_usd: Some(0.0412),
    }
}

pub(super) fn with_turn() -> Status {
    Status {
        turn: Some(turn()),
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
        ..status()
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
