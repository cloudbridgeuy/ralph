use crate::chat_memory::{tail_lines, Entry, Tree};
use crate::chat_nudge::NudgeCommand;
use crate::chat_send_it::SendItCommand;
use crate::chat_status::Route;
use crate::chat_thread::ForkCommand;
use crate::chat_thread_cmd::ThreadOp;
use crate::schedule_command::ScheduleCommand;
use crate::watch_command::WatchCommand;

const RESULT: &str = "\nResult: ";
const NOTICE: &str = "[ralph] ";

fn recorded(route: &Route) -> bool {
    match route {
        Route::SendIt(command) => !matches!(
            command,
            SendItCommand::Status | SendItCommand::Templates | SendItCommand::Usage
        ),
        Route::ThreadOp { op, .. } => !matches!(op, Ok(ThreadOp::Ls)),
        Route::Fork(command) => matches!(command, ForkCommand::Open(_)),
        Route::Watch(command) => matches!(command, WatchCommand::Start(_) | WatchCommand::Stop(_)),
        Route::Schedule(command) => matches!(command, ScheduleCommand::Cancel(_)),
        Route::Nudge(command) => matches!(command, NudgeCommand::Send { .. }),
        Route::DropInbox(_) => true,
        _ => false,
    }
}

pub struct CommandRecord(Option<String>);

impl CommandRecord {
    pub fn for_route(route: &Route, line: &str) -> Self {
        Self(recorded(route).then(|| line.to_string()))
    }

    pub fn row(&self, output: &str) -> Option<String> {
        self.0.as_deref().map(|line| local_record(line, output))
    }
}

pub fn local_record(line: &str, output: &str) -> String {
    format!("{}{RESULT}{}", line.trim(), output.trim())
}

pub fn notice_record(notice: &str) -> String {
    format!("{NOTICE}{}", notice.trim())
}

pub fn is_local_record(text: &str) -> bool {
    let command = text.starts_with('/')
        && text
            .split_once('\n')
            .is_some_and(|(_, rest)| rest.starts_with(&RESULT[1..]));
    command || text.starts_with(NOTICE)
}

pub fn catch_up_prompt(rows: &[String], message: &str) -> String {
    match rows {
        [] => message.to_string(),
        rows => format!(
            "Since your last turn:\n{}\n\nNew message:\n{message}",
            rows.join("\n")
        ),
    }
}

pub fn unseen_lines(entries: &[Entry], seen: Option<usize>) -> Vec<String> {
    match seen {
        Some(seen) if seen < entries.len() => tail_lines(entries, &Tree::new(), seen, usize::MAX),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests;
