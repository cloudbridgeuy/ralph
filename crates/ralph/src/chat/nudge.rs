use crate::chat::monitor::Monitor;
use ralph_core::chat_nudge::{blocked_refusal, failure_line, NudgeCommand, NUDGE_USAGE};
use ralph_core::worker::blocked_path;

pub fn run(command: NudgeCommand, monitor: &Monitor) {
    let NudgeCommand::Send { id, text } = command else {
        return println!("{NUDGE_USAGE}");
    };
    if blocked_path(&monitor.store_dir, &id).exists() {
        return println!("{}", blocked_refusal(&id));
    }
    match monitor.workers.reply(&id, &text, false, None) {
        Ok(line) => println!("{line}"),
        Err(error) => println!("{}", failure_line(&error.to_string())),
    }
}
