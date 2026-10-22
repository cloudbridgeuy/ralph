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

pub(super) fn typed_after_queue_key(bytes: &[u8]) -> String {
    let Some(start) = bytes.iter().position(|byte| *byte == QUEUE_KEY) else {
        return String::new();
    };
    let text = String::from_utf8_lossy(&bytes[start + 1..]);
    let mut kept = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => skip_escape(&mut chars),
            ch if ch.is_control() || ch == char::REPLACEMENT_CHARACTER => {}
            ch => kept.push(ch),
        }
    }
    kept
}

fn skip_escape(chars: &mut std::str::Chars) {
    match chars.next() {
        Some('[') => {
            for ch in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&ch) {
                    break;
                }
            }
        }
        Some('O') => {
            chars.next();
        }
        Some(_) | None => {}
    }
}

pub(super) fn draft_with_queue(draft: Option<String>, entries: &[String]) -> Option<String> {
    match (draft, merged(entries)) {
        (Some(draft), Some(queued)) => Some(format!("{draft}{JOIN}{queued}")),
        (draft, queued) => draft.or(queued),
    }
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
    fn seed_is_the_printable_text_after_ctrl_q() {
        assert_eq!(typed_after_queue_key(b"ab\x11hello"), "hello");
        assert_eq!(typed_after_queue_key(b"\x11h\xc3\xa9y"), "h\u{e9}y");
    }

    #[test]
    fn seed_without_ctrl_q_or_text_is_empty() {
        assert_eq!(typed_after_queue_key(b"abc"), "");
        assert_eq!(typed_after_queue_key(b"\x11"), "");
    }

    #[test]
    fn seed_drops_control_bytes_and_escape_sequences() {
        assert_eq!(typed_after_queue_key(b"\x11a\x03b\r\n\tc\x11d"), "abcd");
        assert_eq!(typed_after_queue_key(b"\x11a\x1b[Ab\x1bOPc\x1bxd"), "abcd");
        assert_eq!(typed_after_queue_key(b"\x11a\x1b[1;5C"), "a");
        assert_eq!(typed_after_queue_key(b"\x11a\x1b[1;5"), "a");
        assert_eq!(typed_after_queue_key(b"\x11a\x1b"), "a");
    }

    #[test]
    fn seed_drops_invalid_utf8() {
        assert_eq!(typed_after_queue_key(b"\x11a\xc3"), "a");
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
