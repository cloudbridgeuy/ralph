use crate::acp_worker::CursorHost;
use crate::chat::socket::SocketFiles;
use crate::cursor_owner::{now_secs, write_file};
use crate::opencode_worker::OpencodeHost;
use crate::watches::Watches;
use ralph_core::worker::ActivityMark;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

/// A chat's host, not an individual worker. One host may own many workers.
pub(crate) trait HeadlessHost: Send + Sync {
    /// Resume persisted workers or supervise shared resources on a dedicated thread.
    fn resume(&self);

    /// Process queued worker requests. False ends the poller after chat shutdown.
    fn poll(&self) -> bool;

    /// Stop the entire host at chat exit, never when one worker closes or reports.
    fn shutdown(&self);
}

pub(crate) struct Heartbeat {
    activity: PathBuf,
    gone: PathBuf,
    last: Mutex<ActivityMark>,
}

impl Heartbeat {
    pub(crate) fn new(activity: PathBuf, gone: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            activity,
            gone,
            last: Mutex::new(ActivityMark {
                at: 0,
                running: false,
            }),
        })
    }

    pub(crate) fn clear(&self) {
        fs::remove_file(&self.activity).ok();
        fs::remove_file(&self.gone).ok();
        *self.last.lock().unwrap_or_else(PoisonError::into_inner) = ActivityMark {
            at: 0,
            running: false,
        };
    }

    pub(crate) fn touch(&self) {
        self.write(None);
    }

    pub(crate) fn set_running(&self, running: bool) {
        self.write(Some(running));
    }

    pub(crate) fn gone(&self, reason: &str) {
        write_file(&self.gone, reason).ok();
    }

    fn write(&self, running: Option<bool>) {
        let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
        let next = ActivityMark {
            at: now_secs(),
            running: running.unwrap_or(last.running),
        };
        if next != *last {
            write_file(&self.activity, &next.render()).ok();
            *last = next;
        }
    }
}

/// Owns host lifetimes for a chat, including early returns from the chat loop.
pub(crate) struct HeadlessShutdown {
    hosts: Vec<Arc<dyn HeadlessHost>>,
}

impl HeadlessShutdown {
    pub(crate) fn new(store_dir: PathBuf) -> Self {
        Self {
            hosts: vec![
                CursorHost::new(store_dir.clone()),
                OpencodeHost::new(store_dir),
            ],
        }
    }

    pub(crate) fn hosts(&self) -> &[Arc<dyn HeadlessHost>] {
        &self.hosts
    }
}

impl Drop for HeadlessShutdown {
    fn drop(&mut self) {
        // Match the previous guards' reverse construction/drop order.
        for host in self.hosts.iter().rev() {
            host.shutdown();
        }
    }
}

pub(crate) struct ExitCleanup {
    watches: Watches,
    hosts: Vec<Arc<dyn HeadlessHost>>,
    socket: SocketFiles,
}

impl ExitCleanup {
    pub(crate) fn new(watches: Watches, headless: &HeadlessShutdown, socket: SocketFiles) -> Self {
        Self {
            watches,
            hosts: headless.hosts.clone(),
            socket,
        }
    }

    pub(crate) fn run(&self) {
        self.watches.kill_all();
        for host in &self.hosts {
            host.shutdown();
        }
        self.socket.remove();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct FakeHost {
        resumed: AtomicUsize,
        polls: AtomicUsize,
        shutdowns: AtomicUsize,
    }

    impl HeadlessHost for FakeHost {
        fn resume(&self) {
            self.resumed.fetch_add(1, Ordering::SeqCst);
        }

        fn poll(&self) -> bool {
            self.polls.fetch_add(1, Ordering::SeqCst);
            self.shutdowns.load(Ordering::SeqCst) == 0
        }

        fn shutdown(&self) {
            self.shutdowns.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn hosts_are_polled_through_the_trait_and_shutdown_only_by_the_exit_guard() {
        let cursor = Arc::new(FakeHost::default());
        let opencode = Arc::new(FakeHost::default());
        let guard = HeadlessShutdown {
            hosts: vec![cursor.clone(), opencode.clone()],
        };
        for host in guard.hosts() {
            host.resume();
            assert!(host.poll());
            assert!(host.poll());
        }
        for host in [&cursor, &opencode] {
            assert_eq!(host.resumed.load(Ordering::SeqCst), 1);
            assert_eq!(host.polls.load(Ordering::SeqCst), 2);
            assert_eq!(host.shutdowns.load(Ordering::SeqCst), 0);
        }
        drop(guard);
        for host in [cursor, opencode] {
            assert_eq!(host.shutdowns.load(Ordering::SeqCst), 1);
            assert!(!host.poll());
        }
    }
}
