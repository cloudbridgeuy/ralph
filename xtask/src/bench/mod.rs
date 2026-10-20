//! `cargo xtask bench-memory`: the cost of one chat-memory `read` or `zoom`
//! when it loads the whole archive and when it goes through `log.idx` and
//! `tree.idx`. Both paths use the `ralph_core` functions the chat-mcp server
//! uses; the file reads mirror `Store`.

use crate::prelude::*;
use alloc::Allocs;
use ralph_core::chat_memory::{
    encode_entry, encode_record, parse_log, parse_tree, pending, read_range, zoom, Archive,
    ByteSource, Entry, LogIndex, Mode, Node, RangeRequest, Read, Role, Tree, TreeIndex, Window,
    LOG_INDEX_FILE, TREE_INDEX_FILE,
};
use std::fs::{self, File};
use std::io::{self, Read as _, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

mod alloc;

#[derive(Debug, clap::Parser)]
pub struct BenchMemoryCommand {
    /// Archive sizes in entries
    #[arg(long, value_delimiter = ',', default_value = "1000,10000,100000")]
    pub sizes: Vec<usize>,

    /// Timed runs per case; the table shows the median latency
    #[arg(long, default_value_t = 9)]
    pub runs: usize,
}

type Answer = Result<String, String>;

#[derive(Debug, Clone)]
enum Case {
    Read(RangeRequest),
    Zoom(Node),
}

/// The cases for an archive of `n` entries: the last 64 entries raw, the
/// whole archive in summary mode, and an 8-entry block in the middle.
fn cases(n: usize) -> Vec<(&'static str, Case)> {
    let request = |from, mode| RangeRequest {
        from,
        to: n - 1,
        mode,
        budget: 4096,
        cursor: None,
    };
    let mid = (n / 2) / 8 * 8;
    let mut cases = vec![
        (
            "read raw tail",
            Case::Read(request(n.saturating_sub(64), Mode::Raw)),
        ),
        ("read summary all", Case::Read(request(0, Mode::Summary))),
    ];
    if let Ok(node) = Node::new(mid, mid + 8) {
        cases.push(("zoom 8 mid", Case::Zoom(node)));
    }
    cases
}

fn text(i: usize) -> String {
    let len = if i.is_multiple_of(3) { 700 } else { 120 };
    format!("entry {i} ").repeat(len / 8 + 1)[..len].to_string()
}

/// `log.jsonl` and `tree.jsonl` for `n` entries, with every node `pending`
/// offers summarized.
fn fixture(n: usize) -> Result<(String, String), String> {
    let entries: Vec<Entry> = (0..n)
        .map(|i| Entry {
            role: if i.is_multiple_of(2) {
                Role::User
            } else {
                Role::Assistant
            },
            at: "2026-10-20T00:00:00+00:00".to_string(),
            text: text(i),
        })
        .collect();
    let log = entries
        .iter()
        .map(encode_entry)
        .collect::<Result<String, String>>()?;
    let mut tree = Tree::new();
    let mut records = String::new();
    for _ in 0..64 {
        let nodes = pending(&entries, &tree);
        if nodes.is_empty() {
            break;
        }
        for node in nodes {
            let summary = format!("summary of #{}: {}", node.id(), "s".repeat(160));
            records.push_str(&encode_record(node, &summary)?);
            tree.insert(node, summary);
        }
    }
    Ok((log, records))
}

fn median(mut values: Vec<u128>) -> u128 {
    values.sort_unstable();
    values.get(values.len() / 2).copied().unwrap_or(0)
}

struct Sample {
    micros: u128,
    read: u64,
    allocs: Allocs,
}

struct Row<'a> {
    entries: usize,
    case: &'a str,
    path: &'a str,
    sample: Sample,
}

fn render(rows: &[Row]) -> String {
    let mut out = String::from(
        "| entries | case | path | median µs | bytes read | allocs | alloc bytes |\n\
         |---:|---|---|---:|---:|---:|---:|\n",
    );
    for row in rows {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            row.entries,
            row.case,
            row.path,
            row.sample.micros,
            row.sample.read,
            row.sample.allocs.count,
            row.sample.allocs.bytes,
        ));
    }
    out
}

static READ: AtomicU64 = AtomicU64::new(0);

fn read_counted(path: &Path) -> io::Result<Vec<u8>> {
    let bytes = fs::read(path)?;
    READ.fetch_add(bytes.len() as u64, Ordering::Relaxed);
    Ok(bytes)
}

/// A file snapshot that counts the bytes it reads.
struct Counted {
    file: File,
    size: u64,
}

impl Counted {
    fn open(path: &Path) -> io::Result<Counted> {
        let file = File::open(path)?;
        let size = file.metadata()?.len();
        Ok(Counted { file, size })
    }
}

impl ByteSource for Counted {
    fn size(&self) -> u64 {
        self.size
    }

    fn read(&mut self, start: u64, end: u64) -> io::Result<Vec<u8>> {
        let mut buf = vec![0; usize::try_from(end - start).map_err(io::Error::other)?];
        self.file.seek(SeekFrom::Start(start))?;
        self.file.read_exact(&mut buf)?;
        READ.fetch_add(buf.len() as u64, Ordering::Relaxed);
        Ok(buf)
    }
}

/// The path before GUZ-709: parse both files whole, then slice.
fn full(dir: &Path, case: &Case) -> io::Result<Answer> {
    let (entries, _) = parse_log(&read_counted(&dir.join("log.jsonl"))?);
    let (tree, _) = parse_tree(&read_counted(&dir.join("tree.jsonl"))?);
    let window = Window::full(&entries);
    Ok(match case {
        Case::Read(request) => match read_range(&window, &tree, request) {
            Ok(Read::Page(page)) => Ok(page.render()),
            Ok(Read::More) => Err("a full window asked for more".to_string()),
            Err(e) => Err(e),
        },
        Case::Zoom(node) => zoom(&window, &tree, *node).map(|lines| lines.join("\n")),
    })
}

/// The path after GUZ-709: decode both indexes, then load only what shows.
fn indexed(dir: &Path, case: &Case) -> io::Result<Answer> {
    let mut archive = Archive::open(
        Counted::open(&dir.join("log.jsonl"))?,
        LogIndex::decode(&read_counted(&dir.join(LOG_INDEX_FILE))?),
        Counted::open(&dir.join("tree.jsonl"))?,
        TreeIndex::decode(&read_counted(&dir.join(TREE_INDEX_FILE))?),
    )?;
    Ok(match case {
        Case::Read(request) => archive.read(request)?.map(|page| page.render()),
        Case::Zoom(node) => archive.zoom(*node)?.map(|lines| lines.join("\n")),
    })
}

type Load = fn(&Path, &Case) -> io::Result<Answer>;

fn measure(dir: &Path, case: &Case, load: Load, runs: usize) -> io::Result<(Sample, Answer)> {
    let read = READ.load(Ordering::Relaxed);
    let allocs = Allocs::now();
    let answer = load(dir, case)?;
    let allocs = Allocs::now().since(allocs);
    let read = READ.load(Ordering::Relaxed) - read;
    let mut times = Vec::with_capacity(runs);
    for _ in 0..runs {
        let started = Instant::now();
        let _ = std::hint::black_box(load(dir, case)?);
        times.push(started.elapsed().as_micros());
    }
    let sample = Sample {
        micros: median(times),
        read,
        allocs,
    };
    Ok((sample, answer))
}

fn write_store(dir: &Path, n: usize) -> io::Result<()> {
    let (log, tree) = fixture(n).map_err(io::Error::other)?;
    let mut log_index = LogIndex::default();
    log_index.extend(log.as_bytes());
    let mut tree_index = TreeIndex::default();
    tree_index.extend(tree.as_bytes());
    fs::write(dir.join("log.jsonl"), log)?;
    fs::write(dir.join("tree.jsonl"), tree)?;
    fs::write(dir.join(LOG_INDEX_FILE), log_index.encode())?;
    fs::write(dir.join(TREE_INDEX_FILE), tree_index.encode())
}

pub fn run(command: BenchMemoryCommand) -> Result<(), Box<dyn std::error::Error>> {
    alloc::start_counting();
    let mut rows = Vec::new();
    for n in command.sizes.iter().copied().filter(|n| *n > 0) {
        let dir = tempfile::tempdir()?;
        write_store(dir.path(), n)?;
        for (name, case) in cases(n) {
            let (before, expected) = measure(dir.path(), &case, full, command.runs)?;
            let (after, answer) = measure(dir.path(), &case, indexed, command.runs)?;
            if answer != expected {
                return Err(format!("{n} entries, {name}: the indexed answer differs").into());
            }
            for (path, sample) in [("full", before), ("indexed", after)] {
                rows.push(Row {
                    entries: n,
                    case: name,
                    path,
                    sample,
                });
            }
        }
    }
    aprintln!("{}", render(&rows));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_summarizes_and_cases_fit_the_archive() {
        let (log, tree) = fixture(40).unwrap();
        let (entries, warnings) = parse_log(log.as_bytes());
        assert_eq!((entries.len(), warnings.len()), (40, 0));
        let (tree, _) = parse_tree(tree.as_bytes());
        assert!(tree.len() > 20);
        assert_eq!(cases(40).len(), 3);
        assert_eq!(text(3).len(), 700);
    }

    #[test]
    fn median_takes_the_middle_value() {
        assert_eq!(median(vec![5, 1, 9]), 5);
        assert_eq!(median(Vec::new()), 0);
    }
}
