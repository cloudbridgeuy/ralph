use crate::chat::term_status::TermStatus;
use crate::inbox_alert;
use crate::signal;
use ralph_core::chat_input::{input_lost_at_prompt, InputWatch};
use ralph_core::chat_keepalive::lock_wait_continues;
use ralph_core::chat_send_it::{blocked_alert, blocked_report};
use ralph_core::inbox_alert::{inbox_alert, AlertConfig};
use ralph_core::worker::{Report, HANDOFF_POLL};
use ralph_server::chat::host::HostEvent;
use ralph_server::chat::keepalive::PING_TIMEOUT;
use ralph_server::chat::monitor::{Monitor as Core, POLL};
use ralph_server::chat::session::{Emit, Event};
use ralph_server::subprocess::Session;
use ralph_server::warn::warn_if_err;
use ralph_server::workers::headless::{ExitCleanup, HeadlessShutdown};
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, TryLockError};
use std::time::{Duration, Instant};

const LOCK_POLL: Duration = Duration::from_millis(100);
const INPUT_LOST_EXIT: i32 = 129;

pub(super) fn client_sink(
    monitor: Arc<Core>,
    alert: AlertConfig,
    term: Arc<TermStatus>,
    repaint: impl Fn() + Send + 'static,
) -> impl Fn(HostEvent) + Send + 'static {
    move |event| match event {
        HostEvent::Repaint => repaint(),
        HostEvent::Status(report) => term.send(&report),
        HostEvent::Fresh { fresh, unread } => {
            if !alert.enabled() {
                return;
            }
            let terminal = monitor.terminal_report(unread).state;
            let fresh: Vec<&Report> = fresh.iter().collect();
            if let Some(alert) = inbox_alert(&fresh, inbox_alert::focus(), terminal, &alert) {
                inbox_alert::deliver(&alert);
            }
        }
    }
}

fn print_line(line: &str) {
    let mut out = io::stdout().lock();
    warn_if_err(
        writeln!(out, "{line}").and_then(|()| out.flush()),
        "Failed to print the send-it line",
    );
}

pub(super) fn client_events(alert: AlertConfig, term: Arc<TermStatus>) -> Emit {
    Arc::new(move |event| match event {
        Event::Notice(line) => print_line(&line),
        Event::Blocked(notice) => {
            if let Some(alert) = blocked_alert(&notice, &alert) {
                inbox_alert::deliver(&alert);
            }
            term.send(&blocked_report(&notice));
        }
    })
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
