use crate::config::{PrProvider, WorkersSection};
use crate::subprocess::{timed_output, TimedOutput};
use crate::watches::locked;
use ralph_core::worker::Ticket;
use ralph_core::workers_view::{
    fetched_of, lookup_command, ticket_lookup, wanted_keys, Fetched, LookupKey, Pr, Remote,
    RemoteCache, TicketRemote,
};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct RemoteStatus {
    cache: Arc<Mutex<RemoteCache>>,
}

pub struct RemoteCells {
    pub ticket: Option<TicketRemote>,
    pub pr: Remote<Pr>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

struct StoreGuard {
    cache: Arc<Mutex<RemoteCache>>,
    key: Option<LookupKey>,
}

impl StoreGuard {
    fn finish(mut self, fetched: Fetched) {
        if let Some(key) = self.key.take() {
            locked(&self.cache).store(key, now_secs(), fetched);
        }
    }
}

impl Drop for StoreGuard {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            locked(&self.cache).store(key, now_secs(), Fetched::Failed);
        }
    }
}

fn fetch(key: &LookupKey, timeout: Duration) -> Fetched {
    let command = lookup_command(key);
    let output = match timed_output(Command::new(command.program).args(&command.args), timeout) {
        TimedOutput::Finished {
            success: true,
            stdout,
        } => String::from_utf8(stdout).ok(),
        TimedOutput::Finished { .. } | TimedOutput::TimedOut | TimedOutput::Failed(_) => None,
    };
    fetched_of(key, output.as_deref())
}

impl RemoteStatus {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(RemoteCache::default())),
        }
    }

    fn start(&self, key: LookupKey, timeout: Duration) {
        let guard = StoreGuard {
            cache: Arc::clone(&self.cache),
            key: Some(key.clone()),
        };
        let job_key = key.clone();
        let spawned = thread::Builder::new().spawn(move || {
            let fetched = fetch(&job_key, timeout);
            guard.finish(fetched);
        });
        if spawned.is_err() {
            locked(&self.cache).store(key, now_secs(), Fetched::Failed);
        }
    }

    pub fn cells(
        &self,
        section: &WorkersSection,
        id: &str,
        ticket: Option<&Ticket>,
    ) -> RemoteCells {
        let github = section.pr_provider == PrProvider::Github;
        let wanted = wanted_keys(id, ticket, github);
        let ttl = section.cache_secs;
        let timeout = Duration::from_millis(section.lookup_timeout_ms);
        let now = now_secs();
        let (start, cells) = {
            let mut cache = locked(&self.cache);
            let start = cache.to_start(&wanted, now, ttl);
            cache.mark_in_flight(&start);
            let ticket = ticket.and_then(|ticket| {
                let key = ticket_lookup(ticket)?;
                Some(TicketRemote {
                    key: ticket.key.clone(),
                    state: cache.ticket_state(&key, now, ttl),
                })
            });
            let pr = if github {
                cache.pr_state(&LookupKey::Pr(id.to_string()), now, ttl)
            } else {
                Remote::None
            };
            (start, RemoteCells { ticket, pr })
        };
        start.into_iter().for_each(|key| self.start(key, timeout));
        cells
    }
}
