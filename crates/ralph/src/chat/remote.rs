use crate::chat::prompt::{retry_interrupted, screen_columns};
use crate::chat::queue::Queue;
use crate::chat::switch::ChatExit;
use crate::chat::term_loop::run_turn;
use crate::highlight::ThemeConfig;
use crate::signal;
use crate::turn_sink::TerminalSink;
use ralph_core::cancel::CancelToken;
use ralph_core::chat_answer::AnswerArgs;
use ralph_core::chat_memory::Trigger;
use ralph_core::chat_send_it::SendItCommand;
use ralph_core::chat_status::{route_input, EmptySubmit, Route, Status};
use ralph_core::chat_switch::SwitchTarget;
use ralph_core::chat_thread_cmd::ThreadOp;
use ralph_core::program_status::{Phase, State};
use ralph_core::turn_event::{TurnEvent, TurnSink};
use ralph_core::worker::{BlockedContent, Report};
use ralph_server::chat::ask::Ask;
use ralph_server::chat::port::{SessionPort, TurnCall};
use ralph_server::chat::send_it::Screen;
use ralph_server::chat::serve_proto::{
    decode_frame, encode_frame, ClientMsg, ServerMsg, ThreadCmd, VERSION,
};
use ralph_server::chat::session::{At, Proposal, Reply};
use ralph_server::chat::turn::TurnOutcome;
use ralph_server::warn::{notice, warn_if_err};
use reedline::{DefaultPrompt, Reedline, Signal};
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::io::{self, BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

const POLL: Duration = Duration::from_millis(100);

#[derive(Debug, PartialEq)]
enum Step {
    Render(TurnEvent),
    Finished,
    Failed(String),
    Notice(String),
    Ignored,
}

fn on_frame(msg: ServerMsg) -> Step {
    match msg {
        ServerMsg::Turn { event } => Step::Render(event),
        ServerMsg::TurnDone => Step::Finished,
        ServerMsg::Error { message } => Step::Failed(message),
        ServerMsg::Blocked {
            thread,
            worker,
            content,
        } => Step::Notice(blocked_notice(thread.as_deref(), &worker, content.as_ref())),
        ServerMsg::Hello { .. } | ServerMsg::Done { .. } | ServerMsg::Status { .. } => {
            Step::Ignored
        }
    }
}

fn blocked_notice(thread: Option<&str>, worker: &str, content: Option<&BlockedContent>) -> String {
    let label = thread.map_or_else(|| worker.to_string(), |name| format!("{name}/{worker}"));
    match content {
        Some(content) => {
            let options: Vec<&str> = content
                .options
                .iter()
                .map(|option| option.option_id.as_str())
                .collect();
            format!(
                "{label} blocked: {} {} request {} options {}",
                content.tool,
                content.summary,
                content.request_id,
                options.join("|")
            )
        }
        None => format!("{label} permission settled"),
    }
}

#[derive(Debug)]
struct Refused(String);

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for Refused {}

fn keeps_chat_open(err: &(dyn Error + 'static)) -> bool {
    err.is::<Refused>()
}

fn state_label(state: State) -> &'static str {
    match state {
        State::Idle => "idle",
        State::Working => "working",
        State::Done => "done",
        State::Blocked(_) => "blocked",
        State::Clear => "clear",
    }
}

fn status_summary(
    attached: Option<&str>,
    state: State,
    unread: usize,
    blocked: &[String],
) -> String {
    let mut line = format!(
        "{}: {}, {unread} unread",
        attached.unwrap_or("main"),
        state_label(state)
    );
    if !blocked.is_empty() {
        line.push_str(&format!(", blocked: {}", blocked.join(", ")));
    }
    line
}

fn thread_request(route: &Route) -> Option<Result<ThreadCmd, String>> {
    let cmd = |cmd| Some(Ok(cmd));
    match route {
        Route::Switch { parsed, .. } => match parsed {
            Ok(SwitchTarget::Main) => cmd(ThreadCmd::Main),
            Ok(SwitchTarget::Thread(name)) => cmd(ThreadCmd::Switch {
                name: name.to_string(),
            }),
            Err(usage) => Some(Err(usage.clone())),
        },
        Route::ThreadOp { op, .. } => match op {
            Ok(ThreadOp::Ls) => Some(Err("/thread ls is not served over --connect".into())),
            Ok(ThreadOp::Open(opts)) => cmd(ThreadCmd::Open {
                name: opts.name.to_string(),
                model: opts.model.clone(),
                auto: opts.auto,
            }),
            Ok(ThreadOp::Pull(name)) => cmd(ThreadCmd::Pull {
                name: name.to_string(),
            }),
            Ok(ThreadOp::Park(name)) => cmd(ThreadCmd::Park {
                name: name.to_string(),
            }),
            Ok(ThreadOp::Revive(name)) => cmd(ThreadCmd::Revive {
                name: name.to_string(),
            }),
            Ok(ThreadOp::Drop(name)) => cmd(ThreadCmd::Drop {
                name: name.to_string(),
            }),
            Ok(ThreadOp::Merge { name, view }) => cmd(ThreadCmd::Merge {
                name: name.to_string(),
                view: *view,
            }),
            Err(usage) => Some(Err(usage.clone())),
        },
        _ => None,
    }
}

fn attached_after(current: Option<String>, cmd: &ThreadCmd) -> Option<String> {
    match cmd {
        ThreadCmd::Switch { name } => Some(name.clone()),
        ThreadCmd::Main => None,
        _ => current,
    }
}

fn cancel_due(cancelled: bool, sent: bool) -> bool {
    cancelled && !sent
}

struct RemoteSession {
    socket: PathBuf,
    stream: RefCell<BufReader<UnixStream>>,
    next_id: Cell<u64>,
    turns: Cell<u64>,
    attached: RefCell<Option<String>>,
    sink: Box<dyn TurnSink>,
}

impl RemoteSession {
    fn connect(socket: &Path, sink: Box<dyn TurnSink>) -> Result<Self, Box<dyn Error>> {
        let stream = UnixStream::connect(socket)?;
        stream.set_read_timeout(Some(POLL))?;
        let session = Self {
            socket: socket.to_path_buf(),
            stream: RefCell::new(BufReader::new(stream)),
            next_id: Cell::new(1),
            turns: Cell::new(0),
            attached: RefCell::new(None),
            sink,
        };
        session.write(&ClientMsg::Hello { proto: VERSION })?;
        match session.read_reply()? {
            ServerMsg::Hello { .. } => Ok(session),
            ServerMsg::Error { message } => Err(message.into()),
            _ => Err("ralph serve did not answer the hello".into()),
        }
    }

    fn ask(&self, msg: &ClientMsg) -> Result<ServerMsg, Box<dyn Error>> {
        self.write(msg)?;
        self.read_reply()
    }

    fn read_reply(&self) -> Result<ServerMsg, Box<dyn Error>> {
        loop {
            let msg = self.read_frame(&mut String::new(), None)?;
            if !matches!(msg, ServerMsg::Blocked { .. }) {
                return Ok(msg);
            }
            if let Step::Notice(text) = on_frame(msg) {
                notice(text);
            }
        }
    }

    fn status_line(&self) -> Result<String, Box<dyn Error>> {
        let attached = self.attached.borrow().clone();
        match self.ask(&ClientMsg::Status {
            thread: attached.clone(),
        })? {
            ServerMsg::Status {
                report,
                unread,
                blocked,
                turns,
            } => {
                self.turns.set(turns);
                Ok(status_summary(
                    attached.as_deref(),
                    report.state,
                    unread,
                    &blocked,
                ))
            }
            ServerMsg::Error { message } => Err(Refused(message).into()),
            _ => Err("ralph serve did not answer the status request".into()),
        }
    }

    fn thread(&self, cmd: ThreadCmd) -> Result<String, Box<dyn Error>> {
        match self.ask(&ClientMsg::Thread(cmd.clone()))? {
            ServerMsg::Done { message } => {
                let current = self.attached.take();
                *self.attached.borrow_mut() = attached_after(current, &cmd);
                Ok(message)
            }
            ServerMsg::Error { message } => Err(Refused(message).into()),
            _ => Err("ralph serve did not answer the thread request".into()),
        }
    }

    fn answer(&self, args: AnswerArgs) -> Result<String, Box<dyn Error>> {
        let thread = self.attached.borrow().clone();
        match self.ask(&ClientMsg::Answer {
            thread,
            worker: args.worker,
            request: args.request,
            option: args.option,
        })? {
            ServerMsg::Done { message } => Ok(message),
            ServerMsg::Error { message } => Err(Refused(message).into()),
            _ => Err("ralph serve did not answer the answer request".into()),
        }
    }

    fn write(&self, msg: &ClientMsg) -> io::Result<()> {
        let id = self.next_id.replace(self.next_id.get() + 1);
        self.stream
            .borrow_mut()
            .get_mut()
            .write_all(encode_frame(id, msg).as_bytes())
    }

    fn read_frame(
        &self,
        line: &mut String,
        cancel: Option<(&CancelToken, &mut bool)>,
    ) -> Result<ServerMsg, Box<dyn Error>> {
        let mut cancel = cancel;
        loop {
            match self.stream.borrow_mut().read_line(line) {
                Ok(0) => return Err("ralph serve closed the connection".into()),
                Ok(_) if line.ends_with('\n') => {
                    let frame = decode_frame::<ServerMsg>(line.trim_end())?;
                    line.clear();
                    return Ok(frame.body);
                }
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(e.into()),
            }
            if let Some((token, sent)) = cancel.as_mut() {
                if cancel_due(token.is_cancelled(), **sent) {
                    **sent = true;
                    warn_if_err(self.cancel_remote(), "Failed to cancel the turn");
                }
            }
        }
    }

    fn cancel_remote(&self) -> io::Result<()> {
        let mut stream = UnixStream::connect(&self.socket)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        for (id, msg) in [
            ClientMsg::Hello { proto: VERSION },
            ClientMsg::Cancel {
                thread: self.attached.borrow().clone(),
            },
        ]
        .iter()
        .enumerate()
        {
            stream.write_all(encode_frame(id as u64 + 1, msg).as_bytes())?;
        }
        let mut reader = BufReader::new(stream);
        reader.read_line(&mut String::new())?;
        reader.read_line(&mut String::new())?;
        Ok(())
    }
}

impl SessionPort for RemoteSession {
    fn around(&self, run: &mut dyn FnMut()) {
        run();
    }

    fn turn(&self, call: &TurnCall, cancel: &CancelToken) -> Result<TurnOutcome, Box<dyn Error>> {
        self.write(&ClientMsg::Send {
            thread: None,
            text: call.sent.to_owned(),
        })?;
        let mut line = String::new();
        let mut sent = false;
        loop {
            match on_frame(self.read_frame(&mut line, Some((cancel, &mut sent)))?) {
                Step::Render(event) => self.sink.emit(event),
                Step::Finished => {
                    self.turns.set(self.turns.get() + 1);
                    return Ok(TurnOutcome::Replied);
                }
                Step::Failed(message) => return Err(Refused(message).into()),
                Step::Notice(text) => notice(text),
                Step::Ignored => {}
            }
        }
    }

    fn send_it(&self, _command: SendItCommand, _at: &At, _ask: &mut dyn Ask) -> Reply {
        Reply::plain("/send-it is not served over --connect")
    }

    fn proposed(&self, _proposal: Proposal, _width: usize) -> Screen {
        Screen::text("")
    }

    fn set_phase(&self, _phase: Phase) {}

    fn refresh(&self) {
        warn_if_err(self.status_line(), "Failed to read the serve status");
    }

    fn status(&self) -> Status {
        Status::default()
    }

    fn unread(&self) -> Vec<Report> {
        Vec::new()
    }

    fn take_cost(&self) -> Option<f64> {
        None
    }

    fn turns(&self) -> u64 {
        self.turns.get()
    }

    fn end_session(&self) {
        warn_if_err(
            self.stream.borrow().get_ref().shutdown(Shutdown::Both),
            "Failed to close the connection",
        );
    }
}

fn send(remote: &RemoteSession, text: &str) -> Result<(), Box<dyn Error>> {
    let call = TurnCall {
        trigger: Trigger::User,
        logged: &[],
        sent: text,
    };
    run_turn(remote, &call, |run| {
        let cancel = CancelToken::default();
        let _armed = signal::arm(&cancel);
        run(&cancel);
        Queue::default()
    })?;
    Ok(())
}

fn settle(result: Result<(), Box<dyn Error>>) -> Result<(), Box<dyn Error>> {
    match result {
        Err(err) if keeps_chat_open(&*err) => {
            println!("{err}");
            Ok(())
        }
        other => other,
    }
}

fn handle(remote: &RemoteSession, line: &str) -> Result<(), Box<dyn Error>> {
    let route = route_input(line, 0, &EmptySubmit::default(), &BTreeSet::new());
    if let Route::Send(text) = &route {
        return send(remote, text);
    }
    if let Route::Answer(parsed) = route {
        match parsed {
            Ok(args) => println!("{}", remote.answer(args)?),
            Err(usage) => println!("{usage}"),
        }
        return Ok(());
    }
    match thread_request(&route) {
        Some(Ok(cmd)) => println!("{}", remote.thread(cmd)?),
        Some(Err(message)) => println!("{message}"),
        None if matches!(route, Route::Ignore) || line.trim().is_empty() => {}
        None => println!("{} is not served over --connect", line.trim()),
    }
    Ok(())
}

pub(super) fn run(socket: &Path) -> Result<ChatExit, Box<dyn Error>> {
    warn_if_err(signal::init(), "Failed to initialize signal handler");
    let theme = ThemeConfig::from_config_and_env();
    let sink = TerminalSink::new(None, &theme, screen_columns)?;
    let remote = RemoteSession::connect(socket, Box::new(sink))?;
    settle(remote.status_line().map(|line| println!("{line}")))?;
    let mut editor = Reedline::create();
    let prompt = DefaultPrompt::default();
    loop {
        match retry_interrupted(|| editor.read_line(&prompt))? {
            Signal::Success(line) => {
                settle(handle(&remote, &line))?;
            }
            Signal::CtrlD => break,
            _ => {}
        }
    }
    remote.end_session();
    Ok(ChatExit::Quit)
}

#[cfg(test)]
mod tests;
