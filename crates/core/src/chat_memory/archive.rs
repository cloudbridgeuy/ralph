use super::index::{scan_lines, Freshness, Index, LogIndex, Slot, TreeIndex};
use super::log::entry_or_corrupt;
use super::lookup::{zoom, zoom_entries};
use super::range::{
    read_range, Mode, RangePage, RangeRequest, Read, BUDGET_RANGE, BYTES_PER_TOKEN,
};
use super::{Entry, Node, Record, Tree, Window, SUMMARY_BYTES};
use std::io;

/// Random reads over a file snapshot of `size` bytes.
pub trait ByteSource {
    fn size(&self) -> u64;
    fn read(&mut self, start: u64, end: u64) -> io::Result<Vec<u8>>;
}

/// Summaries by node id. Returns only the asked nodes that have a summary.
pub trait NodeSource {
    fn nodes(&mut self, wanted: &[Node]) -> io::Result<Tree>;
}

#[cfg(test)]
impl NodeSource for Tree {
    fn nodes(&mut self, wanted: &[Node]) -> io::Result<Tree> {
        Ok(wanted
            .iter()
            .filter_map(|node| self.get(node).map(|text| (*node, text.clone())))
            .collect())
    }
}

/// Summaries read from `tree.jsonl` through its index, one read per node.
pub struct IndexedTree<B> {
    pub file: B,
    pub index: TreeIndex,
}

impl<B: ByteSource> NodeSource for IndexedTree<B> {
    fn nodes(&mut self, wanted: &[Node]) -> io::Result<Tree> {
        self.index
            .find(wanted)
            .into_iter()
            .map(|slot| {
                if slot.end >= self.index.covered {
                    return Err(stale("a tree slot ends past the index"));
                }
                let bytes = self.file.read(slot.start, slot.end)?;
                let record: Record = serde_json::from_slice(&bytes).map_err(stale)?;
                if (record.lo, record.hi) != (slot.node.lo, slot.node.hi) {
                    return Err(stale("a tree slot points at another node"));
                }
                Ok((slot.node, record.text))
            })
            .collect()
    }
}

/// An index that does not match its file: `Archive` rebuilds and retries.
fn stale(e: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

/// Which indexes of an `Archive` differ from the files on disk.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stale {
    pub log: bool,
    pub tree: bool,
}

/// The log, the tree and their indexes for one `read` or `zoom`. A call that
/// finds an index wrong rebuilds both from their files and runs once more.
pub struct Archive<B> {
    pub log: B,
    pub log_index: LogIndex,
    pub tree: IndexedTree<B>,
    pub stale: Stale,
    pub notes: Vec<String>,
}

impl<B: ByteSource> Archive<B> {
    /// Brings the stored indexes up to date in memory.
    pub fn open(
        mut log: B,
        stored_log: Option<LogIndex>,
        mut tree: B,
        stored_tree: Option<TreeIndex>,
    ) -> io::Result<Archive<B>> {
        let (log_index, log_how) = refresh(stored_log, &mut log)?;
        let (index, tree_how) = refresh(stored_tree, &mut tree)?;
        Ok(Archive {
            log,
            log_index,
            tree: IndexedTree { file: tree, index },
            stale: Stale {
                log: log_how != Refresh::Fresh,
                tree: tree_how != Refresh::Fresh,
            },
            notes: Vec::new(),
        })
    }

    pub fn read(&mut self, request: &RangeRequest) -> io::Result<Result<RangePage, String>> {
        self.retry(|a| read_page(&mut a.log, &a.log_index, &mut a.tree, &mut a.notes, request))
    }

    pub fn zoom(&mut self, node: Node) -> io::Result<Result<Vec<String>, String>> {
        self.retry(|a| zoom_node(&mut a.log, &a.log_index, &mut a.tree, &mut a.notes, node))
    }

    pub fn count(&self) -> usize {
        self.log_index.count()
    }

    pub fn entries(&mut self, from: usize, to: usize) -> io::Result<Vec<Entry>> {
        self.retry(|a| load(&mut a.log, &a.log_index, &mut a.notes, from, to))
    }

    /// Visits every entry in `chunk`-sized loads and returns how many it
    /// visited. The count is read again after each load, so entries that
    /// a mid-scan rebuild finds are visited too.
    pub fn scan_entries(
        &mut self,
        chunk: usize,
        mut visit: impl FnMut(usize, &Entry),
    ) -> io::Result<usize> {
        let chunk = chunk.max(1);
        let mut from = 0;
        while from < self.count() {
            let (to, loaded) = self.retry(|a| {
                let to = (from + chunk).min(a.count());
                load(&mut a.log, &a.log_index, &mut a.notes, from, to).map(|loaded| (to, loaded))
            })?;
            for (id, entry) in (from..).zip(loaded) {
                visit(id, &entry);
            }
            from = to;
        }
        Ok(self.count())
    }

    /// A warning per corrupt log line this call loaded, then the `parse_tree`
    /// warning for skipped tree records.
    pub fn warnings(&self) -> Vec<String> {
        self.notes
            .iter()
            .cloned()
            .chain(self.tree.index.warning())
            .collect()
    }

    fn retry<T>(&mut self, call: impl Fn(&mut Self) -> io::Result<T>) -> io::Result<T> {
        match call(self) {
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                self.log_index = refresh(None, &mut self.log)?.0;
                self.tree.index = refresh(None, &mut self.tree.file)?.0;
                self.stale = Stale {
                    log: true,
                    tree: true,
                };
                self.notes.clear();
                call(self)
            }
            other => other,
        }
    }
}

/// What `refresh` did, so a writer knows what to persist: nothing, the
/// slots from the given count on plus the header, or the whole file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refresh {
    Fresh,
    Extended(usize),
    Rebuilt,
}

/// Brings `stored` up to date with `file`. Reads only the hashed tail and the
/// bytes past `covered`, unless the index is missing or stale.
pub fn refresh<S: Slot>(
    stored: Option<Index<S>>,
    file: &mut impl ByteSource,
) -> io::Result<(Index<S>, Refresh)> {
    let size = file.size();
    let Some(mut index) = stored else {
        return rebuild(file, size);
    };
    let end = index.covered.min(size);
    let tail = file.read(index.tail.min(end), end)?;
    match index.freshness(size, &tail) {
        Freshness::Fresh => Ok((index, Refresh::Fresh)),
        Freshness::Rebuild => rebuild(file, size),
        Freshness::Extend => {
            let (count, header) = (index.count(), index.encode_header());
            index.extend(&file.read(index.covered, size)?);
            let refresh = if index.encode_header() != header {
                Refresh::Extended(count)
            } else {
                Refresh::Fresh
            };
            Ok((index, refresh))
        }
    }
}

/// How a writer persists a refreshed index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexWrite {
    Keep,
    /// Write `body` at byte `at`, then `header` at 0. A crash in between
    /// leaves a length the header does not match, so the next load rebuilds.
    InPlace {
        at: u64,
        body: Vec<u8>,
        header: Vec<u8>,
    },
    Replace(Vec<u8>),
}

pub fn write_plan<S: Slot>(index: &Index<S>, refresh: Refresh) -> IndexWrite {
    match refresh {
        Refresh::Fresh => IndexWrite::Keep,
        Refresh::Extended(from) => IndexWrite::InPlace {
            at: Index::<S>::slot_offset(from),
            body: index.encode_slots(from),
            header: index.encode_header(),
        },
        Refresh::Rebuilt => IndexWrite::Replace(index.encode()),
    }
}

fn rebuild<S: Slot>(file: &mut impl ByteSource, size: u64) -> io::Result<(Index<S>, Refresh)> {
    let mut index = Index::default();
    index.extend(&file.read(0, size)?);
    Ok((index, Refresh::Rebuilt))
}

fn load(
    log: &mut impl ByteSource,
    index: &LogIndex,
    notes: &mut Vec<String>,
    from: usize,
    to: usize,
) -> io::Result<Vec<Entry>> {
    let Some((start, end)) = index.span(from, to) else {
        return Ok(Vec::new());
    };
    // One byte early, so an offset into the middle of a line scans as a
    // line that starts elsewhere.
    let before = start.saturating_sub(1);
    let data = log.read(before, end)?;
    let lines = scan_lines(&data, before);
    let starts = lines.iter().map(|line| line.start);
    if !starts.eq(index.slots[from..to].iter().copied()) {
        return Err(stale("log offsets do not match the line starts"));
    }
    let mut entries = Vec::with_capacity(lines.len());
    for (id, line) in (from..).zip(&lines) {
        let bytes = &data[(line.start - before) as usize..(line.end - before) as usize];
        let (entry, warning) = entry_or_corrupt(bytes, id, line.start);
        if let Some(warning) = warning.filter(|w| !notes.contains(w)) {
            notes.push(warning);
        }
        entries.push(entry);
    }
    Ok(entries)
}

/// Entries from `start` whose lines hold at least the page budget in bytes.
fn first_window(index: &LogIndex, start: usize, budget: usize) -> usize {
    let want = (budget * BYTES_PER_TOKEN) as u64;
    let mut used = 0;
    let mut count = 0;
    while used < want {
        let Some((a, b)) = index.span(start + count, start + count + 1) else {
            break;
        };
        used += b - a;
        count += 1;
    }
    count.max(1)
}

fn long_leaves(start: usize, entries: &[Entry]) -> Vec<Node> {
    (start..)
        .zip(entries)
        .filter(|(_, entry)| entry.text.len() > SUMMARY_BYTES)
        .map(|(i, _)| Node::leaf(i))
        .collect()
}

/// One `read` page, loading only a window of entries from the cursor on. The
/// window doubles while the page runs past it before the budget is spent.
fn read_page(
    log: &mut impl ByteSource,
    index: &LogIndex,
    nodes: &mut impl NodeSource,
    notes: &mut Vec<String>,
    request: &RangeRequest,
) -> io::Result<Result<RangePage, String>> {
    let total = index.count();
    let start = request.cursor.map_or(request.from, |c| c.next).min(total);
    let budget = request.budget.min(*BUDGET_RANGE.end());
    let mut count = first_window(index, start, budget);
    loop {
        let end = start.saturating_add(count).min(total);
        let entries = load(log, index, notes, start, end)?;
        let tree = match request.mode {
            Mode::Summary => nodes.nodes(&long_leaves(start, &entries))?,
            Mode::Raw => Tree::new(),
        };
        let window = Window {
            start,
            total,
            entries: &entries,
        };
        match read_range(&window, &tree, request) {
            Ok(Read::Page(page)) => return Ok(Ok(page)),
            Ok(Read::More) if end < total => count = count.saturating_mul(2),
            Ok(Read::More) => return Ok(Err("the read window missed its range.".to_string())),
            Err(message) => return Ok(Err(message)),
        }
    }
}

/// `zoom` reading at most two entries and the two half summaries.
fn zoom_node(
    log: &mut impl ByteSource,
    index: &LogIndex,
    nodes: &mut impl NodeSource,
    notes: &mut Vec<String>,
    node: Node,
) -> io::Result<Result<Vec<String>, String>> {
    let total = index.count();
    let (start, end) = zoom_entries(node, total);
    let entries = load(log, index, notes, start, end)?;
    let tree = if node.size() > 1 {
        let (left, right) = node.halves();
        nodes.nodes(&[left, right])?
    } else {
        Tree::new()
    };
    let window = Window {
        start,
        total,
        entries: &entries,
    };
    Ok(zoom(&window, &tree, node))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_memory::fixtures::{entry, n};
    use crate::chat_memory::index::NodeSlot;
    use crate::chat_memory::range::Cursor;
    use crate::chat_memory::{encode_entry, encode_record, pending};

    struct Bytes {
        data: Vec<u8>,
        read: u64,
    }

    impl ByteSource for Bytes {
        fn size(&self) -> u64 {
            self.data.len() as u64
        }

        fn read(&mut self, start: u64, end: u64) -> io::Result<Vec<u8>> {
            self.read += end - start;
            Ok(self.data[start as usize..end as usize].to_vec())
        }
    }

    fn bytes(entries: &[Entry]) -> Bytes {
        let data = entries
            .iter()
            .map(|e| encode_entry(e).unwrap())
            .collect::<String>()
            .into_bytes();
        Bytes { data, read: 0 }
    }

    fn mixed(count: usize) -> Vec<Entry> {
        (0..count)
            .map(|i| entry(i, if i % 3 == 0 { 700 } else { 10 + i }))
            .collect()
    }

    fn summarized(entries: &[Entry], skip: usize) -> Tree {
        let mut tree = Tree::new();
        for (k, node) in pending(entries, &tree).into_iter().enumerate() {
            if k % skip != 0 {
                tree.insert(node, format!("sum {}", node.id()));
            }
        }
        tree
    }

    /// `tree` as `tree.jsonl` with a losing duplicate after each record.
    fn indexed(tree: &Tree) -> IndexedTree<Bytes> {
        let mut nodes: Vec<_> = tree.iter().collect();
        nodes.sort();
        let data = nodes
            .into_iter()
            .map(|(node, text)| {
                let first = encode_record(*node, text).unwrap();
                first.clone() + &encode_record(*node, "loser").unwrap()
            })
            .collect::<String>()
            .into_bytes();
        let mut file = Bytes { data, read: 0 };
        let (index, _) = refresh(None, &mut file).unwrap();
        IndexedTree { file, index }
    }

    fn full_page(
        entries: &[Entry],
        tree: &Tree,
        request: &RangeRequest,
    ) -> Result<RangePage, String> {
        match read_range(&Window::full(entries), tree, request)? {
            Read::Page(page) => Ok(page),
            Read::More => panic!("a full window never needs more"),
        }
    }

    fn scanned(archive: &mut Archive<Bytes>, chunk: usize) -> (Vec<usize>, usize) {
        let mut seen = Vec::new();
        let count = archive.scan_entries(chunk, |id, _| seen.push(id)).unwrap();
        (seen, count)
    }

    #[test]
    fn a_scan_after_a_shrinking_rebuild_visits_every_remaining_entry() {
        let entries = mixed(6);
        let mut archive = Archive::open(
            bytes(&entries),
            None,
            Bytes {
                data: Vec::new(),
                read: 0,
            },
            None,
        )
        .unwrap();
        let mut damaged = bytes(&entries).data;
        let joined = bytes(&entries[..5]).data.len() - 1;
        damaged[joined] = b' ';
        archive.log.data = damaged;
        let (seen, count) = scanned(&mut archive, 6);
        assert_eq!((seen, count), (vec![0, 1, 2, 3, 4], 5));
    }

    #[test]
    fn a_zero_chunk_scan_ends_and_visits_every_entry() {
        let entries = mixed(3);
        let mut archive = Archive::open(
            bytes(&entries),
            None,
            Bytes {
                data: Vec::new(),
                read: 0,
            },
            None,
        )
        .unwrap();
        assert_eq!(scanned(&mut archive, 0), (vec![0, 1, 2], 3));
    }

    #[test]
    fn refresh_builds_extends_and_rebuilds() {
        let entries = mixed(6);
        let mut log = bytes(&entries[..4]);
        let (index, how) = refresh::<u64>(None, &mut log).unwrap();
        assert_eq!((index.count(), how), (4, Refresh::Rebuilt));
        let (same, how) = refresh(Some(index.clone()), &mut log).unwrap();
        assert_eq!((same, how), (index.clone(), Refresh::Fresh));
        let mut grown = bytes(&entries);
        grown.read = 0;
        let (bigger, how) = refresh(Some(index.clone()), &mut grown).unwrap();
        assert_eq!((bigger.count(), how), (6, Refresh::Extended(4)));
        assert!(grown.read < grown.size());
        let mut shrunk = bytes(&entries[..2]);
        let (rebuilt, how) = refresh(Some(index), &mut shrunk).unwrap();
        assert_eq!((rebuilt.count(), how), (2, Refresh::Rebuilt));
    }

    #[test]
    fn refresh_ignores_a_torn_append() {
        let entries = mixed(3);
        let mut log = bytes(&entries);
        let (index, _) = refresh::<u64>(None, &mut log).unwrap();
        log.data.extend_from_slice(b"{\"role\":\"us");
        let (same, how) = refresh(Some(index.clone()), &mut log).unwrap();
        assert_eq!((same, how), (index, Refresh::Fresh));
    }

    fn apply(file: &mut Vec<u8>, write: IndexWrite) {
        match write {
            IndexWrite::Keep => {}
            IndexWrite::InPlace { at, body, header } => {
                file.truncate(at as usize);
                file.extend_from_slice(&body);
                file[..header.len()].copy_from_slice(&header);
            }
            IndexWrite::Replace(bytes) => *file = bytes,
        }
    }

    #[test]
    fn write_plans_keep_the_file_equal_to_a_full_encode() {
        let entries = mixed(9);
        let mut file = Vec::new();
        let mut stored = None;
        for end in [2, 2, 5, 9] {
            let mut log = bytes(&entries[..end]);
            let (index, how) = refresh(stored, &mut log).unwrap();
            apply(&mut file, write_plan(&index, how));
            assert_eq!(file, index.encode(), "end {end}");
            stored = LogIndex::decode(&file);
        }
        assert_eq!(
            write_plan(&LogIndex::default(), Refresh::Fresh),
            IndexWrite::Keep
        );
    }

    #[test]
    fn a_version_1_index_is_rebuilt_and_replaced() {
        let entries = mixed(5);
        let mut log = bytes(&entries);
        let (current, _) = refresh::<u64>(None, &mut log).unwrap();
        let mut old = current.encode();
        old.drain(40..48);
        old[8..12].copy_from_slice(&1u32.to_le_bytes());
        let (index, how) = refresh(LogIndex::decode(&old), &mut log).unwrap();
        assert_eq!(how, Refresh::Rebuilt);
        assert_eq!(index, current);
        assert_eq!(
            write_plan(&index, how),
            IndexWrite::Replace(current.encode())
        );
    }

    #[test]
    fn tree_index_refreshes_like_a_full_build() {
        let tree = summarized(&mixed(20), 2);
        let whole = indexed(&tree).file.data;
        let mut file = Vec::new();
        let mut stored = None;
        for end in [0, whole.len() / 3, whole.len() / 3, whole.len()] {
            let mut source = Bytes {
                data: whole[..end].to_vec(),
                read: 0,
            };
            let (index, how) = refresh::<NodeSlot>(stored, &mut source).unwrap();
            let mut built = TreeIndex::default();
            built.extend(&whole[..end]);
            assert_eq!(index, built, "end {end}");
            apply(&mut file, write_plan(&index, how));
            assert_eq!(file, index.encode(), "end {end}");
            stored = TreeIndex::decode(&file);
        }
        let mut rewritten = Bytes {
            data: whole.clone(),
            read: 0,
        };
        rewritten.data[stored.as_ref().unwrap().tail as usize + 8] ^= 1;
        let (_, how) = refresh(stored, &mut rewritten).unwrap();
        assert_eq!(how, Refresh::Rebuilt);
    }

    #[test]
    fn read_pages_match_the_full_slice_with_cursors() {
        let entries = mixed(40);
        for skip in [1, 2, 5] {
            let tree = summarized(&entries, skip);
            let mut nodes = indexed(&tree);
            let mut log = bytes(&entries);
            let (index, _) = refresh::<u64>(None, &mut log).unwrap();
            for mode in [Mode::Raw, Mode::Summary] {
                for budget in [1, 7, 60, 400, 16384] {
                    let mut request = RangeRequest {
                        from: 3,
                        to: 37,
                        mode,
                        budget,
                        cursor: None,
                    };
                    let mut pages = 0;
                    loop {
                        let windowed =
                            read_page(&mut log, &index, &mut nodes, &mut Vec::new(), &request)
                                .unwrap();
                        let full = full_page(&entries, &tree, &request);
                        assert_eq!(windowed, full, "{mode:?} budget {budget}");
                        let Some(next) = full.unwrap().next else {
                            break;
                        };
                        request.cursor = Some(next);
                        pages += 1;
                        assert!(pages < 10_000);
                    }
                }
            }
        }
    }

    #[test]
    fn read_errors_match_the_full_slice() {
        let entries = mixed(5);
        let mut log = bytes(&entries);
        let (index, _) = refresh::<u64>(None, &mut log).unwrap();
        let mut tree = Tree::new();
        let base = RangeRequest {
            from: 1,
            to: 3,
            mode: Mode::Raw,
            budget: 100,
            cursor: None,
        };
        let stale = |next, offset, end| RangeRequest {
            cursor: Some(Cursor { next, offset, end }),
            ..base.clone()
        };
        for request in [
            RangeRequest {
                from: 9,
                to: 9,
                ..base.clone()
            },
            RangeRequest {
                from: 3,
                to: 1,
                ..base.clone()
            },
            RangeRequest {
                budget: 0,
                ..base.clone()
            },
            RangeRequest {
                budget: usize::MAX,
                ..base.clone()
            },
            stale(0, 0, 4),
            stale(7, 0, 9),
            stale(2, 999, 4),
        ] {
            let windowed =
                read_page(&mut log, &index, &mut tree, &mut Vec::new(), &request).unwrap();
            assert!(windowed.is_err(), "{request:?}");
            assert_eq!(windowed, full_page(&entries, &tree, &request));
        }
    }

    #[test]
    fn a_small_read_loads_a_small_window() {
        let entries = mixed(400);
        let mut log = bytes(&entries);
        let (index, _) = refresh::<u64>(None, &mut log).unwrap();
        log.read = 0;
        let request = RangeRequest {
            from: 200,
            to: 399,
            mode: Mode::Raw,
            budget: 10,
            cursor: None,
        };
        read_page(
            &mut log,
            &index,
            &mut Tree::new(),
            &mut Vec::new(),
            &request,
        )
        .unwrap()
        .unwrap();
        assert!(log.read * 50 < log.size(), "read {} bytes", log.read);
    }

    #[test]
    fn zoom_matches_the_full_slice_for_every_node() {
        let entries = mixed(21);
        let tree = summarized(&entries, 3);
        let mut nodes = indexed(&tree);
        let mut log = bytes(&entries);
        let (index, _) = refresh::<u64>(None, &mut log).unwrap();
        for size in [1, 2, 4, 8, 16, 32] {
            for lo in (0..40).step_by(size) {
                let node = n(lo, lo + size);
                let windowed =
                    zoom_node(&mut log, &index, &mut nodes, &mut Vec::new(), node).unwrap();
                assert_eq!(
                    windowed,
                    zoom(&Window::full(&entries), &tree, node),
                    "{node:?}"
                );
            }
        }
    }

    /// An archive over `entries` and `tree` whose stored indexes `damage`
    /// may break, as a decode that still passes would leave them.
    fn damaged(
        entries: &[Entry],
        tree: &Tree,
        damage: impl FnOnce(&mut LogIndex, &mut TreeIndex),
    ) -> Archive<Bytes> {
        let mut log = bytes(entries);
        let (mut log_index, _) = refresh::<u64>(None, &mut log).unwrap();
        let IndexedTree { file, mut index } = indexed(tree);
        damage(&mut log_index, &mut index);
        let log_index = LogIndex::decode(&log_index.encode());
        let index = TreeIndex::decode(&index.encode());
        assert!(log_index.is_some() && index.is_some(), "damage must decode");
        Archive::open(log, log_index, file, index).unwrap()
    }

    #[test]
    fn an_intact_archive_answers_without_rebuilding() {
        let entries = mixed(12);
        let tree = summarized(&entries, 2);
        let mut archive = damaged(&entries, &tree, |_, _| {});
        assert_eq!(archive.stale, Stale::default());
        let node = n(0, 8);
        let want = zoom(&Window::full(&entries), &tree, node);
        assert_eq!(archive.zoom(node).unwrap(), want);
        assert_eq!(archive.stale, Stale::default());
        assert!(archive.warnings().is_empty());
    }

    #[test]
    fn a_shifted_log_slot_rebuilds_and_answers_right() {
        let entries = mixed(12);
        let tree = summarized(&entries, 2);
        for shift in [1, 3] {
            let mut archive = damaged(&entries, &tree, |log, _| log.slots[5] += shift);
            let request = RangeRequest {
                from: 2,
                to: 9,
                mode: Mode::Raw,
                budget: 16384,
                cursor: None,
            };
            let page = archive.read(&request).unwrap();
            assert_eq!(page, full_page(&entries, &tree, &request));
            assert_eq!(
                archive.stale,
                Stale {
                    log: true,
                    tree: true
                }
            );
            let mut archive = damaged(&entries, &tree, |log, _| log.slots[5] += shift);
            let want = zoom(&Window::full(&entries), &tree, n(5, 6));
            assert_eq!(archive.zoom(n(5, 6)).unwrap(), want);
        }
    }

    #[test]
    fn a_relabeled_tree_slot_rebuilds_and_answers_right() {
        let entries = mixed(12);
        let tree = summarized(&entries, 100);
        let node = n(0, 8);
        let (left, right) = node.halves();
        assert!(tree.contains_key(&left) && tree.contains_key(&right));
        let mut archive = damaged(&entries, &tree, |_, index| {
            let slot = index.slots.iter_mut().find(|s| s.node != left).unwrap();
            slot.node = left;
        });
        let want = zoom(&Window::full(&entries), &tree, node);
        assert_eq!(archive.zoom(node).unwrap(), want);
        assert!(archive.stale.tree);
    }

    fn with_corrupt_line(entries: &[Entry], at: usize) -> (Bytes, u64) {
        let mut log = bytes(&entries[..at]);
        let offset = log.data.len() as u64;
        log.data.extend_from_slice(b"garbage\n");
        log.data.extend_from_slice(&bytes(&entries[at..]).data);
        (log, offset)
    }

    fn open_log(log: Bytes) -> Archive<Bytes> {
        Archive::open(
            log,
            None,
            Bytes {
                data: Vec::new(),
                read: 0,
            },
            None,
        )
        .unwrap()
    }

    #[test]
    fn a_corrupt_mid_log_line_keeps_later_entries_visible_with_stable_ids() {
        let entries = mixed(5);
        let (log, offset) = with_corrupt_line(&entries, 2);
        let mut archive = open_log(log);
        assert_eq!(archive.count(), 6);
        let loaded = archive.entries(0, 6).unwrap();
        assert_eq!(loaded[..2], entries[..2]);
        assert!(loaded[2].is_corrupt());
        assert_eq!(loaded[3..], entries[2..]);
        let request = RangeRequest {
            from: 0,
            to: 5,
            mode: Mode::Raw,
            budget: 16384,
            cursor: None,
        };
        let page = archive.read(&request).unwrap().unwrap().render();
        assert!(
            page.contains("#2 · worker ·  · 15 chars\n[corrupt entry]"),
            "{page}"
        );
        assert!(page.contains("#3 · user · "), "{page}");
        assert!(page.contains(&entries[2].text), "{page}");
        let zoomed = archive.zoom(n(3, 4)).unwrap().unwrap();
        assert!(zoomed[0].starts_with("#3 "), "{zoomed:?}");
        assert!(zoomed[0].contains(&entries[2].text), "{zoomed:?}");
        let warnings = archive.warnings();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].starts_with(&format!("chat log entry 2 at byte {offset} is corrupt")),
            "{warnings:?}"
        );
        assert_eq!(scanned(&mut archive, 2), ((0..6).collect(), 6));
        assert_eq!(archive.warnings().len(), 1);
    }

    #[test]
    fn archive_warnings_cover_the_tree_and_clean_logs_stay_quiet() {
        let tree = |data: &[u8]| Bytes {
            data: data.to_vec(),
            read: 0,
        };
        let mut archive = Archive::open(bytes(&mixed(2)), None, tree(b"junk\n"), None).unwrap();
        archive.entries(0, 2).unwrap();
        assert_eq!(archive.warnings(), ["chat tree: 1 records skipped"]);
    }

    #[test]
    fn first_window_holds_the_budget() {
        let entries = mixed(10);
        let mut log = bytes(&entries);
        let (index, _) = refresh::<u64>(None, &mut log).unwrap();
        assert_eq!(first_window(&index, 0, 1), 1);
        assert_eq!(first_window(&index, 9, 16384), 1);
        assert_eq!(first_window(&index, 10, 16384), 1);
        assert_eq!(first_window(&index, 0, 16384), 10);
        assert_eq!(long_leaves(3, &entries[3..5]), [Node::leaf(3)]);
    }
}
