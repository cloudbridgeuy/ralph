use super::run_turn;
use crate::chat::queue::Queue;
use ralph_core::cancel::CancelToken;
use ralph_core::chat_memory::Trigger;
use ralph_core::chat_send_it::SendItCommand;
use ralph_core::chat_status::Status;
use ralph_core::program_status::Phase;
use ralph_core::worker::Report;
use ralph_server::chat::ask::Ask;
use ralph_server::chat::port::{SessionPort, TurnCall};
use ralph_server::chat::send_it::Screen;
use ralph_server::chat::session::{At, Proposal, Reply};
use ralph_server::chat::turn::TurnOutcome;
use std::cell::RefCell;
use std::error::Error;

struct FakePort {
    log: RefCell<Vec<String>>,
    outcome: TurnOutcome,
}

impl FakePort {
    fn new(outcome: TurnOutcome) -> Self {
        Self {
            log: RefCell::new(Vec::new()),
            outcome,
        }
    }

    fn note(&self, line: &str) {
        self.log.borrow_mut().push(line.to_owned());
    }
}

impl SessionPort for FakePort {
    fn around(&self, run: &mut dyn FnMut()) {
        self.note("snapshot start");
        run();
        self.note("snapshot end");
    }

    fn turn(&self, call: &TurnCall, _cancel: &CancelToken) -> Result<TurnOutcome, Box<dyn Error>> {
        self.note(&format!(
            "turn {:?} {:?} {}",
            call.trigger, call.logged, call.sent
        ));
        Ok(self.outcome)
    }

    fn send_it(&self, _command: SendItCommand, _at: &At, _ask: &mut dyn Ask) -> Reply {
        Reply::plain("")
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
        0
    }

    fn end_session(&self) {}
}

const LOGGED: [String; 0] = [];

fn call(sent: &str) -> TurnCall<'_> {
    TurnCall {
        trigger: Trigger::User,
        logged: &LOGGED,
        sent,
    }
}

#[test]
fn run_turn_runs_the_turn_inside_the_keys_inside_the_snapshot() {
    let port = FakePort::new(TurnOutcome::Exit);
    let (outcome, queue) = run_turn(&port, &call("hi"), |run| {
        port.note("keys");
        run(&CancelToken::default());
        Queue {
            entries: vec!["typed".to_owned()],
            ..Queue::default()
        }
    })
    .unwrap();
    assert_eq!(outcome, TurnOutcome::Exit);
    assert_eq!(queue.entries, ["typed"]);
    assert_eq!(
        *port.log.borrow(),
        ["snapshot start", "keys", "turn User [] hi", "snapshot end"]
    );
}

#[test]
fn run_turn_fails_when_the_keys_never_run_the_turn() {
    let port = FakePort::new(TurnOutcome::Replied);
    let error = run_turn(&port, &call("hi"), |_run| Queue::default()).unwrap_err();
    assert_eq!(error.to_string(), "keyboard did not run the turn");
    assert_eq!(*port.log.borrow(), ["snapshot start", "snapshot end"]);
}
