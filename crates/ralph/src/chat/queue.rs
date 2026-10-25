use crate::chat::turn::TurnOutcome;

const JOIN: &str = "\n\n";

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Queue {
    pub(super) entries: Vec<String>,
    pub(super) held: bool,
    pub(super) typed: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum EditorExit {
    Submitted(String),
    BackOut(String),
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
        EditorExit::BackOut(draft) => {
            if !draft.trim().is_empty() {
                queue.entries.push(draft);
            }
            queue.held = true;
            (queue, Action::Resume)
        }
        EditorExit::Closed => (queue, Action::Resume),
        EditorExit::Cancelled => {
            queue.held = true;
            (queue, Action::Cancel)
        }
    }
}

pub(super) fn merged(entries: &[String]) -> Option<String> {
    (!entries.is_empty()).then(|| entries.join(JOIN))
}

pub(super) fn draft_with_queue(draft: Option<String>, entries: &[String]) -> Option<String> {
    match (draft, merged(entries)) {
        (Some(draft), Some(queued)) => Some(format!("{draft}{JOIN}{queued}")),
        (draft, queued) => draft.or(queued),
    }
}

pub(super) fn unsent(queue: &Queue) -> Vec<String> {
    queue
        .entries
        .iter()
        .map(String::as_str)
        .chain(Some(queue.typed.as_str()))
        .filter(|text| !text.trim().is_empty())
        .map(str::to_string)
        .collect()
}

pub(super) fn exit_notice(count: usize) -> Option<String> {
    match count {
        0 => None,
        1 => Some("1 queued message not sent; saved to history.".to_string()),
        n => Some(format!("{n} queued messages not sent; saved to history.")),
    }
}

pub(super) fn draft_with_typed(draft: Option<String>, typed: &str) -> Option<String> {
    match (draft, typed) {
        (draft, "") => draft,
        (Some(draft), typed) => Some(format!("{draft}{JOIN}{typed}")),
        (None, typed) => Some(typed.to_string()),
    }
}

pub(super) fn decide(outcome: TurnOutcome, queue: Queue) -> Decision {
    match merged(&queue.entries) {
        Some(text) if outcome == TurnOutcome::Replied && !queue.held => Decision::Send(text),
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
            held: false,
            typed: String::new(),
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
        assert!(queue.held);
        assert_eq!(action, Action::Cancel);
    }

    #[test]
    fn back_out_adds_the_draft_and_holds_the_queue() {
        let (queue, action) = on_exit(queue(&["a"]), EditorExit::BackOut("b".into()));
        assert_eq!(queue.entries, entries(&["a", "b"]));
        assert!(queue.held);
        assert_eq!(action, Action::Resume);
    }

    #[test]
    fn back_out_with_a_blank_draft_adds_nothing() {
        let (queue, action) = on_exit(queue(&["a"]), EditorExit::BackOut(" \n".into()));
        assert_eq!(queue.entries, entries(&["a"]));
        assert!(queue.held);
        assert_eq!(action, Action::Resume);
    }

    #[test]
    fn back_out_after_a_reply_prefills_instead_of_sending() {
        let (held, _) = on_exit(queue(&["a"]), EditorExit::BackOut("b".into()));
        let Decision::Keep(kept) = decide(TurnOutcome::Replied, held) else {
            panic!("a backed-out queue must not send");
        };
        assert_eq!(draft_with_queue(None, &kept), Some("a\n\nb".to_string()));
    }

    #[test]
    fn queue_follows_the_restored_draft() {
        assert_eq!(
            draft_with_queue(Some("msg".into()), &entries(&["a", "b"])),
            Some("msg\n\na\n\nb".to_string())
        );
    }

    #[test]
    fn queue_alone_or_draft_alone_is_unchanged() {
        assert_eq!(
            draft_with_queue(None, &entries(&["a"])),
            Some("a".to_string())
        );
        assert_eq!(
            draft_with_queue(Some("msg".into()), &[]),
            Some("msg".to_string())
        );
        assert_eq!(draft_with_queue(None, &[]), None);
    }

    #[test]
    fn exit_notice_counts_entries_and_typed_text() {
        let queue = Queue {
            entries: entries(&["a", " "]),
            held: false,
            typed: "xyz".into(),
        };
        assert_eq!(unsent(&queue), entries(&["a", "xyz"]));
        assert_eq!(exit_notice(0), None);
        assert_eq!(
            exit_notice(1),
            Some("1 queued message not sent; saved to history.".to_string())
        );
        assert_eq!(
            exit_notice(2),
            Some("2 queued messages not sent; saved to history.".to_string())
        );
    }

    #[test]
    fn typed_text_follows_the_draft_and_is_never_sent() {
        assert_eq!(draft_with_typed(None, ""), None);
        assert_eq!(draft_with_typed(None, "xyz"), Some("xyz".to_string()));
        assert_eq!(
            draft_with_typed(Some("a".into()), "xyz"),
            Some("a\n\nxyz".to_string())
        );
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
