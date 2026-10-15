use crate::watches::Watches;
use ralph_core::watch_command::{
    list_watches, plan_start, stop_message, StartWatch, WatchCommand, WATCH_USAGE,
};

pub fn run(command: WatchCommand, watches: &Watches) {
    match command {
        WatchCommand::Start(start) => start_watch(start, watches),
        WatchCommand::List => println!("{}", list_watches(&watches.list(), chrono::Utc::now())),
        WatchCommand::Stop(id) => println!("{}", stop_message(&id, watches.stop(&id))),
        WatchCommand::Usage => println!("{WATCH_USAGE}"),
    }
}

fn start_watch(start: StartWatch, watches: &Watches) {
    let plan = plan_start(start, &watches.used_ids());
    match watches.start(plan.spec) {
        Ok(()) => println!("{}", plan.confirmation),
        Err(e) => println!("{e}"),
    }
}
