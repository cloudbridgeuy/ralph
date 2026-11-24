use crate::chat::socket::{SocketFiles, SocketPlace};
use crate::cursor_owner::process_start;
use ralph_core::chat_socket::{
    chat_files_dir_pid, orphan_sidecar_fate, sidecar_pid, socket_fate, socket_file_pid,
    stale_root_sockets, SocketFacts, SocketFate, SocketOwner,
};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

fn entry_names(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default()
}

pub fn sweep_stale(place: &SocketPlace) {
    let store_entries = entry_names(place.store);
    let mut active_pids: BTreeSet<u32> = entry_names(place.root)
        .iter()
        .filter_map(|name| socket_file_pid(&place.key, name))
        .collect();
    active_pids.extend(
        store_entries
            .iter()
            .filter_map(|name| chat_files_dir_pid(name)),
    );
    let orphan_pids: BTreeSet<u32> = store_entries
        .iter()
        .filter_map(|name| sidecar_pid(name))
        .filter(|pid| !active_pids.contains(pid))
        .collect();
    for (pids, decide) in [
        (&active_pids, socket_fate as fn(SocketFacts) -> SocketFate),
        (&orphan_pids, orphan_sidecar_fate),
    ] {
        for &pid in pids {
            let files = SocketFiles::new(place, pid);
            if decide_fate(&files, pid, decide) == SocketFate::Delete {
                files.remove();
            }
        }
    }
    let root_names = entry_names(place.root);
    for name in stale_root_sockets(&root_names, |pid| process_start(pid).is_some()) {
        fs::remove_file(place.root.join(name)).ok();
    }
}

fn decide_fate(files: &SocketFiles, pid: u32, decide: fn(SocketFacts) -> SocketFate) -> SocketFate {
    let owner = fs::read_to_string(&files.sidecar)
        .ok()
        .and_then(|text| SocketOwner::parse(&text));
    let start_now = owner.as_ref().and_then(|owner| process_start(owner.pid));
    decide(SocketFacts {
        file_pid: pid,
        owner: owner.as_ref(),
        owner_start_now: start_now.as_deref(),
    })
}
