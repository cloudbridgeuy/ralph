use crate::watches::Watches;
use ralph_core::watch_command::{
    list_watches, plan_start, stop_message, StartWatch, WatchCommand, WATCH_USAGE,
};

pub fn run(command: WatchCommand, watches: &Watches) -> String {
    match command {
        WatchCommand::Start(start) => start_watch(start, watches),
        WatchCommand::List => list_watches(&watches.list(), chrono::Utc::now()),
        WatchCommand::Stop(id) => stop_message(&id, watches.stop(&id)),
        WatchCommand::Usage => WATCH_USAGE.to_string(),
    }
}

fn start_watch(start: StartWatch, watches: &Watches) -> String {
    let plan = plan_start(start, &watches.used_ids());
    match watches.start(plan.spec) {
        Ok(_) => plan.confirmation,
        Err(e) => e.to_string(),
    }
}
