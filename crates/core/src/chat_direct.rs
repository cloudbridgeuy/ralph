use crate::chat_hold::{word_completion, HoldCompletion};
use crate::chat_nudge::blocked_refusal;
use crate::chat_status::Status;
use crate::chat_thread::refuse_thread_report;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Direct {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectLine {
    Message(Direct),
    Escaped(String),
}

pub fn parse_direct(line: &str) -> Option<DirectLine> {
    let rest = line.trim().strip_prefix('@')?;
    if rest.starts_with('@') {
        return Some(DirectLine::Escaped(rest.to_string()));
    }
    let id = rest
        .split(char::is_whitespace)
        .next()
        .filter(|id| !id.is_empty())?;
    Some(DirectLine::Message(Direct {
        id: id.to_string(),
        text: rest[id.len()..].trim().to_string(),
    }))
}

pub fn resolve_direct(
    direct: Direct,
    known: &[String],
    blocked: impl Fn(&str) -> bool,
) -> Result<Direct, String> {
    let id = &direct.id;
    if direct.text.is_empty() {
        return Err(failure_line(id, "nothing to send; give text after the id."));
    }
    refuse_thread_report(id).map_err(|error| failure_line(id, &error))?;
    if !known.contains(id) {
        return Err(failure_line(id, &unknown_worker(known)));
    }
    if blocked(id) {
        return Err(failure_line(id, &blocked_refusal(id)));
    }
    Ok(direct)
}

fn unknown_worker(known: &[String]) -> String {
    let mut known = known.to_vec();
    known.sort();
    let hint = "start the line with @@ to send a leading @ to the model.";
    match known.is_empty() {
        true => format!("no workers running; {hint}"),
        false => format!("no such worker; running: {}; {hint}", known.join(", ")),
    }
}

pub fn direct_completions(line: &str, cursor: usize, status: &Status) -> Option<HoldCompletion> {
    let typed = line.get(..cursor)?.trim_start().strip_prefix('@')?;
    if typed.starts_with('@') || typed.contains(char::is_whitespace) {
        return None;
    }
    let workers = status.workers.iter().map(|worker| worker.id.clone());
    Some(word_completion(typed, cursor, workers.collect()))
}

pub fn memory_line(direct: &Direct, result: &str) -> String {
    format!("@{} {}\nResult: {result}", direct.id, direct.text)
}

pub fn failure_line(id: &str, error: &str) -> String {
    format!("@{id}: {error}")
}

#[cfg(test)]
mod tests;
