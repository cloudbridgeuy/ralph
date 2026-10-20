use super::{Entry, Node, Tree, Window, SUMMARY_BYTES, UNAVAILABLE_MARK};
use crate::mcp::string_arg;
use serde_json::Value;
use std::ops::RangeInclusive;

pub const DEFAULT_BUDGET: usize = 4096;
pub const BUDGET_RANGE: RangeInclusive<usize> = 1..=16384;
pub(super) const BYTES_PER_TOKEN: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Raw,
    Summary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub next: usize,
    pub offset: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeRequest {
    pub from: usize,
    pub to: usize,
    pub mode: Mode,
    pub budget: usize,
    pub cursor: Option<Cursor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangePage {
    pub text: String,
    pub next: Option<Cursor>,
}

/// `More`: the page needs an entry past the window; retry with a larger one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Read {
    Page(RangePage),
    More,
}

impl Cursor {
    pub fn parse(s: &str) -> Result<Cursor, String> {
        let bad =
            || format!("'{s}' is not a cursor. Copy the `next cursor:` value from the last page.");
        let number = |p: &str| {
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return Err(bad());
            }
            p.parse::<usize>().map_err(|_| bad())
        };
        let (position, end) = s.split_once('@').ok_or_else(bad)?;
        let (next, offset) = position.split_once('.').ok_or_else(bad)?;
        Ok(Cursor {
            next: number(next)?,
            offset: number(offset)?,
            end: number(end)?,
        })
    }

    pub fn encode(&self) -> String {
        format!("{}.{}@{}", self.next, self.offset, self.end)
    }
}

impl RangePage {
    pub fn render(&self) -> String {
        match &self.next {
            Some(cursor) => format!("{}\n\nnext cursor: {}", self.text, cursor.encode()),
            None => format!("{}\n\n(end of range)", self.text),
        }
    }
}

pub fn parse_range(s: &str) -> Result<(usize, usize), String> {
    let bad = || format!("'{s}' is not a range. Use an index like 5 or a range like 16-40.");
    let number = |p: &str| {
        if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
            return Err(bad());
        }
        p.parse::<usize>().map_err(|_| bad())
    };
    match s.split_once('-') {
        None => number(s).map(|n| (n, n)),
        Some((a, b)) => Ok((number(a)?, number(b)?)),
    }
}

pub fn parse_read_args(args: &Value) -> Result<RangeRequest, String> {
    let (from, to) = parse_range(string_arg(args, "id")?.trim())?;
    let mode = match args.get("mode").and_then(Value::as_str) {
        None | Some("raw") => Mode::Raw,
        Some("summary") => Mode::Summary,
        Some(other) => return Err(format!("'{other}' is not a mode. Use raw or summary.")),
    };
    let budget = match args.get("budget") {
        None | Some(Value::Null) => DEFAULT_BUDGET,
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or("budget must be a whole number of tokens.")?,
    };
    let cursor = match args.get("cursor").and_then(Value::as_str) {
        None => None,
        Some(s) => Some(Cursor::parse(s.trim())?),
    };
    Ok(RangeRequest {
        from,
        to,
        mode,
        budget,
        cursor,
    })
}

fn view_text(entry: &Entry, tree: &Tree, index: usize, mode: Mode) -> String {
    if mode == Mode::Raw || entry.text.len() <= SUMMARY_BYTES {
        return entry.text.clone();
    }
    tree.get(&Node::leaf(index))
        .cloned()
        .unwrap_or_else(|| UNAVAILABLE_MARK.trim_end().to_string())
}

struct Slice<'a> {
    index: usize,
    entry: &'a Entry,
    total: usize,
    offset: usize,
}

impl Slice<'_> {
    fn header(&self, count: usize) -> String {
        let base = format!(
            "#{} · {} · {} · {} chars",
            self.index,
            self.entry.role.as_str(),
            self.entry.at,
            self.total
        );
        if self.offset == 0 && count == self.total {
            base
        } else {
            format!("{base} · chars {}-{}", self.offset + 1, self.offset + count)
        }
    }

    fn block(&self, text: &str, count: usize) -> String {
        let part: String = text.chars().skip(self.offset).take(count).collect();
        format!("{}\n{part}", self.header(count))
    }

    fn fitting_chars(&self, text: &str, room: usize) -> usize {
        let mut used = 0;
        text.chars()
            .skip(self.offset)
            .take_while(|c| {
                used += c.len_utf8();
                used <= room
            })
            .count()
    }
}

fn validate_cursor(cursor: &Cursor, from: usize, limit: usize, len: usize) -> Result<(), String> {
    if cursor.end > len || cursor.end > limit {
        return Err(format!(
            "cursor {} is stale: its end is past the requested range or the chat.",
            cursor.encode()
        ));
    }
    if cursor.next < from || cursor.next >= cursor.end {
        return Err(format!(
            "cursor {} is outside the range {from}-{}.",
            cursor.encode(),
            limit - 1
        ));
    }
    Ok(())
}

pub fn read_range(window: &Window, tree: &Tree, request: &RangeRequest) -> Result<Read, String> {
    let len = window.total;
    if request.from > request.to {
        return Err(format!(
            "range {}-{} is backwards.",
            request.from, request.to
        ));
    }
    if request.from >= len {
        return Err(format!(
            "#{} is beyond the chat: it holds {len} entries.",
            request.from
        ));
    }
    if !BUDGET_RANGE.contains(&request.budget) {
        return Err(format!(
            "budget {} is out of range. Use {} to {} tokens.",
            request.budget,
            BUDGET_RANGE.start(),
            BUDGET_RANGE.end()
        ));
    }
    let limit = request.to.saturating_add(1).min(len);
    let mut cursor = match request.cursor {
        None => Cursor {
            next: request.from,
            offset: 0,
            end: limit,
        },
        Some(c) => {
            validate_cursor(&c, request.from, limit, len)?;
            c
        }
    };
    let mut left = request.budget * BYTES_PER_TOKEN;
    let mut blocks: Vec<String> = Vec::new();
    while cursor.next < cursor.end {
        let Some(entry) = window.get(cursor.next) else {
            return Ok(Read::More);
        };
        let text = view_text(entry, tree, cursor.next, request.mode);
        let total = text.chars().count();
        if cursor.offset > 0 && cursor.offset >= total {
            return Err(format!(
                "cursor {} is stale: #{} has only {total} chars in this mode.",
                cursor.encode(),
                cursor.next
            ));
        }
        let slice = Slice {
            index: cursor.next,
            entry,
            total,
            offset: cursor.offset,
        };
        let remaining = total - cursor.offset;
        let whole = slice.block(&text, remaining);
        if whole.len() <= left || (blocks.is_empty() && remaining == 0) {
            left = left.saturating_sub(whole.len() + 2);
            blocks.push(whole);
            cursor = Cursor {
                next: cursor.next + 1,
                offset: 0,
                ..cursor
            };
            continue;
        }
        let overhead = slice.header(remaining.saturating_sub(1)).len() + 1;
        let mut count = slice.fitting_chars(&text, left.saturating_sub(overhead));
        if count == 0 {
            if !blocks.is_empty() {
                break;
            }
            count = 1;
        }
        blocks.push(slice.block(&text, count));
        cursor = if count == remaining {
            Cursor {
                next: cursor.next + 1,
                offset: 0,
                ..cursor
            }
        } else {
            Cursor {
                offset: cursor.offset + count,
                ..cursor
            }
        };
        break;
    }
    Ok(Read::Page(RangePage {
        text: blocks.join("\n\n"),
        next: (cursor.next < cursor.end).then_some(cursor),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_memory::fixtures::entry;
    use crate::chat_memory::Role;
    use serde_json::json;

    fn text_entry(text: &str) -> Entry {
        Entry {
            role: Role::User,
            at: "2026-01-01T00:00:00Z".to_string(),
            text: text.to_string(),
            corrupt: false,
        }
    }

    fn request(from: usize, to: usize, budget: usize) -> RangeRequest {
        RangeRequest {
            from,
            to,
            mode: Mode::Raw,
            budget,
            cursor: None,
        }
    }

    fn read_full(
        entries: &[Entry],
        tree: &Tree,
        request: &RangeRequest,
    ) -> Result<RangePage, String> {
        match read_range(&Window::full(entries), tree, request)? {
            Read::Page(page) => Ok(page),
            Read::More => panic!("a full window never needs more"),
        }
    }

    fn pages(entries: &[Entry], tree: &Tree, first: RangeRequest) -> Vec<RangePage> {
        let mut out = Vec::new();
        let mut req = first;
        loop {
            let page = read_full(entries, tree, &req).unwrap();
            let next = page.next;
            out.push(page);
            match next {
                Some(c) => req.cursor = Some(c),
                None => return out,
            }
            assert!(out.len() < 10_000);
        }
    }

    fn bodies(page: &RangePage) -> Vec<(String, String)> {
        page.text
            .split("\n\n#")
            .map(|b| {
                let b = b.strip_prefix('#').unwrap_or(b);
                let (h, t) = b.split_once('\n').unwrap();
                (format!("#{h}"), t.to_string())
            })
            .collect()
    }

    #[test]
    fn parses_ranges_and_cursors() {
        assert_eq!(parse_range("5"), Ok((5, 5)));
        assert_eq!(parse_range("16-40"), Ok((16, 40)));
        assert!(parse_range("a").is_err());
        assert!(parse_range("1-").is_err());
        assert!(parse_range("-1").is_err());
        let c = Cursor {
            next: 3,
            offset: 7,
            end: 9,
        };
        assert_eq!(Cursor::parse(&c.encode()), Ok(c));
        for bad in ["", "3", "3.7", "3@9", "a.1@2", "3.-1@9", "3.1@"] {
            assert!(Cursor::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn parses_tool_args_with_defaults() {
        let r = parse_read_args(&json!({"id": "2-4"})).unwrap();
        assert_eq!(r, request(2, 4, DEFAULT_BUDGET));
        let r = parse_read_args(
            &json!({"id": "3", "mode": "summary", "budget": 10, "cursor": "3.2@4"}),
        )
        .unwrap();
        assert_eq!(r.mode, Mode::Summary);
        assert_eq!(r.budget, 10);
        assert_eq!(
            r.cursor,
            Some(Cursor {
                next: 3,
                offset: 2,
                end: 4
            })
        );
        for bad in [
            json!({}),
            json!({"id": "x"}),
            json!({"id": "1", "mode": "full"}),
            json!({"id": "1", "budget": "big"}),
            json!({"id": "1", "budget": -1}),
            json!({"id": "1", "cursor": "zz"}),
        ] {
            assert!(parse_read_args(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn reads_whole_range_in_one_page() {
        let entries: Vec<Entry> = (0..4).map(|i| entry(i, 5)).collect();
        let page = read_full(&entries, &Tree::new(), &request(1, 2, 4096)).unwrap();
        assert_eq!(page.next, None);
        assert_eq!(
            page.text,
            "#1 · assistant · 2026-01-01T00:00:01Z · 8 chars\nm1 xxxxx\n\n#2 · user · 2026-01-01T00:00:02Z · 8 chars\nm2 xxxxx"
        );
        assert!(page.render().ends_with("(end of range)"));
    }

    #[test]
    fn clamps_to_to_log_length() {
        let entries: Vec<Entry> = (0..3).map(|i| entry(i, 5)).collect();
        let page = read_full(&entries, &Tree::new(), &request(1, 99, 4096)).unwrap();
        assert_eq!(bodies(&page).len(), 2);
        assert_eq!(page.next, None);
    }

    #[test]
    fn rejects_bad_requests() {
        let entries: Vec<Entry> = (0..3).map(|i| entry(i, 5)).collect();
        let t = Tree::new();
        assert!(read_full(&entries, &t, &request(2, 1, 100)).is_err());
        assert!(read_full(&entries, &t, &request(3, 4, 100)).is_err());
        assert!(read_full(&entries, &t, &request(0, 2, 0)).is_err());
        assert!(read_full(&entries, &t, &request(0, 2, 16385)).is_err());
        assert!(read_full(&entries, &t, &request(0, 2, 16384)).is_ok());
    }

    #[test]
    fn rejects_stale_cursors() {
        let entries: Vec<Entry> = (0..3).map(|i| entry(i, 5)).collect();
        let t = Tree::new();
        let with = |next, offset, end| RangeRequest {
            cursor: Some(Cursor { next, offset, end }),
            ..request(1, 2, 100)
        };
        assert!(read_full(&entries, &t, &with(1, 0, 3)).is_ok());
        assert!(read_full(&entries, &t, &with(0, 0, 3)).is_err());
        assert!(read_full(&entries, &t, &with(3, 0, 3)).is_err());
        assert!(read_full(&entries, &t, &with(1, 0, 4)).is_err());
        assert!(read_full(&entries, &t, &with(1, 8, 3)).is_err());
        let narrow = RangeRequest {
            cursor: Some(Cursor {
                next: 1,
                offset: 0,
                end: 3,
            }),
            ..request(1, 1, 100)
        };
        assert!(read_full(&entries, &t, &narrow).is_err());
    }

    #[test]
    fn full_traversal_has_no_gaps_or_duplicates() {
        let entries: Vec<Entry> = (0..12).map(|i| entry(i, 20 + i)).collect();
        for budget in [1, 5, 9, 40] {
            let all = pages(&entries, &Tree::new(), request(2, 9, budget));
            let mut joined: Vec<(usize, String)> = Vec::new();
            for (header, text) in all.iter().flat_map(bodies) {
                let id: usize = header[1..].split(' ').next().unwrap().parse().unwrap();
                match joined.last_mut() {
                    Some((last, acc)) if *last == id => acc.push_str(&text),
                    _ => joined.push((id, text)),
                }
            }
            let ids: Vec<usize> = joined.iter().map(|(i, _)| *i).collect();
            assert_eq!(ids, (2..=9).collect::<Vec<_>>(), "budget {budget}");
            for (i, text) in joined {
                assert_eq!(text, entries[i].text, "budget {budget}");
            }
        }
    }

    #[test]
    fn oversized_record_spans_pages_and_always_advances() {
        let entries = vec![text_entry(&"a".repeat(300))];
        let all = pages(&entries, &Tree::new(), request(0, 0, 20));
        assert!(all.len() > 2);
        for p in &all {
            assert!(!bodies(p)[0].1.is_empty());
        }
        assert!(all[0].text.contains("chars 1-"));
        let joined: String = all.iter().flat_map(bodies).map(|(_, t)| t).collect();
        assert_eq!(joined, entries[0].text);
    }

    #[test]
    fn splits_on_char_boundaries() {
        let text = "é世🎉".repeat(40);
        let entries = vec![text_entry(&text)];
        let all = pages(&entries, &Tree::new(), request(0, 0, 8));
        assert!(all.len() > 1);
        let joined: String = all.iter().flat_map(bodies).map(|(_, t)| t).collect();
        assert_eq!(joined, text);
    }

    #[test]
    fn multiline_text_round_trips_exactly() {
        let text = "  line one\n\n\tline  three \n\n\n#9 fake header\n  ".repeat(10);
        let entries = vec![text_entry(&text)];
        for budget in [3, 15, 4096] {
            let all = pages(&entries, &Tree::new(), request(0, 0, budget));
            let rebuilt: String = all
                .iter()
                .filter_map(|p| p.text.split_once('\n'))
                .map(|(_, body)| body)
                .collect();
            assert_eq!(rebuilt, text, "budget {budget}");
        }
    }

    #[test]
    fn appended_messages_stay_out_of_a_snapshot() {
        let mut entries: Vec<Entry> = (0..4).map(|i| entry(i, 30)).collect();
        let first = read_full(&entries, &Tree::new(), &request(0, 99, 15)).unwrap();
        let cursor = first.next.unwrap();
        assert_eq!(cursor.end, 4);
        entries.push(entry(4, 30));
        entries.push(entry(5, 30));
        let mut req = RangeRequest {
            cursor: Some(cursor),
            ..request(0, 99, 15)
        };
        let mut text = first.text;
        loop {
            let page = read_full(&entries, &Tree::new(), &req).unwrap();
            text.push_str(&page.text);
            match page.next {
                Some(c) => req.cursor = Some(c),
                None => break,
            }
        }
        assert!(text.contains("#3 ") && !text.contains("#4 ") && !text.contains("#5 "));
    }

    #[test]
    fn summary_mode_uses_leaf_summaries() {
        let long = "z".repeat(SUMMARY_BYTES + 1);
        let entries = vec![
            text_entry(&long),
            text_entry(&long),
            text_entry("short one"),
        ];
        let mut tree = Tree::new();
        tree.insert(Node::leaf(0), "the gist".to_string());
        let req = RangeRequest {
            mode: Mode::Summary,
            ..request(0, 2, 4096)
        };
        let page = read_full(&entries, &tree, &req).unwrap();
        let parts = bodies(&page);
        assert_eq!(parts[0].1, "the gist");
        assert_eq!(parts[1].1, "[not summarized]");
        assert_eq!(parts[2].1, "short one");
        let raw = read_full(&entries, &tree, &request(0, 0, 4096)).unwrap();
        assert_eq!(bodies(&raw)[0].1, long);
    }
}
