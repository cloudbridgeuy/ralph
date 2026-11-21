use crate::chat::prompt::{retry_interrupted, screen_columns};
use crate::chat::queue::Queue;
use crate::chat::switch::ChatExit;
use crate::chat::term_loop::run_turn;
use crate::highlight::ThemeConfig;
use crate::signal;
use crate::turn_sink::TerminalSink;
use ralph_core::cancel::CancelToken;
use ralph_core::chat_memory::Trigger;
use ralph_core::chat_send_it::SendItCommand;
use ralph_core::chat_status::{route_input, EmptySubmit, Route, Status};
use ralph_core::program_status::Phase;
use ralph_core::turn_event::{TurnEvent, TurnSink};
use ralph_core::worker::Report;
use ralph_server::chat::ask::Ask;
use ralph_server::chat::port::{SessionPort, TurnCall};
use ralph_server::chat::send_it::Screen;
use ralph_server::chat::serve_proto::{decode_frame, encode_frame, ClientMsg, ServerMsg, VERSION};
use ralph_server::chat::session::{At, Proposal, Reply};
use ralph_server::chat::turn::TurnOutcome;
use ralph_server::warn::warn_if_err;
use reedline::{DefaultPrompt, Reedline, Signal};
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::error::Error;
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
    Ignored,
}

fn on_frame(msg: ServerMsg) -> Step {
    match msg {
        ServerMsg::Turn { event } => Step::Render(event),
        ServerMsg::TurnDone => Step::Finished,
        ServerMsg::Error { message } => Step::Failed(message),
        ServerMsg::Hello { .. } | ServerMsg::Done { .. } | ServerMsg::Status { .. } => {
            Step::Ignored
        }
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
            sink,
        };
        session.write(&ClientMsg::Hello { proto: VERSION })?;
        match session.read_frame(&mut String::new(), None)? {
            ServerMsg::Hello { .. } => Ok(session),
            ServerMsg::Error { message } => Err(message.into()),
            _ => Err("ralph serve did not answer the hello".into()),
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
            ClientMsg::Cancel { thread: None },
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
                Step::Failed(message) => return Err(message.into()),
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

    fn refresh(&self) {}

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

pub(super) fn run(socket: &Path) -> Result<ChatExit, Box<dyn Error>> {
    warn_if_err(signal::init(), "Failed to initialize signal handler");
    let theme = ThemeConfig::from_config_and_env();
    let sink = TerminalSink::new(None, &theme, screen_columns)?;
    let remote = RemoteSession::connect(socket, Box::new(sink))?;
    let mut editor = Reedline::create();
    let prompt = DefaultPrompt::default();
    loop {
        match retry_interrupted(|| editor.read_line(&prompt))? {
            Signal::Success(line) => {
                let route = route_input(&line, 0, &EmptySubmit::default(), &BTreeSet::new());
                match route {
                    Route::Send(text) => send(&remote, &text)?,
                    Route::Ignore => {}
                    _ if line.trim().is_empty() => {}
                    _ => println!("{} is not served over --connect", line.trim()),
                }
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
