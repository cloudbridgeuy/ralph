use super::index::{scan_lines, Freshness, Index, LogIndex, Slot, TreeIndex};
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
                let bytes = self.file.read(slot.start, slot.end)?;
                let record: Record = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
                Ok((slot.node, record.text))
            })
            .collect()
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
    from: usize,
    to: usize,
) -> io::Result<Vec<Entry>> {
    let Some((start, end)) = index.span(from, to) else {
        return Ok(Vec::new());
    };
    let data = log.read(start, end)?;
    scan_lines(&data, start)
        .iter()
        .map(|line| {
            let bytes = &data[(line.start - start) as usize..(line.end - start) as usize];
            serde_json::from_slice(bytes).map_err(io::Error::other)
        })
        .collect()
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
pub fn read_page(
    log: &mut impl ByteSource,
    index: &LogIndex,
    nodes: &mut impl NodeSource,
    request: &RangeRequest,
) -> io::Result<Result<RangePage, String>> {
    let total = index.count();
    let start = request.cursor.map_or(request.from, |c| c.next).min(total);
    let budget = request.budget.min(*BUDGET_RANGE.end());
    let mut count = first_window(index, start, budget);
    loop {
        let end = start.saturating_add(count).min(total);
        let entries = load(log, index, start, end)?;
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
pub fn zoom_node(
    log: &mut impl ByteSource,
    index: &LogIndex,
    nodes: &mut impl NodeSource,
    node: Node,
) -> io::Result<Result<Vec<String>, String>> {
    let total = index.count();
    let (start, end) = zoom_entries(node, total);
    let entries = load(log, index, start, end)?;
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
                        let windowed = read_page(&mut log, &index, &mut nodes, &request).unwrap();
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
            let windowed = read_page(&mut log, &index, &mut tree, &request).unwrap();
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
        read_page(&mut log, &index, &mut Tree::new(), &request)
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
                let windowed = zoom_node(&mut log, &index, &mut nodes, node).unwrap();
                assert_eq!(
                    windowed,
                    zoom(&Window::full(&entries), &tree, node),
                    "{node:?}"
                );
            }
        }
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
