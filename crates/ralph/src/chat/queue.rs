use crate::chat::turn::TurnOutcome;

pub(super) const QUEUE_KEY: u8 = 0x11;
const JOIN: &str = "\n\n";

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Queue {
    pub(super) entries: Vec<String>,
    pub(super) cancelled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum EditorExit {
    Submitted(String),
    Closed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    Resume,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Decision {
    Send(String),
    Keep(Vec<String>),
}

pub(super) fn on_exit(mut queue: Queue, exit: EditorExit) -> (Queue, Action) {
    match exit {
        EditorExit::Submitted(text) => {
            if !text.trim().is_empty() {
                queue.entries.push(text);
            }
            (queue, Action::Resume)
        }
        EditorExit::Closed => (queue, Action::Resume),
        EditorExit::Cancelled => {
            queue.cancelled = true;
            (queue, Action::Cancel)
        }
    }
}

pub(super) fn merged(entries: &[String]) -> Option<String> {
    (!entries.is_empty()).then(|| entries.join(JOIN))
}

pub(super) fn queued_label(count: usize) -> Option<String> {
    (count > 0).then(|| format!("QUEUED {count}"))
}

pub(super) fn has_queue_key(bytes: &[u8]) -> bool {
    bytes.contains(&QUEUE_KEY)
}

pub(super) fn decide(outcome: TurnOutcome, queue: Queue) -> Decision {
    match merged(&queue.entries) {
        Some(text) if outcome == TurnOutcome::Replied && !queue.cancelled => Decision::Send(text),
        _ => Decision::Keep(queue.entries),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    fn queue(items: &[&str]) -> Queue {
        Queue {
            entries: entries(items),
            cancelled: false,
        }
    }

    #[test]
    fn merges_in_typed_order_with_blank_line() {
        assert_eq!(
            merged(&entries(&["a", "b", "c"])),
            Some("a\n\nb\n\nc".to_string())
        );
    }

    #[test]
    fn merge_of_nothing_is_none() {
        assert_eq!(merged(&[]), None);
    }

    #[test]
    fn merge_of_one_entry_is_unchanged() {
        assert_eq!(
            merged(&entries(&["/skill x"])),
            Some("/skill x".to_string())
        );
    }

    #[test]
    fn submit_adds_entry_and_resumes() {
        let (queue, action) = on_exit(queue(&["a"]), EditorExit::Submitted("b".into()));
        assert_eq!(queue.entries, entries(&["a", "b"]));
        assert_eq!(action, Action::Resume);
    }

    #[test]
    fn blank_submit_adds_nothing() {
        let (queue, action) = on_exit(queue(&["a"]), EditorExit::Submitted(" \n\t".into()));
        assert_eq!(queue.entries, entries(&["a"]));
        assert_eq!(action, Action::Resume);
    }

    #[test]
    fn close_adds_nothing_and_resumes() {
        let (queue, action) = on_exit(queue(&["a"]), EditorExit::Closed);
        assert_eq!(queue.entries, entries(&["a"]));
        assert_eq!(action, Action::Resume);
    }

    #[test]
    fn cancel_keeps_entries_and_cancels() {
        let (queue, action) = on_exit(queue(&["a"]), EditorExit::Cancelled);
        assert_eq!(queue.entries, entries(&["a"]));
        assert!(queue.cancelled);
        assert_eq!(action, Action::Cancel);
    }

    #[test]
    fn label_shows_only_for_a_nonempty_queue() {
        assert_eq!(queued_label(0), None);
        assert_eq!(queued_label(1), Some("QUEUED 1".to_string()));
        assert_eq!(queued_label(12), Some("QUEUED 12".to_string()));
    }

    #[test]
    fn finds_ctrl_q_byte() {
        assert!(has_queue_key(b"ab\x11c"));
        assert!(!has_queue_key(b"abc\x03"));
        assert!(!has_queue_key(b""));
    }

    #[test]
    fn replied_turn_sends_merged_entries() {
        assert_eq!(
            decide(TurnOutcome::Replied, queue(&["a", "b"])),
            Decision::Send("a\n\nb".to_string())
        );
    }

    #[test]
    fn replied_turn_with_empty_queue_keeps_nothing() {
        assert_eq!(
            decide(TurnOutcome::Replied, Queue::default()),
            Decision::Keep(Vec::new())
        );
    }

    #[test]
    fn failed_or_cancelled_turn_keeps_entries() {
        assert_eq!(
            decide(TurnOutcome::Unanswered, queue(&["a"])),
            Decision::Keep(entries(&["a"]))
        );
    }

    #[test]
    fn exit_keeps_entries() {
        assert_eq!(
            decide(TurnOutcome::Exit, queue(&["a"])),
            Decision::Keep(entries(&["a"]))
        );
    }

    #[test]
    fn editor_cancel_after_reply_keeps_entries() {
        let (cancelled, _) = on_exit(queue(&["a"]), EditorExit::Cancelled);
        assert_eq!(
            decide(TurnOutcome::Replied, cancelled),
            Decision::Keep(entries(&["a"]))
        );
    }
}
