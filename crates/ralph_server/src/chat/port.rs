use crate::chat::ask::Ask;
use crate::chat::send_it::Screen;
use crate::chat::session::{At, Proposal, Reply};
use crate::chat::turn::TurnOutcome;
use ralph_core::cancel::CancelToken;
use ralph_core::chat_memory::Trigger;
use ralph_core::chat_send_it::SendItCommand;
use std::error::Error;

pub struct TurnCall<'a> {
    pub trigger: Trigger,
    pub logged: &'a [String],
    pub sent: &'a str,
}

pub trait SessionPort {
    fn around(&self, run: &mut dyn FnMut());
    fn turn(&self, call: &TurnCall, cancel: &CancelToken) -> Result<TurnOutcome, Box<dyn Error>>;
    fn send_it(&self, command: SendItCommand, at: &At, ask: &mut dyn Ask) -> Reply;
    fn proposed(&self, proposal: Proposal, width: usize) -> Screen;
}
