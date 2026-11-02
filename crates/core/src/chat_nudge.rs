use crate::chat_hold::{command_args, word_completion, HoldCompletion};
use crate::chat_status::Status;

const NUDGE: &str = "/nudge";
pub const NUDGE_USAGE: &str = "Usage: /nudge <id> [text]";
pub const PROGRESS_REQUEST: &str =
    "Nudge: reply with one line of progress (current step, blocker), then keep going on the same plan.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NudgeCommand {
    Send { id: String, text: String },
    Usage,
}

pub fn parse_nudge_args(args: &str) -> NudgeCommand {
    let args = args.trim();
    let Some(id) = args.split_whitespace().next() else {
        return NudgeCommand::Usage;
    };
    let text = match args[id.len()..].trim() {
        "" => PROGRESS_REQUEST,
        text => text,
    };
    NudgeCommand::Send {
        id: id.to_string(),
        text: text.to_string(),
    }
}

pub fn nudge_completions(line: &str, cursor: usize, status: &Status) -> Option<HoldCompletion> {
    let args = command_args(line, cursor, NUDGE)?;
    if args.trim_start().contains(char::is_whitespace) {
        return None;
    }
    let workers = status.workers.iter().map(|worker| worker.id.clone());
    Some(word_completion(args, cursor, workers.collect()))
}

pub fn blocked_refusal(id: &str) -> String {
    format!("{NUDGE}: worker {id} waits on a permission request; answer it by request id.")
}

pub fn failure_line(error: &str) -> String {
    format!("{NUDGE}: {error}")
}

#[cfg(test)]
mod tests;
