use crate::chat::memory::Store;
use crate::chat::monitor::Monitor;
use crate::chat::thread::{claim_closed, config_root, read_meta, thread_dir};
use crate::workers::running_workers;
use ralph_core::chat_switch::{switch_blockers, switch_decision, SwitchArgv, SwitchTarget};
use ralph_core::chat_thread::{ThreadName, THREAD_FILE};
use std::path::Path;

pub(crate) enum ChatExit {
    Quit,
    Switch(SwitchTarget, SwitchArgv),
}

pub(super) struct Leave<'a> {
    pub store_dir: &'a Path,
    pub current: Option<&'a ThreadName>,
    pub monitor: &'a Monitor,
}

pub(super) fn check(leave: &Leave, target: &SwitchTarget) -> Result<(), String> {
    let main = config_root(leave.store_dir);
    let known = match target {
        SwitchTarget::Thread(name) => {
            let dir = thread_dir(&main, name);
            if dir.join(THREAD_FILE).is_file() {
                let meta = read_meta(&dir).map_err(|e| format!("thread {name}: {e}"))?;
                Some((dir, meta))
            } else {
                None
            }
        }
        SwitchTarget::Main => None,
    };
    switch_decision(leave.current, target, known.as_ref().map(|(_, meta)| meta))?;
    let workers: Vec<String> = running_workers(&leave.monitor.store_dir)
        .into_iter()
        .map(|worker| worker.id)
        .collect();
    let watches: Vec<String> = leave
        .monitor
        .status()
        .watches
        .into_iter()
        .map(|watch| watch.id)
        .collect();
    switch_blockers(&workers, &watches)?;
    match (target, known) {
        (SwitchTarget::Thread(name), Some((dir, _))) => claim_closed(&dir, name).map(drop),
        (SwitchTarget::Main, _) => main_free(&main),
        (SwitchTarget::Thread(_), None) => Ok(()),
    }
}

fn main_free(main: &Path) -> Result<(), String> {
    Store::new(main.to_path_buf())
        .claim_chat()
        .map_err(|e| format!("could not claim main: {e}"))?
        .map(drop)
        .map_err(|holder| {
            let by = holder
                .map(|pid| format!(" by pid {pid}"))
                .unwrap_or_default();
            format!("main is in use{by}; close its chat first.")
        })
}
