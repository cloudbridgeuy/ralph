//! Shell side of entry tagging: the `TAG` lock, `tags.jsonl`, and one pass
//! over the untagged entries. The pure parts live in `ralph_core::chat_memory`.

use crate::private_file::check_path;
use ralph_core::chat_memory::{
    after_call, batches, build_batch, context_exclusions, decode_rows, encode_row, encode_start,
    failure_kind, ids_with, isolated, next_size, parse_log, parse_reply, restrict_decision, retake,
    take_confirmed, untagged, CallReply, Entry, PassHealth, Role, RowKind, SearchRequest, TagRow,
    TagState, Verdict, BATCH_LADDER, MAX_PER_PASS, PROMPT_VERSION, TAGS_FILE, TAG_MODEL,
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

/// What one pass has done so far: the entries it asked, the rows it wrote, how
/// its calls fared, and whether the health guard ended it.
#[derive(Default)]
struct Progress {
    attempted: BTreeSet<usize>,
    report: Report,
    health: PassHealth,
    /// Entries that failed alone, in this pass or an earlier one; their text
    /// stays out of the context of later batches.
    excluded: BTreeSet<usize>,
    stopped: bool,
}

/// Asks the model one batch and returns what the call gave.
pub type Ask<'a> = dyn FnMut(&str) -> Result<CallReply, String> + 'a;

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

    /// Moves the start point to the end of the log without tagging, for a
    /// chat whose tagging is off, so turning it on never backfills.
    pub fn skip_ahead(&self, now: &dyn Fn() -> String) -> io::Result<()> {
        let lock = self.open_lock("TAG")?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(()),
            Err(TryLockError::Error(e)) => return Err(e),
        }
        let end = self.read_entries()?.len();
        if self.read_state()?.start != Some(end) {
            self.append(&encode_start(end, &now()).map_err(io::Error::other)?)?;
        }
        Ok(())
    }

    /// One pass: write the start row on first use, then tag the untagged entries
    /// (at most `MAX_PER_PASS`), re-reading the log between rounds. A call that
    /// never finished, or a run of failed calls with no good one (see
    /// `after_call`), ends the pass; only an entry that fails alone is marked,
    /// and only once a call of the pass exited 0. An entry that lands after the last read but
    /// before `TAG` is released is covered by one more look after the release.
    pub fn run_pass(&self, now: &dyn Fn() -> String, ask: &mut Ask) -> io::Result<Pass> {
        let mut progress = Progress::default();
        let Some(mut seen) = self.locked_pass(now, ask, &mut progress)? else {
            return Ok(Pass::Busy);
        };
        while !progress.stopped && retake(self.read_entries()?.len(), seen) {
            match self.locked_pass(now, ask, &mut progress)? {
                Some(len) => seen = len,
                None => break,
            }
        }
        Ok(Pass::Done(progress.report))
    }

    /// Holds `TAG` for the rounds. Returns the log length of the last read, or
    /// `None` when another pass holds the lock.
    fn locked_pass(
        &self,
        now: &dyn Fn() -> String,
        ask: &mut Ask,
        progress: &mut Progress,
    ) -> io::Result<Option<usize>> {
        let lock = self.open_lock("TAG")?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Error(e)) => return Err(e),
        }
        loop {
            let entries = self.read_entries()?;
            let state = self.read_state()?;
            progress.excluded.extend(context_exclusions(&state));
            if state.start.is_none() {
                self.append(&encode_start(entries.len(), &now()).map_err(io::Error::other)?)?;
                return Ok(Some(entries.len()));
            }
            let room = MAX_PER_PASS.saturating_sub(progress.attempted.len());
            let todo: Vec<usize> = untagged(&entries, &state)
                .into_iter()
                .filter(|id| !progress.attempted.contains(id))
                .take(BATCH_LADDER[0].min(room))
                .collect();
            if todo.is_empty() {
                return Ok(Some(entries.len()));
            }
            progress.attempted.extend(&todo);
            let round = Round {
                store: self,
                entries: &entries,
                state: &state,
                now,
            };
            round.resolve(&todo, BATCH_LADDER[0], ask, progress)?;
            if progress.stopped {
                return Ok(Some(entries.len()));
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
    /// lower. An id that still fails alone goes to the pass health, which says
    /// when it may get a `failed` row.
    fn resolve(
        &self,
        ids: &[usize],
        size: usize,
        ask: &mut Ask,
        progress: &mut Progress,
    ) -> io::Result<()> {
        let size = if ids.len() == 1 { 1 } else { size };
        let batch = build_batch(self.entries, ids, &progress.excluded);
        let reply = ask(&batch).map_err(io::Error::other)?;
        let (health, verdict) = after_call(std::mem::take(&mut progress.health), reply.exited_ok);
        progress.health = health;
        if verdict == Verdict::Stop {
            progress.stopped = true;
            return Ok(());
        }
        self.mark_confirmed(progress)?;
        let parsed = parse_reply(&reply.text, ids);
        let rows = parsed
            .tagged
            .iter()
            .map(|(n, flags)| {
                let role = self.entries.get(*n).map_or(Role::Assistant, |e| e.role);
                let flags = restrict_decision(role, *flags);
                encode_row(&self.row(*n, RowKind::Tagged(flags)))
            })
            .collect::<Result<String, String>>()
            .map_err(io::Error::other)?;
        if !rows.is_empty() {
            self.store.append(&rows)?;
            progress.report.tagged += parsed.tagged.len();
        }
        let Some(next) = next_size(size) else {
            for id in parsed.missing {
                progress.health =
                    isolated(std::mem::take(&mut progress.health), id, reply.exited_ok);
                progress.excluded.insert(id);
            }
            return self.mark_confirmed(progress);
        };
        if !parsed.missing.is_empty() {
            for chunk in batches(&parsed.missing, next) {
                self.resolve(&chunk, next, ask, progress)?;
                if progress.stopped {
                    break;
                }
            }
        }
        Ok(())
    }

    /// Writes the `failed` rows of the entries the pass health has confirmed.
    fn mark_confirmed(&self, progress: &mut Progress) -> io::Result<()> {
        let (health, confirmed) = take_confirmed(std::mem::take(&mut progress.health));
        progress.health = health;
        if confirmed.is_empty() {
            return Ok(());
        }
        self.mark_failed(&confirmed)?;
        progress.report.failed += confirmed.len();
        Ok(())
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
pub fn with_tags(dir: &Path, request: &SearchRequest) -> io::Result<SearchRequest> {
    match request.tag() {
        Some(label) => {
            let store = TagStore::new(dir.to_path_buf());
            let state = store.read_state()?;
            let entries = store.read_entries()?;
            Ok(request
                .clone()
                .with_tag_ids(ids_with(&state, &entries, label)))
        }
        None => Ok(request.clone()),
    }
}

#[cfg(test)]
#[path = "tag_store_tests.rs"]
mod tests;
