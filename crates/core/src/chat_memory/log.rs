use super::{Entry, Node, Record, Tree};

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

pub fn parse_log(data: &[u8]) -> (Vec<Entry>, Vec<String>) {
    let mut entries = Vec::new();
    for line in complete_lines(data) {
        match serde_json::from_slice::<Entry>(line) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                return (
                    entries.clone(),
                    vec![format!(
                        "chat log entry {} is corrupt; later entries ignored: {e}",
                        entries.len()
                    )],
                )
            }
        }
    }
    (entries, Vec::new())
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

pub fn encode_entry(entry: &Entry) -> Result<String, String> {
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
    use crate::chat_memory::fixtures::{entry, n};

    #[test]
    fn parse_ignores_partial_and_stops_at_corrupt() {
        let a = encode_entry(&entry(0, 1)).unwrap();
        let b = encode_entry(&entry(1, 1)).unwrap();
        let partial = format!("{a}{}", &b[..10]);
        let (entries, warnings) = parse_log(partial.as_bytes());
        assert_eq!((entries.len(), warnings.len()), (1, 0));
        let corrupt = format!("{a}garbage\n{b}");
        let (entries, warnings) = parse_log(corrupt.as_bytes());
        assert_eq!((entries.len(), warnings.len()), (1, 1));
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
