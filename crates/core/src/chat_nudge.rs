use crate::chat_hold::{command_args, word_completion, HoldCompletion};
use crate::chat_status::Status;

const NUDGE: &str = "/nudge";
const NUDGE_USAGE: &str = "Usage: /nudge <id> [text]";
pub const PROGRESS_REQUEST: &str =
    "Nudge: reply with one line of progress (current step, blocker), then keep going on the same plan.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NudgeCommand {
    Send { id: String, text: String },
    Bare,
}

pub fn parse_nudge_args(args: &str) -> NudgeCommand {
    let args = args.trim();
    let Some(id) = args.split_whitespace().next() else {
        return NudgeCommand::Bare;
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

pub fn resolve_nudge(
    command: NudgeCommand,
    running: &[String],
    blocked: impl Fn(&str) -> bool,
) -> Result<(String, String), String> {
    let (id, text) = match (command, running) {
        (NudgeCommand::Send { id, text }, _) => (id, text),
        (NudgeCommand::Bare, []) => return Err(format!("{NUDGE}: no workers running")),
        (NudgeCommand::Bare, [id]) => (id.clone(), PROGRESS_REQUEST.to_string()),
        (NudgeCommand::Bare, ids) => {
            let mut ids = ids.to_vec();
            ids.sort();
            return Err(format!("{NUDGE_USAGE}; running: {}", ids.join(", ")));
        }
    };
    if blocked(&id) {
        return Err(blocked_refusal(&id));
    }
    Ok((id, text))
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
