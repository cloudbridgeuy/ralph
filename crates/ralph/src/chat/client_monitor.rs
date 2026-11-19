use crate::chat::term_status::TermStatus;
use crate::inbox_alert;
use crate::signal;
use ralph_core::chat_input::{input_lost_at_prompt, InputWatch};
use ralph_core::chat_keepalive::lock_wait_continues;
use ralph_core::inbox_alert::{inbox_alert, AlertConfig};
use ralph_core::worker::{Report, HANDOFF_POLL, INBOX_DIR};
use ralph_core::worker_check::check_every_polls;
use ralph_server::chat::host::HostEvent;
use ralph_server::chat::keepalive::PING_TIMEOUT;
use ralph_server::chat::monitor::{Monitor as Core, POLL};
use ralph_server::chat::schedule::fire_due;
use ralph_server::chat::schedule_worker::tick_workers;
use ralph_server::subprocess::Session;
use ralph_server::version_drift::DailyTick;
use ralph_server::watch_requests;
use ralph_server::workers::headless::{ExitCleanup, HeadlessShutdown};
use std::io::{self, IsTerminal};
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, TryLockError};
use std::time::{Duration, Instant};

const LOCK_POLL: Duration = Duration::from_millis(100);
const MEMORY_EVERY_POLLS: u64 = 10;
const INPUT_LOST_EXIT: i32 = 129;

pub(super) struct Monitor {
    pub(super) core: Core,
    pub(super) alert: AlertConfig,
    pub(super) term: Arc<TermStatus>,
}

impl Deref for Monitor {
    type Target = Core;

    fn deref(&self) -> &Core {
        &self.core
    }
}

pub(super) fn client_sink(
    monitor: Arc<Monitor>,
    repaint: impl Fn() + Send + 'static,
) -> impl Fn(HostEvent) + Send + 'static {
    move |event| match event {
        HostEvent::Repaint => repaint(),
        HostEvent::Status(report) => monitor.term.send(&report),
        HostEvent::Fresh { fresh, unread } => {
            if !monitor.alert.enabled() {
                return;
            }
            let terminal = monitor.terminal_report(unread).state;
            let fresh: Vec<&Report> = fresh.iter().collect();
            if let Some(alert) = inbox_alert(&fresh, inbox_alert::focus(), terminal, &monitor.alert)
            {
                inbox_alert::deliver(&alert);
            }
        }
    }
}

pub(super) fn watch_status(monitor: Arc<Monitor>, sink: impl Fn(HostEvent) + Send + 'static) {
    std::thread::spawn(move || {
        let check_every = monitor
            .worker_check_secs
            .map(|secs| check_every_polls(secs, POLL));
        let mut tick: u64 = 0;
        let mut daily = DailyTick::start();
        let inbox_dir = monitor.store_dir.join(INBOX_DIR);
        loop {
            std::thread::sleep(POLL);
            tick += 1;
            daily.tick(&inbox_dir);
            watch_requests::start_queued(&monitor.watches, &monitor.inbox, &monitor.store_dir);
            fire_due(&monitor.schedule, &monitor.watches, &monitor.inbox);
            tick_workers(&monitor);
            let changed = monitor.refresh(tick.is_multiple_of(MEMORY_EVERY_POLLS));
            for event in monitor.tick_events(changed) {
                sink(event);
            }
            if check_every.is_some_and(|every| tick.is_multiple_of(every)) {
                monitor.check();
            }
        }
    });
}

pub(super) fn watch_headless(store_dir: PathBuf) -> HeadlessShutdown {
    let headless = HeadlessShutdown::new(store_dir);
    for host in headless.hosts() {
        let resumed = Arc::clone(host);
        std::thread::spawn(move || resumed.resume());
        let polled = Arc::clone(host);
        // Keep each host independent: HTTP and ACP handoffs can block, and neither
        // should delay the other host or the status/check loop.
        std::thread::spawn(move || loop {
            std::thread::sleep(HANDOFF_POLL);
            if !polled.poll() {
                break;
            }
        });
    }
    headless
}

pub(super) fn stdin_hung_up() -> bool {
    let mut stdin = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: ralph's direct dependencies have no safe poll wrapper, and this is
    // the backup check for terminals that close without sending SIGHUP.
    // `stdin` is a valid, initialized pollfd on the stack and nfds is 1, so poll
    // only touches that one struct. fd 0 is not owned or closed here. A zero
    // timeout means poll never blocks. revents is read only when poll returns > 0.
    unsafe { libc::poll(&mut stdin, 1, 0) > 0 && stdin.revents & libc::POLLHUP != 0 }
}

fn drop_session(session: &Mutex<Option<Session>>) {
    let started = Instant::now();
    loop {
        match session.try_lock() {
            Ok(mut live) => {
                live.take();
                return;
            }
            Err(TryLockError::Poisoned(poisoned)) => {
                poisoned.into_inner().take();
                return;
            }
            Err(TryLockError::WouldBlock) => {
                if !lock_wait_continues(started.elapsed(), PING_TIMEOUT) {
                    return;
                }
                std::thread::sleep(LOCK_POLL);
            }
        }
    }
}

pub(super) fn watch_input(
    at_prompt: Arc<AtomicBool>,
    session: Arc<Mutex<Option<Session>>>,
    plugin_dir: Option<PathBuf>,
    exit: ExitCleanup,
    term: Arc<TermStatus>,
) {
    let started_on_terminal = io::stdin().is_terminal();
    std::thread::spawn(move || loop {
        std::thread::sleep(POLL);
        if input_lost_at_prompt(&InputWatch {
            hangup_signal: signal::is_terminated(),
            started_on_terminal,
            stdin_hung_up: stdin_hung_up(),
            at_prompt: at_prompt.load(Ordering::SeqCst),
        }) {
            drop_session(&session);
            if let Some(dir) = &plugin_dir {
                std::fs::remove_dir_all(dir).ok();
            }
            term.clear();
            exit.run();
            std::process::exit(INPUT_LOST_EXIT);
        }
    });
}
