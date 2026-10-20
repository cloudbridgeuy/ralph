use super::{Entry, Node, Record};
use std::collections::HashSet;

pub const LOG_INDEX_FILE: &str = "log.idx";
pub const TREE_INDEX_FILE: &str = "tree.idx";

const VERSION: u32 = 1;
const HEADER_BYTES: usize = 48;
const STOPPED: u32 = 1;

/// Byte span of one complete, non-blank line, newline excluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line {
    pub start: u64,
    pub end: u64,
}

/// What one complete line adds to an index.
pub enum Scan<S> {
    Add(S),
    Skip,
    Stop,
}

/// One fixed-size record of an index over a JSONL file.
pub trait Slot: Copy {
    const FILE: &'static str;
    const INDEX_FILE: &'static str;
    const MAGIC: [u8; 8];
    const BYTES: usize;
    fn scan(bytes: &[u8], line: Line) -> Scan<Self>;
    fn start(&self) -> u64;
    fn write(&self, out: &mut Vec<u8>);
    fn read(bytes: &[u8]) -> Option<Self>;
}

/// The start of a `log.jsonl` entry. A line that is not an entry stops the
/// index, as it stops `parse_log`.
impl Slot for u64 {
    const FILE: &'static str = "log.jsonl";
    const INDEX_FILE: &'static str = LOG_INDEX_FILE;
    const MAGIC: [u8; 8] = *b"RLPHLOGX";
    const BYTES: usize = 8;

    fn scan(bytes: &[u8], line: Line) -> Scan<u64> {
        match serde_json::from_slice::<Entry>(bytes) {
            Ok(_) => Scan::Add(line.start),
            Err(_) => Scan::Stop,
        }
    }

    fn start(&self) -> u64 {
        *self
    }

    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }

    fn read(bytes: &[u8]) -> Option<u64> {
        Some(read_u64(bytes))
    }
}

/// A `tree.jsonl` record of `node` on bytes `[start, end)`. A line that is not
/// a record is skipped, as `parse_tree` skips it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeSlot {
    pub node: Node,
    pub start: u64,
    pub end: u64,
}

impl Slot for NodeSlot {
    const FILE: &'static str = "tree.jsonl";
    const INDEX_FILE: &'static str = TREE_INDEX_FILE;
    const MAGIC: [u8; 8] = *b"RLPHTREX";
    const BYTES: usize = 32;

    fn scan(bytes: &[u8], line: Line) -> Scan<NodeSlot> {
        let node = serde_json::from_slice::<Record>(bytes)
            .ok()
            .and_then(|r| Node::new(r.lo, r.hi).ok());
        match node {
            Some(node) => Scan::Add(NodeSlot {
                node,
                start: line.start,
                end: line.end,
            }),
            None => Scan::Skip,
        }
    }

    fn start(&self) -> u64 {
        self.start
    }

    fn write(&self, out: &mut Vec<u8>) {
        for word in [
            self.node.lo as u64,
            self.node.hi as u64,
            self.start,
            self.end,
        ] {
            out.extend_from_slice(&word.to_le_bytes());
        }
    }

    fn read(bytes: &[u8]) -> Option<NodeSlot> {
        let word = |k: usize| read_u64(&bytes[8 * k..]);
        let lo = usize::try_from(word(0)).ok()?;
        let hi = usize::try_from(word(1)).ok()?;
        let (start, end) = (word(2), word(3));
        (start < end).then_some(())?;
        Some(NodeSlot {
            node: Node::new(lo, hi).ok()?,
            start,
            end,
        })
    }
}

/// Slots for the lines of a JSONL file in `[0, covered)`. `covered` ends on
/// the newline of the last scanned line, which starts at `tail`; `hash` is the
/// FNV-1a hash of the bytes `[tail, covered)`, so a rewritten file shows as
/// stale. `stopped` means a line that stops the scan follows `covered`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index<S> {
    pub slots: Vec<S>,
    pub covered: u64,
    pub tail: u64,
    pub hash: u64,
    pub stopped: bool,
}

pub type LogIndex = Index<u64>;
pub type TreeIndex = Index<NodeSlot>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Fresh,
    Extend,
    Rebuild,
}

pub fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, b| {
        (hash ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Complete, non-blank lines of `data`, which starts at file offset `base`.
/// A trailing line without a newline is left out, as in `parse_log`.
pub fn scan_lines(data: &[u8], base: u64) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (i, b) in data.iter().enumerate() {
        if *b != b'\n' {
            continue;
        }
        if !data[start..i].iter().all(u8::is_ascii_whitespace) {
            lines.push(Line {
                start: base + start as u64,
                end: base + i as u64,
            });
        }
        start = i + 1;
    }
    lines
}

impl<S> Default for Index<S> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            covered: 0,
            tail: 0,
            hash: fnv1a(&[]),
            stopped: false,
        }
    }
}

impl<S: Slot> Index<S> {
    pub fn count(&self) -> usize {
        self.slots.len()
    }

    /// Scans `data`, the file bytes from `covered` on. A line that stops the
    /// scan stops the index for good.
    pub fn extend(&mut self, data: &[u8]) {
        if self.stopped {
            return;
        }
        let base = self.covered;
        let mut last = None;
        for line in scan_lines(data, base) {
            let bytes = &data[(line.start - base) as usize..(line.end - base) as usize];
            match S::scan(bytes, line) {
                Scan::Add(slot) => self.slots.push(slot),
                Scan::Skip => {}
                Scan::Stop => {
                    self.stopped = true;
                    break;
                }
            }
            last = Some(line);
        }
        if let Some(line) = last {
            self.tail = line.start;
            self.covered = line.end + 1;
            self.hash = fnv1a(&data[(line.start - base) as usize..(self.covered - base) as usize]);
        }
    }

    /// Compares the index with a file of `len` bytes whose bytes
    /// `[tail, covered)` are `tail` (shorter when the file is shorter).
    pub fn freshness(&self, len: u64, tail: &[u8]) -> Freshness {
        if len < self.covered || fnv1a(tail) != self.hash {
            Freshness::Rebuild
        } else if self.stopped || len == self.covered {
            Freshness::Fresh
        } else {
            Freshness::Extend
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.encode_header();
        out.extend_from_slice(&self.encode_slots(0));
        out
    }

    /// The header alone, for an in-place extend: slots first, then this.
    pub fn encode_header(&self) -> Vec<u8> {
        let flags = if self.stopped { STOPPED } else { 0 };
        let mut out = Vec::with_capacity(HEADER_BYTES);
        out.extend_from_slice(&S::MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        for word in [self.covered, self.tail, self.hash, self.count() as u64] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out
    }

    /// The slots from `from` on.
    pub fn encode_slots(&self, from: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.count().saturating_sub(from) * S::BYTES);
        for slot in self.slots.iter().skip(from) {
            slot.write(&mut out);
        }
        out
    }

    /// File offset of slot `count`.
    pub fn slot_offset(count: usize) -> u64 {
        (HEADER_BYTES + count * S::BYTES) as u64
    }

    /// `None` for a missing, foreign, torn or inconsistent file; the caller
    /// then rebuilds from the JSONL file.
    pub fn decode(bytes: &[u8]) -> Option<Index<S>> {
        if bytes.len() < HEADER_BYTES || bytes[..8] != S::MAGIC {
            return None;
        }
        if read_u32(&bytes[8..]) != VERSION {
            return None;
        }
        let stopped = read_u32(&bytes[12..]) & STOPPED != 0;
        let word = |k: usize| read_u64(&bytes[16 + 8 * k..]);
        let (covered, tail, hash) = (word(0), word(1), word(2));
        let count = usize::try_from(word(3)).ok()?;
        let body = &bytes[HEADER_BYTES..];
        if body.len() != count.checked_mul(S::BYTES)? {
            return None;
        }
        let slots = body
            .chunks_exact(S::BYTES)
            .map(S::read)
            .collect::<Option<Vec<S>>>()?;
        let ordered = slots.windows(2).all(|w| w[0].start() < w[1].start());
        let inside = slots.last().is_none_or(|last| last.start() <= tail);
        let tail_ok = tail < covered || (tail, covered) == (0, 0);
        (ordered && inside && tail_ok).then_some(Index {
            slots,
            covered,
            tail,
            hash,
            stopped,
        })
    }
}

impl LogIndex {
    /// Byte span of entries `[from, to)`; `None` when the range is empty or
    /// past the index.
    pub fn span(&self, from: usize, to: usize) -> Option<(u64, u64)> {
        if from >= to || to > self.count() {
            return None;
        }
        let end = self.slots.get(to).copied().unwrap_or(self.covered);
        Some((self.slots[from], end))
    }
}

impl TreeIndex {
    /// The first record of each wanted node, as `parse_tree` keeps it.
    pub fn find(&self, wanted: &[Node]) -> Vec<NodeSlot> {
        let mut left: HashSet<Node> = wanted.iter().copied().collect();
        let mut found = Vec::new();
        for slot in &self.slots {
            if left.is_empty() {
                break;
            }
            if left.remove(&slot.node) {
                found.push(*slot);
            }
        }
        found
    }
}

fn read_u32(bytes: &[u8]) -> u32 {
    let mut word = [0; 4];
    word.copy_from_slice(&bytes[..4]);
    u32::from_le_bytes(word)
}

fn read_u64(bytes: &[u8]) -> u64 {
    let mut word = [0; 8];
    word.copy_from_slice(&bytes[..8]);
    u64::from_le_bytes(word)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_memory::fixtures::{entry, n};
    use crate::chat_memory::{encode_entry, encode_record, parse_tree};

    fn log(count: usize) -> Vec<u8> {
        (0..count)
            .map(|i| encode_entry(&entry(i, 3 * i)).unwrap())
            .collect::<String>()
            .into_bytes()
    }

    fn built(data: &[u8]) -> LogIndex {
        let mut index = LogIndex::default();
        index.extend(data);
        index
    }

    fn tail<'a, S>(index: &Index<S>, data: &'a [u8]) -> &'a [u8] {
        let end = (index.covered as usize).min(data.len());
        &data[(index.tail as usize).min(end)..end]
    }

    #[test]
    fn scan_lines_skips_blank_lines_and_a_torn_tail() {
        let lines = scan_lines(b"ab\n\n  \ncd\nef", 10);
        assert_eq!(
            lines,
            [Line { start: 10, end: 12 }, Line { start: 17, end: 19 }]
        );
        assert!(scan_lines(b"no newline", 0).is_empty());
    }

    #[test]
    fn extend_indexes_complete_entries_only() {
        let mut data = log(3);
        let complete = data.len() as u64;
        data.extend_from_slice(b"{\"role\":\"user\"");
        let index = built(&data);
        assert_eq!(index.count(), 3);
        assert_eq!(index.covered, complete);
        assert!(!index.stopped);
        let (start, end) = index.span(1, 2).unwrap();
        let text = std::str::from_utf8(&data[start as usize..end as usize]).unwrap();
        assert_eq!(text, encode_entry(&entry(1, 3)).unwrap());
    }

    #[test]
    fn extend_stops_at_a_corrupt_line_like_parse_log() {
        let mut data = log(2);
        let good = data.len() as u64;
        data.extend_from_slice(b"not json\n");
        data.extend_from_slice(&log(1));
        let index = built(&data);
        assert_eq!(
            (index.count(), index.covered, index.stopped),
            (2, good, true)
        );
        let mut more = index.clone();
        more.extend(&log(1));
        assert_eq!(more, index);
    }

    #[test]
    fn extend_in_steps_matches_one_build() {
        let data = log(6);
        let whole = built(&data);
        let mut stepped = built(&data[..whole.slots[3] as usize]);
        stepped.extend(&data[stepped.covered as usize..]);
        assert_eq!(stepped, whole);
    }

    #[test]
    fn encode_decode_round_trips() {
        let mut data = log(4);
        data.extend_from_slice(b"oops\n");
        for index in [LogIndex::default(), built(&log(4)), built(&data)] {
            assert_eq!(LogIndex::decode(&index.encode()), Some(index.clone()));
            let header = index.encode_header();
            assert_eq!(header, index.encode()[..header.len()]);
        }
    }

    #[test]
    fn decode_rejects_bad_files() {
        let good = built(&log(3)).encode();
        assert!(LogIndex::decode(&[]).is_none());
        assert!(LogIndex::decode(&good[..good.len() - 1]).is_none());
        let mut magic = good.clone();
        magic[0] = b'X';
        assert!(LogIndex::decode(&magic).is_none());
        let mut version = good.clone();
        version[8] = 9;
        assert!(LogIndex::decode(&version).is_none());
        let mut unordered = good.clone();
        unordered[HEADER_BYTES + 8..HEADER_BYTES + 16].copy_from_slice(&0u64.to_le_bytes());
        assert!(LogIndex::decode(&unordered).is_none());
        let mut torn = good.clone();
        torn.extend_from_slice(&[0; 8]);
        assert!(LogIndex::decode(&torn).is_none());
    }

    #[test]
    fn freshness_detects_growth_truncation_and_rewrites() {
        let data = log(4);
        let index = built(&data[..built(&data).slots[3] as usize]);
        assert_eq!(
            index.freshness(index.covered, tail(&index, &data)),
            Freshness::Fresh
        );
        assert_eq!(
            index.freshness(data.len() as u64, tail(&index, &data)),
            Freshness::Extend
        );
        let short = &data[..index.covered as usize - 1];
        assert_eq!(
            index.freshness(short.len() as u64, tail(&index, short)),
            Freshness::Rebuild
        );
        let mut rewritten = data.clone();
        rewritten[index.tail as usize + 2] ^= 1;
        assert_eq!(
            index.freshness(rewritten.len() as u64, tail(&index, &rewritten)),
            Freshness::Rebuild
        );
        let empty = LogIndex::default();
        assert_eq!(empty.freshness(0, &[]), Freshness::Fresh);
        assert_eq!(empty.freshness(9, &[]), Freshness::Extend);
    }

    #[test]
    fn a_stopped_index_stays_fresh_while_the_log_grows() {
        let mut data = log(1);
        data.extend_from_slice(b"bad\n");
        let index = built(&data);
        data.extend_from_slice(&log(2));
        assert_eq!(
            index.freshness(data.len() as u64, tail(&index, &data)),
            Freshness::Fresh
        );
    }

    #[test]
    fn span_bounds() {
        let index = built(&log(3));
        assert_eq!(index.span(0, 3).map(|s| s.1), Some(index.covered));
        assert_eq!(index.span(0, 1).map(|s| s.1), Some(index.slots[1]));
        assert_eq!(index.span(2, 2), None);
        assert_eq!(index.span(2, 4), None);
    }

    fn tree_file() -> Vec<u8> {
        [
            encode_record(n(0, 2), "first").unwrap(),
            "not json\n".to_string(),
            "{\"lo\":1,\"hi\":3,\"text\":\"unaligned\"}\n".to_string(),
            encode_record(n(2, 3), "leaf").unwrap(),
            encode_record(n(0, 2), "second").unwrap(),
            "\n".to_string(),
        ]
        .concat()
        .into_bytes()
    }

    #[test]
    fn tree_index_skips_bad_records_and_finds_the_first_one() {
        let data = tree_file();
        let mut index = TreeIndex::default();
        index.extend(&data);
        assert_eq!(index.count(), 3);
        assert!(!index.stopped);
        assert_eq!(index.covered, data.len() as u64 - 1);
        let (tree, _) = parse_tree(&data);
        let found = index.find(&[n(0, 2), n(2, 3), n(4, 8)]);
        assert_eq!(found.len(), 2);
        for slot in found {
            let line = &data[slot.start as usize..slot.end as usize];
            let record: Record = serde_json::from_slice(line).unwrap();
            assert_eq!(Some(&record.text), tree.get(&slot.node));
        }
    }

    #[test]
    fn tree_index_covers_trailing_skipped_lines() {
        let mut data = encode_record(n(0, 1), "a").unwrap().into_bytes();
        data.extend_from_slice(b"junk\n");
        let mut index = TreeIndex::default();
        index.extend(&data);
        assert_eq!((index.count(), index.covered), (1, data.len() as u64));
        let (fresh, extend) = (Freshness::Fresh, Freshness::Extend);
        assert_eq!(
            index.freshness(data.len() as u64, tail(&index, &data)),
            fresh
        );
        let mut grown = data.clone();
        grown.extend_from_slice(encode_record(n(1, 2), "b").unwrap().as_bytes());
        assert_eq!(
            index.freshness(grown.len() as u64, tail(&index, &grown)),
            extend
        );
        let mut rewritten = data.clone();
        let at = index.tail as usize;
        rewritten[at] = b'J';
        assert_eq!(
            index.freshness(rewritten.len() as u64, tail(&index, &rewritten)),
            Freshness::Rebuild
        );
    }

    #[test]
    fn tree_index_round_trips_and_rejects_bad_slots() {
        let mut index = TreeIndex::default();
        index.extend(&tree_file());
        let bytes = index.encode();
        assert_eq!(TreeIndex::decode(&bytes), Some(index.clone()));
        assert!(LogIndex::decode(&bytes).is_none());
        let mut unaligned = bytes.clone();
        unaligned[HEADER_BYTES..HEADER_BYTES + 8].copy_from_slice(&3u64.to_le_bytes());
        assert!(TreeIndex::decode(&unaligned).is_none());
        let mut empty_span = bytes.clone();
        let end = HEADER_BYTES + 24..HEADER_BYTES + 32;
        empty_span[end].copy_from_slice(&index.slots[0].start.to_le_bytes());
        assert!(TreeIndex::decode(&empty_span).is_none());
    }
}
