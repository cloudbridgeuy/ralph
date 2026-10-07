use crate::highlight::ThemeConfig;
use crate::human;
use crate::invoke::{build_command, CommandSpec, DEFAULT_PERMISSION_MODE};
use crate::orchestrator::display::{print_meeting_route, print_persona_banner};
use crate::paths;
use crate::spinner::SpinnerSessionInfo;
use crate::stream_processor::VerboseToolsConfig;
use crate::subprocess::{
    invoke_subprocess_with_spinner_config, SpinnerSubprocessConfig, StreamingSubprocessResult,
    SubprocessError,
};
use crate::warn::warn;
use chrono::Utc;
use ralph_core::meeting::{build_mcp_args, MeetingEvent, MeetingReply};
use ralph_core::meeting_transcript::{
    answer_line, end_line, kickoff_line, message_line, question_line, to_jsonl, TranscriptLine,
    HUMAN,
};
use ralph_core::meeting_turn::{
    parse_human_answer, scene_prompt, HumanAnswer, MeetingPlan, Reaction, Step, StopReason,
    TurnState, HUMAN_ENDED_REPLY, HUMAN_ENDED_SUMMARY,
};
use ralph_core::transcript::HumanResponse;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

type PendingEvent = (MeetingEvent, mpsc::Sender<MeetingReply>);

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const SPINNER_SETTLE: Duration = Duration::from_millis(300);

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
    pause: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
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
            pause: Some(Arc::clone(&self.pause)),
            cancel: Some(Arc::clone(&self.cancel)),
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

struct Transcript {
    path: PathBuf,
}

impl Transcript {
    fn create() -> io::Result<Self> {
        let slug = format!("meeting-{}", Utc::now().format("%Y%m%d-%H%M%S"));
        let dir = paths::session_dir(&slug);
        fs::create_dir_all(&dir)?;
        Ok(Self {
            path: dir.join("transcript.jsonl"),
        })
    }

    fn append(&self, line: &TranscriptLine) {
        let written = to_jsonl(line).map_err(io::Error::other).and_then(|out| {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?
                .write_all(out.as_bytes())
        });
        if let Err(e) = written {
            warn(format!("could not write transcript: {e}"));
        }
    }
}

struct TurnIo<'a> {
    events: &'a mpsc::Receiver<PendingEvent>,
    transcript: &'a Transcript,
    roster: &'a [String],
    pause: &'a AtomicBool,
    cancel: &'a AtomicBool,
}

fn ask_human(asker: &str, question: &str, io: &TurnIo) -> Option<HumanAnswer> {
    io.pause.store(true, Ordering::SeqCst);
    thread::sleep(SPINNER_SETTLE);
    let response = human::open_editor_for_meeting_ask(asker, question);
    io.pause.store(false, Ordering::SeqCst);
    match response {
        Ok(HumanResponse::Content(content)) => parse_human_answer(asker, io.roster, &content),
        Ok(HumanResponse::Abort) => None,
        Err(e) => Some(HumanAnswer {
            text: format!("Could not reach the human: {e}"),
            ..HumanAnswer::default()
        }),
    }
}

fn answer(state: &mut TurnState, event: &MeetingEvent, io: &TurnIo) -> MeetingReply {
    let speaker = state.speaker().to_string();
    match state.on_event(event) {
        Reaction::Reply(reply) => reply,
        Reaction::Sent { reply, to, text } => {
            io.transcript
                .append(&message_line(&speaker, &to, &text, Utc::now()));
            reply
        }
        Reaction::AskHuman(question) => {
            io.transcript
                .append(&question_line(&speaker, &question, Utc::now()));
            let Some(human_answer) = ask_human(&speaker, &question, io) else {
                state.end_by_human();
                io.cancel.store(true, Ordering::SeqCst);
                io.transcript
                    .append(&end_line(HUMAN, HUMAN_ENDED_SUMMARY, Utc::now()));
                return MeetingReply {
                    text: HUMAN_ENDED_REPLY.to_string(),
                };
            };
            for name in &human_answer.unknown {
                warn(format!("unknown persona '{name}' in the to: line; skipped"));
            }
            state.route_answer(&human_answer.extra, &question, &human_answer.text);
            let recipients = std::iter::once(speaker.as_str())
                .chain(human_answer.extra.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(", ");
            io.transcript
                .append(&answer_line(&recipients, &human_answer.text, Utc::now()));
            MeetingReply {
                text: human_answer.text,
            }
        }
    }
}

fn run_turn(
    config: SpinnerSubprocessConfig,
    state: &mut TurnState,
    io: &TurnIo,
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
            if let Ok((event, reply_tx)) = io.events.recv_timeout(POLL_INTERVAL) {
                let _ = reply_tx.send(answer(state, &event, io));
            }
        }
    })
}

pub fn run(config: &MeetingConfig) -> Result<StopReason, Box<dyn std::error::Error>> {
    let transcript = Transcript::create()?;
    transcript.append(&kickoff_line(
        &config.plan.first,
        &config.kickoff,
        Utc::now(),
    ));
    let dir = tempfile::tempdir()?;
    let socket = dir.path().join("meeting.sock");
    let (events_tx, events_rx) = mpsc::channel();
    let pause = Arc::new(AtomicBool::new(false));
    let cancel = Arc::new(AtomicBool::new(false));
    spawn_listener(UnixListener::bind(&socket)?, events_tx);

    let mut moderator = Moderator {
        config,
        exe: std::env::current_exe()?,
        socket,
        home: dirs::home_dir(),
        sessions: HashMap::new(),
        pause: Arc::clone(&pause),
        cancel: Arc::clone(&cancel),
    };
    let mut state = TurnState::new(&config.plan.first, config.plan.max_turns);
    let mut message = config.kickoff.clone();
    let io = TurnIo {
        events: &events_rx,
        transcript: &transcript,
        roster: &config.plan.roster,
        pause: &pause,
        cancel: &cancel,
    };

    loop {
        let speaker = state.speaker().to_string();
        print_persona_banner(&speaker);
        let turn_config = moderator.turn_config(&state, &message);
        let result = run_turn(turn_config, &mut state, &io)?;
        if let Some(session_id) = result.stream_result.metadata.session_id {
            moderator.sessions.insert(speaker.clone(), session_id);
        }
        match state.finish_turn(result.exit_code) {
            Step::Resume {
                persona,
                message: next,
            } => {
                print_meeting_route(&speaker, &persona, state.turn(), config.plan.max_turns);
                message = next;
            }
            Step::Stop(reason) => {
                if let StopReason::EndMeeting(summary) = &reason {
                    transcript.append(&end_line(&speaker, summary, Utc::now()));
                }
                println!("\nTranscript: {}", transcript.path.display());
                return Ok(reason);
            }
        }
    }
}
