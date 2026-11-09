//! Shell side of entry tagging: the `TAG` lock, `tags.jsonl`, and one pass
//! over the untagged entries. The pure parts live in `ralph_core::chat_memory`.

use crate::private_file::check_path;
use ralph_core::chat_memory::{
    batches, build_batch, decode_rows, encode_row, encode_start, failure_kind, ids_with, next_size,
    parse_log, parse_reply, untagged, Entry, RowKind, SearchRequest, TagRow, TagState,
    BATCH_LADDER, PROMPT_VERSION, TAGS_FILE, TAG_MODEL,
};
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub struct TagStore {
    dir: PathBuf,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub tagged: usize,
    pub failed: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Pass {
    /// Another tag pass holds `TAG` and will cover the new entries.
    Busy,
    Done(Report),
}

/// Asks the model one batch and returns its reply text.
pub type Ask<'a> = dyn FnMut(&str) -> Result<String, String> + 'a;

impl TagStore {
    pub fn new(dir: PathBuf) -> Self {
        TagStore { dir }
    }

    fn tags_path(&self) -> PathBuf {
        self.dir.join(TAGS_FILE)
    }

    fn open_lock(&self, name: &str) -> io::Result<File> {
        fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(name);
        check_path(&path)?;
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
    }

    /// Reads the log under a shared `LOCK`, so an append never shows half a line.
    pub fn read_entries(&self) -> io::Result<Vec<Entry>> {
        let lock = self.open_lock("LOCK")?;
        lock.lock_shared()?;
        let path = self.dir.join("log.jsonl");
        check_path(&path)?;
        let mut data = Vec::new();
        match File::open(path) {
            Ok(mut file) => {
                file.read_to_end(&mut data)?;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        Ok(parse_log(&data).0)
    }

    pub fn read_state(&self) -> io::Result<TagState> {
        let path = self.tags_path();
        check_path(&path)?;
        match fs::read(path) {
            Ok(data) => Ok(decode_rows(&data)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(TagState::default()),
            Err(e) => Err(e),
        }
    }

    /// Appends whole rows. A torn last line from a crash gets a newline first,
    /// so it stays one ignored line and never glues onto the new row.
    fn append(&self, text: &str) -> io::Result<()> {
        let path = self.tags_path();
        check_path(&path)?;
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(path)?;
        let mut last = [b'\n'];
        if file.metadata()?.len() > 0 {
            file.seek(SeekFrom::End(-1))?;
            file.read_exact(&mut last)?;
        }
        let mut out = String::new();
        if last[0] != b'\n' {
            out.push('\n');
        }
        out.push_str(text);
        file.write_all(out.as_bytes())?;
        file.sync_data()
    }

    /// One pass: write the start row on first use, then tag every untagged entry,
    /// re-reading the log between rounds. A model error ends the pass and writes
    /// no failure row; only an entry that fails alone is marked.
    pub fn run_pass(&self, now: &dyn Fn() -> String, ask: &mut Ask) -> io::Result<Pass> {
        let lock = self.open_lock("TAG")?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(Pass::Busy),
            Err(TryLockError::Error(e)) => return Err(e),
        }
        let mut report = Report::default();
        let mut attempted: BTreeSet<usize> = BTreeSet::new();
        loop {
            let entries = self.read_entries()?;
            let state = self.read_state()?;
            if state.start.is_none() {
                self.append(&encode_start(entries.len(), &now()).map_err(io::Error::other)?)?;
                return Ok(Pass::Done(report));
            }
            let todo: Vec<usize> = untagged(&entries, &state)
                .into_iter()
                .filter(|id| !attempted.contains(id))
                .take(BATCH_LADDER[0])
                .collect();
            if todo.is_empty() {
                return Ok(Pass::Done(report));
            }
            attempted.extend(&todo);
            let round = Round {
                store: self,
                entries: &entries,
                state: &state,
                now,
            };
            let failed = round.resolve(&todo, BATCH_LADDER[0], ask, &mut report)?;
            if !failed.is_empty() {
                round.mark_failed(&failed)?;
                report.failed += failed.len();
            }
        }
    }
}

struct Round<'a> {
    store: &'a TagStore,
    entries: &'a [Entry],
    state: &'a TagState,
    now: &'a dyn Fn() -> String,
}

impl Round<'_> {
    fn row(&self, n: usize, kind: RowKind) -> TagRow {
        TagRow {
            n,
            kind,
            model: TAG_MODEL.to_string(),
            prompt: PROMPT_VERSION.to_string(),
            at: (self.now)(),
        }
    }

    /// Asks `ids` at `size`; writes the valid rows; retries the rest one rung
    /// lower. Returns the ids that still failed alone.
    fn resolve(
        &self,
        ids: &[usize],
        size: usize,
        ask: &mut Ask,
        report: &mut Report,
    ) -> io::Result<Vec<usize>> {
        let size = if ids.len() == 1 { 1 } else { size };
        let reply = ask(&build_batch(self.entries, ids)).map_err(io::Error::other)?;
        let parsed = parse_reply(&reply, ids);
        let rows = parsed
            .tagged
            .iter()
            .map(|(n, flags)| encode_row(&self.row(*n, RowKind::Tagged(*flags))))
            .collect::<Result<String, String>>()
            .map_err(io::Error::other)?;
        if !rows.is_empty() {
            self.store.append(&rows)?;
            report.tagged += parsed.tagged.len();
        }
        let Some(next) = next_size(size) else {
            return Ok(parsed.missing);
        };
        let mut failed = Vec::new();
        if !parsed.missing.is_empty() {
            for chunk in batches(&parsed.missing, next) {
                failed.extend(self.resolve(&chunk, next, ask, report)?);
            }
        }
        Ok(failed)
    }

    fn mark_failed(&self, ids: &[usize]) -> io::Result<()> {
        let rows = ids
            .iter()
            .map(|n| {
                let earlier = self.state.failures.get(n).copied().unwrap_or(0);
                encode_row(&self.row(*n, failure_kind(earlier)))
            })
            .collect::<Result<String, String>>()
            .map_err(io::Error::other)?;
        self.store.append(&rows)
    }
}

/// Adds the tagged ids to a search request that filters by tag. Other
/// requests pass through without reading `tags.jsonl`.
pub(crate) fn with_tags(dir: &Path, request: &SearchRequest) -> io::Result<SearchRequest> {
    match request.tag() {
        Some(label) => {
            let state = TagStore::new(dir.to_path_buf()).read_state()?;
            Ok(request.clone().with_tag_ids(ids_with(&state, label)))
        }
        None => Ok(request.clone()),
    }
}

#[cfg(test)]
#[path = "tag_store_tests.rs"]
mod tests;
