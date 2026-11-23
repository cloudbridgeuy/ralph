use super::clients::Clients;
use super::sink::{inbox_sink, SocketSink};
use crate::chat::assemble::{assemble, load_settings, AssembleOptions, Assembled, TurnOutput};
use crate::chat::client_monitor::watch_headless;
use crate::chat::session::load_custom_prompt;
use ralph_core::cancel::CancelToken;
use ralph_core::chat::ReplyImages;
use ralph_core::chat_memory::Trigger;
use ralph_core::chat_process::SessionKey;
use ralph_core::turn_event::TurnSink;
use ralph_server::chat::claim::ChatClaim;
use ralph_server::chat::fork::open_fork;
use ralph_server::chat::host::{spawn_session_host, HostGuard};
use ralph_server::chat::port::{SessionPort, TurnCall};
use ralph_server::chat::serve_proto::{status_frame, ServerMsg};
use ralph_server::chat::session::Session;
use ralph_server::chat::socket::serve::SocketGuard;
use ralph_server::chat::turn::TurnOutcome;
use ralph_server::subprocess::OutputGate;
use ralph_server::warn::warn;
use ralph_server::watches::CloseGuard;
use ralph_server::workers::headless::HeadlessShutdown;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub(super) struct OpenSession {
    maintenance: Mutex<Option<HostGuard>>,
    idle: Arc<AtomicBool>,
    session: Session,
    store: PathBuf,
    pub(super) sink: Arc<SocketSink>,
    cancel: Mutex<CancelToken>,
    _claim: Option<ChatClaim>,
    _guard: SocketGuard,
    _close: CloseGuard,
    _headless: HeadlessShutdown,
}

impl OpenSession {
    pub(super) fn turn(&self, text: &str) -> Result<TurnOutcome, Box<dyn Error>> {
        let logged = [text.to_owned()];
        let call = TurnCall {
            trigger: Trigger::User,
            logged: &logged,
            sent: text,
        };
        let maintenance = self
            .maintenance
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if maintenance.is_none() {
            return Err("session is closed".into());
        }
        self.idle.store(false, Ordering::SeqCst);
        let cancel = CancelToken::default();
        *self.current_cancel() = cancel.clone();
        let mut ran = None;
        self.session
            .around(&mut || ran = Some(self.session.turn(&call, &cancel)));
        self.idle.store(true, Ordering::SeqCst);
        ran.ok_or("snapshots did not run the turn")?
    }

    pub(super) fn holding(mut self, claim: ChatClaim) -> Self {
        self._claim = Some(claim);
        self
    }

    fn current_cancel(&self) -> std::sync::MutexGuard<'_, CancelToken> {
        self.cancel
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn cancel(&self) {
        self.current_cancel().cancel();
    }

    pub(super) fn status(&self) -> ServerMsg {
        status_frame(&self.session.status(), self.session.turns())
    }

    pub(super) fn end(&self) {
        self.cancel();
        let mut maintenance = self
            .maintenance
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.idle.store(false, Ordering::SeqCst);
        drop(maintenance.take());
        self.session.end_session();
    }
}

fn start_hosts<H, M>(
    key: &SessionKey,
    headless: impl FnOnce() -> H,
    maintenance: impl FnOnce(bool) -> M,
) -> (H, M) {
    (headless(), maintenance(*key == SessionKey::Main))
}

pub(super) fn open_session(
    key: &SessionKey,
    store_dir: &Path,
    model_args: Vec<String>,
    clients: Arc<Clients>,
) -> Result<OpenSession, Box<dyn Error>> {
    let settings = load_settings(store_dir, None, None)?;
    let files = settings
        .loaded
        .config
        .chat
        .prompt_files(dirs::home_dir().as_deref());
    let (custom_prompt, warnings) = load_custom_prompt(&files);
    warnings.iter().for_each(warn);
    let sink = Arc::new(SocketSink::default());
    let turn_sink: Arc<dyn TurnSink> = sink.clone();
    let Assembled {
        session,
        guard,
        close,
        ..
    } = assemble(AssembleOptions {
        store_dir,
        store_root: store_dir.to_path_buf(),
        settings: &settings,
        fork: open_fork(store_dir)?,
        model_args,
        skills: &[],
        worktree_dir: None,
        custom_prompt,
        output: serve_output(turn_sink),
        events: Arc::new(|_| {}),
    })?;
    let idle = Arc::new(AtomicBool::new(true));
    let (headless, maintenance) = start_hosts(
        key,
        || watch_headless(store_dir.to_path_buf()),
        |daily_drift| {
            session.refresh();
            spawn_session_host(
                &session,
                settings.tuning.keepalive_hours,
                Arc::clone(&idle),
                daily_drift,
                inbox_sink(key.clone(), move |msg| clients.push(&msg)),
            )
        },
    );
    Ok(OpenSession {
        maintenance: Mutex::new(Some(maintenance)),
        idle,
        session,
        store: store_dir.to_path_buf(),
        sink,
        cancel: Mutex::new(CancelToken::default()),
        _claim: None,
        _guard: guard,
        _close: close,
        _headless: headless,
    })
}

#[derive(Default)]
pub(super) struct Sessions(Mutex<Vec<(SessionKey, Arc<OpenSession>)>>);

impl Sessions {
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<(SessionKey, Arc<OpenSession>)>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn insert(&self, key: SessionKey, session: OpenSession) {
        self.lock().push((key, Arc::new(session)));
    }

    pub(super) fn get(&self, key: &SessionKey) -> Option<Arc<OpenSession>> {
        self.lock()
            .iter()
            .find(|(open, _)| open == key)
            .map(|(_, session)| Arc::clone(session))
    }

    pub(super) fn store(&self, key: &SessionKey) -> Option<PathBuf> {
        self.get(key).map(|session| session.store.clone())
    }

    pub(super) fn stores(&self) -> Vec<(SessionKey, PathBuf)> {
        self.lock()
            .iter()
            .map(|(key, session)| (key.clone(), session.store.clone()))
            .collect()
    }

    pub(super) fn remove(&self, key: &SessionKey) -> Option<Arc<OpenSession>> {
        let mut sessions = self.lock();
        let at = sessions.iter().position(|(open, _)| open == key)?;
        Some(sessions.remove(at).1)
    }
}

fn serve_output(sink: Arc<dyn TurnSink>) -> TurnOutput {
    TurnOutput {
        gate: Arc::new(OutputGate::new()),
        sink,
        reply_images: ReplyImages::Text,
        show_tools: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::chat_thread::ThreadName;
    use std::cell::RefCell;

    #[test]
    fn serve_output_keeps_tool_blocks_for_remote_clients() {
        assert!(serve_output(Arc::new(SocketSink::default())).show_tools);
    }

    #[test]
    fn main_and_thread_start_maintenance_after_headless_ownership() {
        for (key, daily) in [
            (SessionKey::Main, true),
            (
                SessionKey::Thread(ThreadName::parse("side").unwrap()),
                false,
            ),
        ] {
            let starts = RefCell::new(Vec::new());
            start_hosts(
                &key,
                || starts.borrow_mut().push("headless"),
                |daily_drift| {
                    assert_eq!(daily_drift, daily);
                    starts.borrow_mut().push("maintenance");
                },
            );
            assert_eq!(starts.into_inner(), ["headless", "maintenance"], "{key}");
        }
    }
}
