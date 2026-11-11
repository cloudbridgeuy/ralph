use crate::chat_nudge::NudgeCommand;
use crate::chat_send_it::SendItCommand;
use crate::chat_status::Route;
use crate::chat_thread::ForkCommand;
use crate::chat_thread_cmd::ThreadOp;
use crate::schedule_command::ScheduleCommand;
use crate::watch_command::WatchCommand;

const RESULT: &str = "\nResult: ";
const NOTICE: &str = "[ralph] ";

pub fn recorded(route: &Route) -> bool {
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

#[cfg(test)]
mod tests;
