use crate::highlight::ThemeConfig;
use crate::human;
use crate::invoke::{build_command, CommandSpec, DEFAULT_PERMISSION_MODE};
use crate::spinner::SpinnerSessionInfo;
use crate::stream_processor::VerboseToolsConfig;
use crate::subprocess::{
    invoke_subprocess_with_spinner_config, SpinnerSubprocessConfig, StreamingSubprocessResult,
    SubprocessError,
};
use crate::warn::warn;
use ralph_core::meeting::{build_mcp_args, MeetingEvent, MeetingReply};
use ralph_core::meeting_turn::{scene_prompt, MeetingPlan, Reaction, Step, StopReason, TurnState};
use ralph_core::transcript::HumanResponse;
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

type PendingEvent = (MeetingEvent, mpsc::Sender<MeetingReply>);

const POLL_INTERVAL: Duration = Duration::from_millis(50);

pub struct MeetingConfig {
    pub plan: MeetingPlan,
    pub kickoff: String,
    pub timeout_secs: u64,
    pub theme_config: ThemeConfig,
}

struct Moderator<'a> {
    config: &'a MeetingConfig,
    exe: PathBuf,
    socket: PathBuf,
    home: Option<PathBuf>,
    sessions: HashMap<String, String>,
}

impl Moderator<'_> {
    fn turn_config(&self, state: &TurnState, message: &str) -> SpinnerSubprocessConfig {
        let plan = &self.config.plan;
        let speaker = state.speaker();
        let system_prompt = scene_prompt(&plan.roster, speaker, &plan.first, &self.config.kickoff);
        let mcp_args = build_mcp_args(&self.exe, &self.socket, speaker, &plan.roster);
        let command = build_command(&CommandSpec {
            resume: self.sessions.get(speaker).map(String::as_str),
            model: plan.models.get(speaker).map(String::as_str),
            system_prompt: Some(&system_prompt),
            extra_args: &mcp_args,
            ..CommandSpec::new(
                message,
                DEFAULT_PERMISSION_MODE,
                Some(speaker),
                self.home.as_deref(),
            )
        });
        SpinnerSubprocessConfig {
            command,
            timeout_secs: self.config.timeout_secs,
            theme_config: self.config.theme_config.clone(),
            session_elapsed_ms: 0,
            verbose_tools: VerboseToolsConfig::new(),
            session_info: SpinnerSessionInfo {
                persona: Some(speaker.to_string()),
                slug: None,
                current_iteration: Some(state.turn()),
                max_iterations: Some(plan.max_turns),
                on_behalf_of: None,
            },
        }
    }
}

fn serve_connection(stream: UnixStream, events: &mpsc::Sender<PendingEvent>) -> io::Result<()> {
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    let event: MeetingEvent = serde_json::from_str(&line).map_err(io::Error::other)?;
    let (reply_tx, reply_rx) = mpsc::channel();
    events.send((event, reply_tx)).map_err(io::Error::other)?;
    let reply = reply_rx.recv().map_err(io::Error::other)?;
    let mut out = serde_json::to_string(&reply).map_err(io::Error::other)?;
    out.push('\n');
    (&stream).write_all(out.as_bytes())
}

fn spawn_listener(listener: UnixListener, events: mpsc::Sender<PendingEvent>) {
    thread::spawn(move || {
        for stream in listener.incoming().map_while(Result::ok) {
            let events = events.clone();
            thread::spawn(move || {
                if let Err(e) = serve_connection(stream, &events) {
                    warn(format!("meeting connection failed: {e}"));
                }
            });
        }
    });
}

fn ask_human(question: &str) -> MeetingReply {
    let text = match human::open_editor_for_ask(question) {
        Ok(HumanResponse::Content(answer)) => answer,
        Ok(HumanResponse::Abort) => "The human gave no answer.".to_string(),
        Err(e) => format!("Could not reach the human: {e}"),
    };
    MeetingReply { text }
}

fn answer(state: &mut TurnState, event: &MeetingEvent) -> MeetingReply {
    match state.on_event(event) {
        Reaction::Reply(reply) => reply,
        Reaction::AskHuman(question) => ask_human(&question),
    }
}

fn run_turn(
    config: SpinnerSubprocessConfig,
    state: &mut TurnState,
    events: &mpsc::Receiver<PendingEvent>,
) -> Result<StreamingSubprocessResult, SubprocessError> {
    thread::scope(|scope| {
        let (done_tx, done_rx) = mpsc::channel();
        scope.spawn(move || {
            let _ = done_tx.send(invoke_subprocess_with_spinner_config(&config));
        });
        loop {
            match done_rx.try_recv() {
                Ok(result) => return result,
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(SubprocessError::OutputCaptureFailed(
                        "subprocess thread ended without a result".to_string(),
                    ))
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            if let Ok((event, reply_tx)) = events.recv_timeout(POLL_INTERVAL) {
                let _ = reply_tx.send(answer(state, &event));
            }
        }
    })
}

pub fn run(config: &MeetingConfig) -> Result<StopReason, Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let socket = dir.path().join("meeting.sock");
    let (events_tx, events_rx) = mpsc::channel();
    spawn_listener(UnixListener::bind(&socket)?, events_tx);

    let mut moderator = Moderator {
        config,
        exe: std::env::current_exe()?,
        socket,
        home: dirs::home_dir(),
        sessions: HashMap::new(),
    };
    let mut state = TurnState::new(&config.plan.first, config.plan.max_turns);
    let mut message = config.kickoff.clone();

    loop {
        let speaker = state.speaker().to_string();
        let turn_config = moderator.turn_config(&state, &message);
        let result = run_turn(turn_config, &mut state, &events_rx)?;
        if let Some(session_id) = result.stream_result.metadata.session_id {
            moderator.sessions.insert(speaker.clone(), session_id);
        }
        match state.finish_turn(result.exit_code) {
            Step::Resume {
                persona,
                message: next,
            } => {
                println!("\n{speaker} -> {persona}");
                message = next;
            }
            Step::Stop(reason) => return Ok(reason),
        }
    }
}
