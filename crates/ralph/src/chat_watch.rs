use crate::watches::Watches;
use ralph_core::watch_command::{
    list_watches, next_watch_id, start_message, stop_message, StartWatch, WatchCommand, WATCH_USAGE,
};
use ralph_core::watch_request::WatchRequest;

pub fn run(command: WatchCommand, watches: &Watches) {
    match command {
        WatchCommand::Start(start) => start_watch(start, watches),
        WatchCommand::List => println!("{}", list_watches(&watches.list(), chrono::Utc::now())),
        WatchCommand::Stop(id) => println!("{}", stop_message(&id, watches.stop(&id))),
        WatchCommand::Usage => println!("{WATCH_USAGE}"),
    }
}

fn start_watch(start: StartWatch, watches: &Watches) {
    let spec = WatchRequest {
        id: next_watch_id(&watches.used_ids()),
        command: start.command,
        follow_up: start.follow_up,
        persistent: start.persistent,
        timeout_secs: None,
    }
    .into_spec();
    let confirmation = start_message(&spec.id, spec.kind);
    match watches.start(spec) {
        Ok(()) => println!("{confirmation}"),
        Err(e) => println!("{e}"),
    }
}
