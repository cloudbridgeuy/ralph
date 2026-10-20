use super::Entry;

pub const LOG_INDEX_FILE: &str = "log.idx";

const LOG_MAGIC: [u8; 8] = *b"RLPHLOGX";
const VERSION: u32 = 1;
const HEADER_BYTES: usize = 40;
const STOPPED: u32 = 1;

/// Byte span of one complete, non-blank line, newline excluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line {
    pub start: u64,
    pub end: u64,
}

/// Start offsets of the `log.jsonl` entries in `[0, covered)`. `covered` ends
/// on the newline of the last indexed entry; `hash` is the FNV-1a hash of the
/// bytes `[last_start, covered)`, so a rewritten log shows as stale.
/// `stopped` means a corrupt line follows `covered`: like `parse_log`, nothing
/// after it is visible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogIndex {
    pub offsets: Vec<u64>,
    pub covered: u64,
    pub hash: u64,
    pub stopped: bool,
}

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

impl Default for LogIndex {
    fn default() -> Self {
        Self {
            offsets: Vec::new(),
            covered: 0,
            hash: fnv1a(&[]),
            stopped: false,
        }
    }
}

impl LogIndex {
    pub fn count(&self) -> usize {
        self.offsets.len()
    }

    /// Start of the bytes that `hash` covers.
    pub fn last_start(&self) -> u64 {
        self.offsets.last().copied().unwrap_or(0)
    }

    /// Indexes the entries in `data`, the log bytes from `covered` on. Each
    /// new line must parse as an `Entry`; the first one that does not stops
    /// the index for good.
    pub fn extend(&mut self, data: &[u8]) {
        if self.stopped {
            return;
        }
        let base = self.covered;
        let mut last = None;
        for line in scan_lines(data, base) {
            let bytes = &data[(line.start - base) as usize..(line.end - base) as usize];
            if serde_json::from_slice::<Entry>(bytes).is_err() {
                self.stopped = true;
                break;
            }
            self.offsets.push(line.start);
            last = Some(line);
        }
        if let Some(line) = last {
            self.covered = line.end + 1;
            let from = (self.last_start() - base) as usize;
            self.hash = fnv1a(&data[from..(self.covered - base) as usize]);
        }
    }

    /// Compares the index with a log of `log_len` bytes whose bytes
    /// `[last_start, covered)` are `tail` (shorter when the log is shorter).
    pub fn freshness(&self, log_len: u64, tail: &[u8]) -> Freshness {
        if log_len < self.covered || fnv1a(tail) != self.hash {
            Freshness::Rebuild
        } else if self.stopped || log_len == self.covered {
            Freshness::Fresh
        } else {
            Freshness::Extend
        }
    }

    /// Byte span of entries `[from, to)`; `None` when the range is empty or
    /// past the index.
    pub fn span(&self, from: usize, to: usize) -> Option<(u64, u64)> {
        if from >= to || to > self.count() {
            return None;
        }
        let end = self.offsets.get(to).copied().unwrap_or(self.covered);
        Some((self.offsets[from], end))
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.encode_header();
        for offset in &self.offsets {
            out.extend_from_slice(&offset.to_le_bytes());
        }
        out
    }

    /// The header alone, for an in-place extend: body first, then this.
    pub fn encode_header(&self) -> Vec<u8> {
        let mut out = header(LOG_MAGIC, self.stopped, self.covered, self.hash);
        out.extend_from_slice(&(self.count() as u64).to_le_bytes());
        out
    }

    pub fn body_offset(count: usize) -> u64 {
        (HEADER_BYTES + count * 8) as u64
    }

    /// `None` for a missing, foreign, torn or inconsistent file; the caller
    /// then rebuilds from the log.
    pub fn decode(bytes: &[u8]) -> Option<LogIndex> {
        let (stopped, covered, hash, count) = read_header(bytes, LOG_MAGIC)?;
        let body = &bytes[HEADER_BYTES..];
        if body.len() != count.checked_mul(8)? {
            return None;
        }
        let offsets: Vec<u64> = body.chunks_exact(8).map(read_u64).collect();
        let ordered = offsets.windows(2).all(|w| w[0] < w[1]);
        if !ordered || offsets.last().is_some_and(|last| *last >= covered) {
            return None;
        }
        if offsets.is_empty() && covered != 0 {
            return None;
        }
        Some(LogIndex {
            offsets,
            covered,
            hash,
            stopped,
        })
    }
}

fn header(magic: [u8; 8], stopped: bool, covered: u64, hash: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_BYTES);
    out.extend_from_slice(&magic);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(if stopped { STOPPED } else { 0 }).to_le_bytes());
    out.extend_from_slice(&covered.to_le_bytes());
    out.extend_from_slice(&hash.to_le_bytes());
    out
}

fn read_u64(bytes: &[u8]) -> u64 {
    let mut word = [0; 8];
    word.copy_from_slice(&bytes[..8]);
    u64::from_le_bytes(word)
}

/// Returns `(stopped, covered, hash, count)`.
fn read_header(bytes: &[u8], magic: [u8; 8]) -> Option<(bool, u64, u64, usize)> {
    if bytes.len() < HEADER_BYTES || bytes[..8] != magic {
        return None;
    }
    let mut version = [0; 4];
    version.copy_from_slice(&bytes[8..12]);
    let mut flags = [0; 4];
    flags.copy_from_slice(&bytes[12..16]);
    if u32::from_le_bytes(version) != VERSION {
        return None;
    }
    let count = usize::try_from(read_u64(&bytes[32..40])).ok()?;
    Some((
        u32::from_le_bytes(flags) & STOPPED != 0,
        read_u64(&bytes[16..24]),
        read_u64(&bytes[24..32]),
        count,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_memory::encode_entry;
    use crate::chat_memory::fixtures::entry;

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

    fn tail<'a>(index: &LogIndex, data: &'a [u8]) -> &'a [u8] {
        let end = (index.covered as usize).min(data.len());
        &data[(index.last_start() as usize).min(end)..end]
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
        let mut stepped = built(&data[..whole.offsets[3] as usize]);
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
        let index = built(&data[..built(&data).offsets[3] as usize]);
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
        rewritten[index.last_start() as usize + 2] ^= 1;
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
        assert_eq!(index.span(0, 1).map(|s| s.1), Some(index.offsets[1]));
        assert_eq!(index.span(2, 2), None);
        assert_eq!(index.span(2, 4), None);
    }
}
