use super::Store;
use crate::timing;
use crate::warn::warn;
use ralph_core::chat_memory::{
    call_facts, check_summary, fallback_summary, nap_call_row, nap_row, nap_source, nap_step,
    pending, retry_prompt, CallVerdict, CuratorError, Entry, NapCallFacts, NapStats, NapStep,
    NapTimes, Node, Tree, MAX_NAP_ATTEMPTS,
};
use std::io;
use std::path::Path;
use std::time::Instant;

struct Caller<'a, C> {
    dir: &'a Path,
    run: Option<&'a str>,
    curate: C,
    stats: NapStats,
}

impl<C: FnMut(&str) -> Result<String, CuratorError>> Caller<'_, C> {
    fn record(&mut self, call: &NapCallFacts) {
        self.stats = self.stats.with_call();
        timing::record(self.dir, self.run, |key| nap_call_row(key, call));
    }
}

impl Store {
    pub fn nap(
        &self,
        run: Option<&str>,
        curate: impl FnMut(&str) -> Result<String, CuratorError>,
    ) -> io::Result<()> {
        let began = Instant::now();
        let nap_lock = self.open_nap_lock()?;
        nap_lock.lock()?;
        let acquired = Instant::now();
        let (entries, tree) = self.load_settled(|| ())?;
        let todo = pending(&entries, &tree);
        let mut caller = Caller {
            dir: &self.dir,
            run,
            curate,
            stats: NapStats::start(u64::try_from(todo.len()).unwrap_or(u64::MAX)),
        };
        let done = self.nap_nodes(&entries, tree, &todo, &mut caller);
        let finished = Instant::now();
        let times = NapTimes {
            total: finished - began,
            lock_wait: acquired - began,
            lock_held: finished - acquired,
        };
        timing::record(&self.dir, run, |key| nap_row(key, caller.stats, times));
        done
    }

    fn nap_nodes<C: FnMut(&str) -> Result<String, CuratorError>>(
        &self,
        entries: &[Entry],
        mut tree: Tree,
        todo: &[Node],
        caller: &mut Caller<C>,
    ) -> io::Result<()> {
        for &node in todo {
            let base = match nap_step(entries, &tree, node) {
                NapStep::Done => continue,
                NapStep::Blocked(e) => {
                    warn(format!("chat memory skipped #{}: {e}", node.id()));
                    continue;
                }
                NapStep::Ask(prompt) => prompt,
            };
            match summarize(entries, &tree, node, base, caller) {
                Some(summary) => tree = self.commit(node, &summary)?,
                None => warn(format!("chat memory left #{} pending", node.id())),
            }
        }
        Ok(())
    }
}

fn summarize<C: FnMut(&str) -> Result<String, CuratorError>>(
    entries: &[Entry],
    tree: &Tree,
    node: Node,
    base: String,
    caller: &mut Caller<C>,
) -> Option<String> {
    let mut prompt = base.clone();
    for attempt in 1..=MAX_NAP_ATTEMPTS {
        let started = Instant::now();
        let reply = (caller.curate)(&prompt);
        let took = started.elapsed();
        let checked = reply.as_ref().map(|text| check_summary(text));
        let verdict = match &checked {
            Ok(Ok(_)) => CallVerdict::Accepted,
            Ok(Err(_)) => CallVerdict::Rejected,
            Err(error) => CallVerdict::Failed(error),
        };
        caller.record(&call_facts(
            node.id(),
            u64::try_from(attempt).unwrap_or(u64::MAX),
            took,
            verdict,
        ));
        match checked {
            Ok(Ok(summary)) => return Some(summary),
            Ok(Err(e)) => prompt = retry_prompt(&base, &e),
            Err(e) => {
                warn(format!("chat memory curator failed on #{}: {e}", node.id()));
                return None;
            }
        }
    }
    let source = nap_source(entries, tree, node).ok()?;
    warn(format!(
        "chat memory stored raw fallback for #{}",
        node.id()
    ));
    Some(fallback_summary(&source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::chat_memory::timing_path;
    use serde_json::Value;
    use std::fs;

    fn seeded(total: usize) -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let log: String = (0..total)
            .map(|i| {
                format!(
                    "{{\"role\":\"user\",\"at\":\"t\",\"text\":\"{}\"}}\n",
                    "y".repeat(600 + i)
                )
            })
            .collect();
        fs::write(dir.path().join("log.jsonl"), log).unwrap();
        let store = Store::new(dir.path().to_path_buf());
        (dir, store)
    }

    fn rows(dir: &Path) -> Vec<Value> {
        fs::read_to_string(timing_path(dir))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn a_failed_curator_call_writes_a_failed_row_and_a_nap_row() {
        let (dir, store) = seeded(1);
        store.nap(Some("r1"), |_| Err("down".into())).unwrap();
        let rows = rows(dir.path());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["event"], "nap_call");
        assert_eq!(rows[0]["outcome"], "failed");
        assert_eq!(rows[0]["attempt"], 1);
        assert_eq!(rows[0]["run"], "r1");
        assert_eq!(rows[1]["event"], "nap");
        assert_eq!(rows[1]["calls"], 1);
        assert_eq!(rows[1]["pending_at_start"], 1);
    }

    #[test]
    fn a_timed_out_call_writes_a_timeout_row_with_its_exit() {
        let (dir, store) = seeded(1);
        store
            .nap(None, |_| {
                Err(CuratorError {
                    message: "slow".into(),
                    timed_out: true,
                    exit: Some(-1),
                })
            })
            .unwrap();
        let rows = rows(dir.path());
        assert_eq!(
            (rows[0]["outcome"].as_str(), rows[0]["exit"].as_i64()),
            (Some("timeout"), Some(-1))
        );
    }

    #[test]
    fn a_rejected_reply_then_an_accepted_one_write_two_call_rows() {
        let (dir, store) = seeded(1);
        let mut calls = 0;
        store
            .nap(None, |_| {
                calls += 1;
                Ok(if calls == 1 {
                    "a\nb".to_string()
                } else {
                    "fine".to_string()
                })
            })
            .unwrap();
        let rows = rows(dir.path());
        let outcomes: Vec<_> = rows
            .iter()
            .filter_map(|row| row["outcome"].as_str())
            .collect();
        assert_eq!(outcomes, ["rejected", "ok"]);
        assert_eq!(rows[1]["attempt"], 2);
        assert_eq!(rows[2]["calls"], 2);
    }
}
