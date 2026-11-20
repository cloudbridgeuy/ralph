use super::sink::SocketSink;
use crate::chat::assemble::{assemble, load_settings, AssembleOptions, Assembled, TurnOutput};
use crate::chat::session::load_custom_prompt;
use ralph_core::cancel::CancelToken;
use ralph_core::chat::ReplyImages;
use ralph_core::chat_memory::Trigger;
use ralph_core::chat_process::SessionKey;
use ralph_core::turn_event::TurnSink;
use ralph_server::chat::claim::ChatClaim;
use ralph_server::chat::fork::open_fork;
use ralph_server::chat::port::{SessionPort, TurnCall};
use ralph_server::chat::session::Session;
use ralph_server::chat::socket::serve::SocketGuard;
use ralph_server::chat::turn::TurnOutcome;
use ralph_server::subprocess::OutputGate;
use ralph_server::warn::warn;
use ralph_server::watches::CloseGuard;
use std::error::Error;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub(super) struct OpenSession {
    session: Session,
    pub(super) sink: Arc<SocketSink>,
    cancel: CancelToken,
    turning: Mutex<()>,
    _claim: Option<ChatClaim>,
    _guard: SocketGuard,
    _plugin: Option<tempfile::TempDir>,
    _close: CloseGuard,
}

impl OpenSession {
    pub(super) fn turn(&self, text: &str) -> Result<TurnOutcome, Box<dyn Error>> {
        let logged = [text.to_owned()];
        let call = TurnCall {
            trigger: Trigger::User,
            logged: &logged,
            sent: text,
        };
        let _turning = self
            .turning
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut ran = None;
        self.session
            .around(&mut || ran = Some(self.session.turn(&call, &self.cancel)));
        ran.ok_or("snapshots did not run the turn")?
    }

    pub(super) fn holding(mut self, claim: ChatClaim) -> Self {
        self._claim = Some(claim);
        self
    }

    pub(super) fn end(&self) {
        self.cancel.cancel();
        let _turning = self
            .turning
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.session.end_session();
    }
}

pub(super) fn open_session(
    store_dir: &Path,
    model_args: Vec<String>,
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
        plugin,
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
        output: TurnOutput {
            gate: Arc::new(OutputGate::new()),
            sink: turn_sink,
            reply_images: ReplyImages::Text,
            show_tools: false,
        },
        events: Arc::new(|_| {}),
    })?;
    Ok(OpenSession {
        session,
        sink,
        cancel: CancelToken::default(),
        turning: Mutex::new(()),
        _claim: None,
        _guard: guard,
        _plugin: plugin,
        _close: close,
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

    pub(super) fn remove(&self, key: &SessionKey) -> Option<Arc<OpenSession>> {
        let mut sessions = self.lock();
        let at = sessions.iter().position(|(open, _)| open == key)?;
        Some(sessions.remove(at).1)
    }
}
