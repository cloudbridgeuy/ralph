use crate::chat::prompt::screen_columns;
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
use ralph_core::sanitize::strip_controls;
use ralph_core::stream::blocks::{truncate_string, OutputBlock};
use ralph_core::turn_event::{TurnEvent, TurnSink};
use ralph_core::worker::{blocked_notice, Report};
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
use std::io::{self, BufRead, BufReader, IsTerminal, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
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
        _ => push_notice(&msg).map_or(Step::Ignored, Step::Notice),
    }
}

fn remote_blocks(blocks: Vec<OutputBlock>, stdout_is_tty: bool) -> Vec<OutputBlock> {
    blocks
        .into_iter()
        .filter(|block| {
            stdout_is_tty
                || !matches!(
                    block,
                    OutputBlock::ToolInvocation(_) | OutputBlock::ToolResult(_)
                )
        })
        .collect()
}

fn remote_event(event: TurnEvent, stdout_is_tty: bool) -> TurnEvent {
    match event {
        TurnEvent::Blocks(blocks) => TurnEvent::Blocks(remote_blocks(blocks, stdout_is_tty)),
        other => other,
    }
}

fn push_notice(msg: &ServerMsg) -> Option<String> {
    match msg {
        ServerMsg::Blocked {
            thread,
            worker,
            content,
        } => Some(blocked_notice(thread.as_deref(), worker, content.as_ref())),
        ServerMsg::Inbox {
            thread,
            report,
            unread,
        } => {
            let label = thread.as_deref().unwrap_or("main");
            let line = format!(
                "{label}/{} {} ({unread} unread): {}",
                report.worker, report.status, report.text
            );
            let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
            Some(truncate_string(&strip_controls(&line), 240))
        }
        _ => None,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WaitStep {
    Continue,
    SendCancel,
    Exit,
}

fn wait_step(terminated: bool, cancelled: bool, sent: bool) -> WaitStep {
    if terminated {
        WaitStep::Exit
    } else if cancelled && !sent {
        WaitStep::SendCancel
    } else {
        WaitStep::Continue
    }
}

#[derive(Debug, PartialEq, Eq)]
enum PromptStep {
    Send(String),
    Continue,
    Exit,
}

fn prompt_step(signal: Signal, terminated: bool) -> PromptStep {
    if terminated {
        return PromptStep::Exit;
    }
    match signal {
        Signal::Success(line) => PromptStep::Send(line),
        Signal::CtrlD => PromptStep::Exit,
        _ => PromptStep::Continue,
    }
}

fn read_prompt_until<T>(
    mut read: impl FnMut() -> io::Result<T>,
    terminated: impl Fn() -> bool,
) -> io::Result<Option<T>> {
    loop {
        if terminated() {
            return Ok(None);
        }
        match read() {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            outcome => return outcome.map(Some),
        }
    }
}

struct RemoteSession {
    socket: PathBuf,
    stream: RefCell<BufReader<UnixStream>>,
    next_id: Cell<u64>,
    turns: Cell<u64>,
    attached: RefCell<Option<String>>,
    sink: Box<dyn TurnSink>,
    stdout_is_tty: bool,
}

impl RemoteSession {
    fn connect(socket: &Path, sink: Box<dyn TurnSink>) -> Result<Option<Self>, Box<dyn Error>> {
        let stream = UnixStream::connect(socket)?;
        stream.set_read_timeout(Some(POLL))?;
        let session = Self {
            socket: socket.to_path_buf(),
            stream: RefCell::new(BufReader::new(stream)),
            next_id: Cell::new(1),
            turns: Cell::new(0),
            attached: RefCell::new(None),
            sink,
            stdout_is_tty: io::stdout().is_terminal(),
        };
        session.write(&ClientMsg::Hello { proto: VERSION })?;
        let Some(reply) = session.read_reply()? else {
            session.end_session();
            return Ok(None);
        };
        match reply {
            ServerMsg::Hello { .. } => Ok(Some(session)),
            ServerMsg::Error { message } => Err(message.into()),
            _ => Err("ralph serve did not answer the hello".into()),
        }
    }

    fn ask(&self, msg: &ClientMsg) -> Result<Option<ServerMsg>, Box<dyn Error>> {
        self.write(msg)?;
        self.read_reply()
    }

    fn read_reply(&self) -> Result<Option<ServerMsg>, Box<dyn Error>> {
        loop {
            let Some(msg) = self.read_frame(&mut String::new(), None)? else {
                return Ok(None);
            };
            if let Some(text) = push_notice(&msg) {
                notice(text);
            } else {
                return Ok(Some(msg));
            }
        }
    }

    fn status_line(&self) -> Result<Option<String>, Box<dyn Error>> {
        let attached = self.attached.borrow().clone();
        let Some(reply) = self.ask(&ClientMsg::Status {
            thread: attached.clone(),
        })?
        else {
            return Ok(None);
        };
        match reply {
            ServerMsg::Status {
                report,
                unread,
                blocked,
                turns,
            } => {
                self.turns.set(turns);
                Ok(Some(status_summary(
                    attached.as_deref(),
                    report.state,
                    unread,
                    &blocked,
                )))
            }
            ServerMsg::Error { message } => Err(Refused(message).into()),
            _ => Err("ralph serve did not answer the status request".into()),
        }
    }

    fn thread(&self, cmd: ThreadCmd) -> Result<Option<String>, Box<dyn Error>> {
        let Some(reply) = self.ask(&ClientMsg::Thread(cmd.clone()))? else {
            return Ok(None);
        };
        match reply {
            ServerMsg::Done { message } => {
                let current = self.attached.take();
                *self.attached.borrow_mut() = attached_after(current, &cmd);
                Ok(Some(message))
            }
            ServerMsg::Error { message } => Err(Refused(message).into()),
            _ => Err("ralph serve did not answer the thread request".into()),
        }
    }

    fn answer(&self, args: AnswerArgs) -> Result<Option<String>, Box<dyn Error>> {
        let thread = self.attached.borrow().clone();
        let Some(reply) = self.ask(&ClientMsg::Answer {
            thread,
            worker: args.worker,
            request: args.request,
            option: args.option,
        })?
        else {
            return Ok(None);
        };
        match reply {
            ServerMsg::Done { message } => Ok(Some(message)),
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
    ) -> Result<Option<ServerMsg>, Box<dyn Error>> {
        self.read_frame_with(line, cancel, signal::is_terminated)
    }

    fn read_frame_with(
        &self,
        line: &mut String,
        cancel: Option<(&CancelToken, &mut bool)>,
        terminated: impl Fn() -> bool,
    ) -> Result<Option<ServerMsg>, Box<dyn Error>> {
        let mut cancel = cancel;
        loop {
            if terminated() {
                return Ok(None);
            }
            match self.stream.borrow_mut().read_line(line) {
                Ok(0) => return Err("ralph serve closed the connection".into()),
                Ok(_) if line.ends_with('\n') => {
                    let frame = decode_frame::<ServerMsg>(line.trim_end())?;
                    line.clear();
                    return Ok(Some(frame.body));
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
            let (cancelled, sent) = match cancel.as_ref() {
                Some((token, sent)) => (token.is_cancelled(), **sent),
                None => (false, false),
            };
            match wait_step(terminated(), cancelled, sent) {
                WaitStep::Exit => return Ok(None),
                WaitStep::SendCancel => {
                    if let Some((_, sent)) = cancel.as_mut() {
                        **sent = true;
                    }
                    warn_if_err(self.cancel_remote(), "Failed to cancel the turn");
                }
                WaitStep::Continue => {}
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
            let Some(msg) = self.read_frame(&mut line, Some((cancel, &mut sent)))? else {
                return Ok(TurnOutcome::Exit);
            };
            match on_frame(msg) {
                Step::Render(event) => self.sink.emit(remote_event(event, self.stdout_is_tty)),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Continue,
    Exit,
}

fn send(remote: &RemoteSession, text: &str) -> Result<Flow, Box<dyn Error>> {
    let call = TurnCall {
        trigger: Trigger::User,
        logged: &[],
        sent: text,
    };
    let (outcome, _) = run_turn(remote, &call, |run| {
        let cancel = CancelToken::default();
        let _armed = signal::arm(&cancel);
        run(&cancel);
        Queue::default()
    })?;
    Ok(if outcome == TurnOutcome::Exit {
        Flow::Exit
    } else {
        Flow::Continue
    })
}

fn settle(result: Result<Flow, Box<dyn Error>>) -> Result<Flow, Box<dyn Error>> {
    match result {
        Err(err) if keeps_chat_open(&*err) => {
            println!("{err}");
            Ok(Flow::Continue)
        }
        other => other,
    }
}

fn print_or_exit(reply: Option<String>) -> Flow {
    match reply {
        Some(text) => {
            println!("{text}");
            Flow::Continue
        }
        None => Flow::Exit,
    }
}

fn handle(remote: &RemoteSession, line: &str) -> Result<Flow, Box<dyn Error>> {
    let route = route_input(line, 0, &EmptySubmit::default(), &BTreeSet::new());
    if let Route::Send(text) = &route {
        return send(remote, text);
    }
    if let Route::Answer(parsed) = route {
        return match parsed {
            Ok(args) => Ok(print_or_exit(remote.answer(args)?)),
            Err(usage) => {
                println!("{usage}");
                Ok(Flow::Continue)
            }
        };
    }
    match thread_request(&route) {
        Some(Ok(cmd)) => Ok(print_or_exit(remote.thread(cmd)?)),
        Some(Err(message)) => {
            println!("{message}");
            Ok(Flow::Continue)
        }
        None if matches!(route, Route::Ignore) || line.trim().is_empty() => Ok(Flow::Continue),
        None => {
            println!("{} is not served over --connect", line.trim());
            Ok(Flow::Continue)
        }
    }
}

pub(super) fn run(socket: &Path) -> Result<ChatExit, Box<dyn Error>> {
    warn_if_err(signal::init(), "Failed to initialize signal handler");
    let theme = ThemeConfig::from_config_and_env();
    let sink = TerminalSink::new(None, &theme, screen_columns)?;
    let Some(remote) = RemoteSession::connect(socket, Box::new(sink))? else {
        return Ok(ChatExit::Quit);
    };
    if settle(remote.status_line().map(print_or_exit))? == Flow::Exit {
        remote.end_session();
        return Ok(ChatExit::Quit);
    }
    let break_flag = Arc::new(AtomicBool::new(false));
    let _term_break = signal::arm_terminate(&break_flag);
    let mut editor = Reedline::create().with_break_signal(Arc::clone(&break_flag));
    let prompt = DefaultPrompt::default();
    while let Some(sig) = read_prompt_until(|| editor.read_line(&prompt), signal::is_terminated)? {
        match prompt_step(sig, signal::is_terminated()) {
            PromptStep::Send(line) => {
                if settle(handle(&remote, &line))? == Flow::Exit {
                    break;
                }
            }
            PromptStep::Continue => {}
            PromptStep::Exit => break,
        }
    }
    remote.end_session();
    Ok(ChatExit::Quit)
}

#[cfg(test)]
mod tests;
