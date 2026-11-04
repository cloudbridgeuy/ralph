use crate::chat::monitor::Monitor;
use ralph_core::chat_nudge::{failure_line, resolve_nudge, NudgeCommand};
use ralph_core::worker::blocked_path;

pub fn run(command: NudgeCommand, monitor: &Monitor) {
    let running: Vec<String> = monitor.status().workers.into_iter().map(|w| w.id).collect();
    let blocked = |id: &str| blocked_path(&monitor.store_dir, id).exists();
    let (id, text) = match resolve_nudge(command, &running, blocked) {
        Ok(send) => send,
        Err(line) => return println!("{line}"),
    };
    match monitor.workers.send_reply(&id, &text) {
        Ok(line) => println!("{line}"),
        Err(error) => println!("{}", failure_line(&error.to_string())),
    }
}
