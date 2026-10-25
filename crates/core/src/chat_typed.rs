const QUEUE_KEY: char = '\u{11}';
const ESCAPE: char = '\u{1b}';
const BELL: char = '\u{7}';

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Typed {
    Open(String),
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueCtrlC {
    ClearDraft,
    CancelTurn,
}

#[derive(Default)]
struct Scan {
    text: String,
    queue_key: bool,
    cut: bool,
}

fn skip_string(chars: &mut std::str::Chars) -> bool {
    loop {
        match chars.next() {
            None => return false,
            Some(BELL) => return true,
            Some(ESCAPE) => return chars.next().is_some(),
            Some(_) => {}
        }
    }
}

fn skip_escape(chars: &mut std::str::Chars) -> bool {
    match chars.next() {
        None => false,
        Some('[') => chars.any(|ch| ('\u{40}'..='\u{7e}').contains(&ch)),
        Some('O') => chars.next().is_some(),
        Some(']' | 'P' | '_' | 'X' | '^') => skip_string(chars),
        Some(_) => true,
    }
}

fn scan(bytes: &[u8]) -> Scan {
    let text = String::from_utf8_lossy(bytes);
    let mut scan = Scan::default();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        match ch {
            ESCAPE => scan.cut = !skip_escape(&mut chars),
            QUEUE_KEY => scan.queue_key = true,
            '\t' | '\n' => scan.text.push(ch),
            ch if ch.is_control() || ch == char::REPLACEMENT_CHARACTER => {}
            ch => scan.text.push(ch),
        }
    }
    scan
}

fn ends_mid_char(mut rest: &[u8]) -> bool {
    loop {
        match std::str::from_utf8(rest) {
            Ok(_) => return false,
            Err(error) => match error.error_len() {
                None => return true,
                Some(len) => rest = &rest[error.valid_up_to() + len..],
            },
        }
    }
}

pub fn typed(bytes: &[u8]) -> Typed {
    let scan = scan(bytes);
    let seed = scan.text.trim_end_matches('\n');
    if scan.queue_key || !seed.is_empty() {
        Typed::Open(seed.to_string())
    } else {
        Typed::Ignore
    }
}

pub fn incomplete(bytes: &[u8]) -> bool {
    ends_mid_char(bytes) || scan(bytes).cut
}

pub fn queue_ctrl_c(draft: &str) -> QueueCtrlC {
    if draft.is_empty() {
        QueueCtrlC::CancelTurn
    } else {
        QueueCtrlC::ClearDraft
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(text: &str) -> Typed {
        Typed::Open(text.to_string())
    }

    #[test]
    fn ctrl_q_alone_opens_empty() {
        assert_eq!(typed(b"\x11"), open(""));
    }

    #[test]
    fn text_before_and_after_ctrl_q_is_kept() {
        assert_eq!(typed(b"ab\x11cd"), open("abcd"));
        assert_eq!(typed(b"\x11h\xc3\xa9y"), open("h\u{e9}y"));
    }

    #[test]
    fn plain_text_opens_with_it() {
        assert_eq!(typed(b"/workers\n"), open("/workers"));
    }

    #[test]
    fn arrow_mixed_with_text_keeps_the_text() {
        assert_eq!(typed(b"ab\x1b[Ac"), open("abc"));
        assert_eq!(typed(b"a\x1b[1;5Cb\x1bOPc\x1bxd"), open("abcd"));
    }

    #[test]
    fn paste_keeps_content_without_markers() {
        assert_eq!(typed(b"\x1b[200~one\ntwo\x1b[201~"), open("one\ntwo"));
    }

    #[test]
    fn osc_reply_is_dropped() {
        assert_eq!(typed(b"\x1b]11;rgb:0000/0000/0000\x07"), Typed::Ignore);
        assert_eq!(typed(b"a\x1b]0;t\x1b\\b"), open("ab"));
    }

    #[test]
    fn keys_without_text_are_ignored() {
        assert_eq!(typed(b"\x1b[A"), Typed::Ignore);
        assert_eq!(typed(b"\n"), Typed::Ignore);
        assert_eq!(typed(b"\x04"), Typed::Ignore);
        assert_eq!(typed(b"\x0c"), Typed::Ignore);
        assert_eq!(typed(b"\x1b"), Typed::Ignore);
        assert_eq!(typed(b"\xc3"), Typed::Ignore);
    }

    #[test]
    fn tab_and_inner_newline_are_kept() {
        assert_eq!(typed(b"a\tb\nc\n"), open("a\tb\nc"));
    }

    #[test]
    fn split_utf8_is_incomplete_and_joins_whole() {
        assert!(incomplete(b"a\xc3"));
        assert!(!incomplete(b"a\xc3\xa9"));
        assert!(!incomplete(b"a\xff"));
        assert_eq!(typed(b"a\xc3\xa9"), open("a\u{e9}"));
    }

    #[test]
    fn cut_escape_is_incomplete() {
        assert!(incomplete(b"a\x1b"));
        assert!(incomplete(b"\x1b["));
        assert!(incomplete(b"\x1b[1;5"));
        assert!(incomplete(b"\x1bO"));
        assert!(incomplete(b"\x1b]11;rgb"));
        assert!(!incomplete(b"\x1b[A"));
        assert!(!incomplete(b"\x1bOP"));
        assert!(!incomplete(b"\x1b]0;t\x07"));
        assert!(!incomplete(b"abc"));
    }

    #[test]
    fn ctrl_c_in_queue_editor_clears_a_draft_else_cancels() {
        assert_eq!(queue_ctrl_c("x"), QueueCtrlC::ClearDraft);
        assert_eq!(queue_ctrl_c(""), QueueCtrlC::CancelTurn);
    }
}
