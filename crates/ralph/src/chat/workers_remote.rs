use crate::config::{PrProvider, WorkersSection};
use ralph_core::worker::Ticket;
use ralph_core::workers_view::{
    fetched_of, lookup_command, ticket_lookup, wanted_keys, Fetched, LookupKey, Pr, Remote,
    RemoteCache, TicketRemote,
};
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const POLL_INTERVAL: Duration = Duration::from_millis(10);

pub struct RemoteStatus {
    cache: Arc<Mutex<RemoteCache>>,
    section: WorkersSection,
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

fn locked(cache: &Mutex<RemoteCache>) -> MutexGuard<'_, RemoteCache> {
    cache.lock().unwrap_or_else(PoisonError::into_inner)
}

fn timed_output(program: &str, args: &[String], timeout: Duration) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = thread::spawn(move || {
        let mut buf = Vec::new();
        stdout.read_to_end(&mut buf).ok().map(|_| buf)
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL_INTERVAL),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let output = reader.join().ok()??;
    if !status.success() {
        return None;
    }
    String::from_utf8(output).ok()
}

fn fetch(key: &LookupKey, timeout: Duration) -> Fetched {
    let command = lookup_command(key);
    let output = timed_output(command.program, &command.args, timeout);
    fetched_of(key, output.as_deref())
}

impl RemoteStatus {
    pub fn new(section: WorkersSection) -> Self {
        Self {
            cache: Arc::new(Mutex::new(RemoteCache::default())),
            section,
        }
    }

    fn start(&self, key: LookupKey) {
        let cache = Arc::clone(&self.cache);
        let timeout = Duration::from_millis(self.section.lookup_timeout_ms);
        let job_key = key.clone();
        let spawned = thread::Builder::new().spawn(move || {
            let fetched = fetch(&job_key, timeout);
            locked(&cache).store(job_key, now_secs(), fetched);
        });
        if spawned.is_err() {
            locked(&self.cache).store(key, now_secs(), Fetched::Failed);
        }
    }

    pub fn cells(&self, id: &str, ticket: Option<&Ticket>) -> RemoteCells {
        let github = self.section.pr_provider == PrProvider::Github;
        let wanted = wanted_keys(id, ticket, github);
        let ttl = self.section.cache_secs;
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
        start.into_iter().for_each(|key| self.start(key));
        cells
    }
}
