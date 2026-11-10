use crate::chat::monitor::Monitor;
use crate::chat::route::LocalRoute;
use ralph_core::chat_direct::{failure_line, memory_line, resolve_direct, Direct};
use ralph_core::chat_memory::Role;
use ralph_core::schedule_worker::reply_refusal;
use ralph_core::worker::blocked_path;
use ralph_server::chat::memory::Store;
use std::io;

pub fn run(direct: Direct, ctx: &LocalRoute) {
    let monitor = ctx.monitor;
    let known: Vec<String> = monitor.status().workers.into_iter().map(|w| w.id).collect();
    let blocked = |id: &str| blocked_path(&monitor.store_dir, id).exists();
    let direct = match resolve_direct(direct, &known, blocked) {
        Ok(direct) => direct,
        Err(line) => return println!("{line}"),
    };
    let result = match deliver(&direct, monitor) {
        Ok(result) => result,
        Err(error) => return println!("{}", failure_line(&direct.id, &error.to_string())),
    };
    println!("{result}");
    let entry = memory_line(&direct, &result);
    if let Err(error) = Store::new(ctx.store.to_path_buf()).append(Role::User, &entry) {
        println!(
            "{}",
            failure_line(&direct.id, &format!("not recorded in memory: {error}"))
        );
    }
}

fn deliver(direct: &Direct, monitor: &Monitor) -> io::Result<String> {
    if let Some(refusal) = reply_refusal(&direct.id, &monitor.schedule.load_all()) {
        return Err(io::Error::other(refusal));
    }
    monitor.workers.reply(&direct.id, &direct.text, false, None)
}
