use super::index::scan_lines;
use super::{Entry, NewEntry, Node, Record, Tree};

fn complete_lines(data: &[u8]) -> Vec<&[u8]> {
    let end = data.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    data[..end]
        .split(|b| *b == b'\n')
        .filter(|line| !line.iter().all(u8::is_ascii_whitespace))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogTail {
    Whole,
    Torn(u64),
    NeedMore,
}

pub fn log_tail(chunk: &[u8], chunk_start: u64) -> LogTail {
    match chunk.iter().rposition(|b| *b == b'\n') {
        _ if chunk.last().is_none_or(|b| *b == b'\n') => LogTail::Whole,
        Some(i) => LogTail::Torn(chunk_start + i as u64 + 1),
        None if chunk_start == 0 => LogTail::Torn(0),
        None => LogTail::NeedMore,
    }
}

pub const CORRUPT_TEXT: &str = "[corrupt entry]";

pub fn entry_or_corrupt(bytes: &[u8], id: usize, offset: u64) -> (Entry, Option<String>) {
    match serde_json::from_slice::<Entry>(bytes) {
        Ok(entry) => (entry, None),
        Err(e) => (
            Entry::corrupt(),
            Some(format!(
                "chat log entry {id} at byte {offset} is corrupt and shows as {CORRUPT_TEXT}: {e}"
            )),
        ),
    }
}

pub fn parse_log(data: &[u8]) -> (Vec<Entry>, Vec<String>) {
    let mut entries = Vec::new();
    let mut warnings = Vec::new();
    for (id, line) in scan_lines(data, 0).into_iter().enumerate() {
        let bytes = &data[line.start as usize..line.end as usize];
        let (entry, warning) = entry_or_corrupt(bytes, id, line.start);
        entries.push(entry);
        warnings.extend(warning);
    }
    (entries, warnings)
}

pub fn parse_tree(data: &[u8]) -> (Tree, Vec<String>) {
    let mut tree = Tree::new();
    let mut warnings = Vec::new();
    for line in complete_lines(data) {
        let parsed = serde_json::from_slice::<Record>(line)
            .map_err(|e| e.to_string())
            .and_then(|r| Node::new(r.lo, r.hi).map(|n| (n, r.text)));
        match parsed {
            Ok((node, text)) => {
                tree.entry(node).or_insert(text);
            }
            Err(e) => warnings.push(format!("chat tree record skipped: {e}")),
        }
    }
    (tree, warnings)
}

pub fn encode_entry(entry: &NewEntry) -> Result<String, String> {
    let mut line = serde_json::to_string(entry).map_err(|e| e.to_string())?;
    line.push('\n');
    Ok(line)
}

pub fn encode_record(node: Node, text: &str) -> Result<String, String> {
    let record = Record {
        lo: node.lo,
        hi: node.hi,
        text: text.to_string(),
    };
    let mut line = serde_json::to_string(&record).map_err(|e| e.to_string())?;
    line.push('\n');
    Ok(line)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_memory::fixtures::{entry, n, written};
    use crate::chat_memory::Role;

    #[test]
    fn parse_ignores_a_partial_last_line() {
        let a = written(&entry(0, 1));
        let b = written(&entry(1, 1));
        let partial = format!("{a}{}", &b[..10]);
        let (entries, warnings) = parse_log(partial.as_bytes());
        assert_eq!((entries.len(), warnings.len()), (1, 0));
    }

    #[test]
    fn parse_keeps_every_entry_after_a_bad_line_with_stable_ids() {
        let a = written(&entry(0, 1));
        let b = written(&entry(1, 1));
        let c = written(&entry(2, 1));
        let log = format!("{a}garbage\n\n{b}also bad\n{c}");
        let (entries, warnings) = parse_log(log.as_bytes());
        assert_eq!(entries.len(), 5);
        let bad: Vec<bool> = entries.iter().map(Entry::is_corrupt).collect();
        assert_eq!(bad, [false, true, false, true, false]);
        assert_eq!(entries[2], entry(1, 1));
        assert_eq!(entries[4], entry(2, 1));
        let first = a.len();
        assert_eq!(warnings.len(), 2);
        assert!(
            warnings[0].starts_with(&format!("chat log entry 1 at byte {first} is corrupt")),
            "{}",
            warnings[0]
        );
        let second = first + "garbage\n\n".len() + b.len();
        assert!(
            warnings[1].starts_with(&format!("chat log entry 3 at byte {second} is corrupt")),
            "{}",
            warnings[1]
        );
    }

    #[test]
    fn entry_or_corrupt_keeps_good_lines_and_marks_bad_ones() {
        let good = entry(0, 1);
        assert!(!good.is_corrupt());
        let line = written(&good);
        assert_eq!(
            entry_or_corrupt(line.trim_end().as_bytes(), 4, 9),
            (good, None)
        );
        let (placeholder, warning) = entry_or_corrupt(b"garbage", 4, 9);
        assert!(placeholder.is_corrupt());
        assert_eq!(placeholder, Entry::corrupt());
        let warning = warning.unwrap();
        assert!(
            warning.starts_with("chat log entry 4 at byte 9 is corrupt"),
            "{warning}"
        );
    }

    #[test]
    fn a_real_entry_with_the_placeholder_text_stays_normal() {
        let real = Entry {
            role: Role::Worker,
            at: String::new(),
            text: CORRUPT_TEXT.to_string(),
            corrupt: false,
        };
        assert!(!real.is_corrupt());
        let line = written(&real);
        let (parsed, warning) = entry_or_corrupt(line.trim_end().as_bytes(), 0, 0);
        assert!(!parsed.is_corrupt());
        assert_eq!((parsed, warning), (real, None));
    }

    #[test]
    fn the_flag_is_never_written_or_read_from_the_log() {
        let forged = r#"{"role":"worker","at":"","text":"x","corrupt":true}"#;
        let (entry, warning) = entry_or_corrupt(forged.as_bytes(), 0, 0);
        assert_eq!((entry.is_corrupt(), warning), (false, None));
    }

    #[test]
    fn log_tail_finds_where_a_torn_line_starts() {
        assert_eq!(log_tail(b"", 0), LogTail::Whole);
        assert_eq!(log_tail(b"a\nb\n", 7), LogTail::Whole);
        assert_eq!(log_tail(b"a\nbc", 7), LogTail::Torn(9));
        assert_eq!(log_tail(b"abc", 0), LogTail::Torn(0));
        assert_eq!(log_tail(b"abc", 5), LogTail::NeedMore);
    }

    #[test]
    fn parse_tree_skips_bad_and_keeps_first() {
        let data = format!(
            "{}{}bad\n{}{{\"lo\":1,\"hi\":3,\"text\":\"x\"}}\n{{\"lo\":0,\"hi\":2",
            encode_record(n(0, 2), "first").unwrap(),
            encode_record(n(0, 2), "second").unwrap(),
            encode_record(n(2, 4), "other").unwrap(),
        );
        let (tree, warnings) = parse_tree(data.as_bytes());
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[&n(0, 2)], "first");
        assert_eq!(warnings.len(), 2);
    }
}
