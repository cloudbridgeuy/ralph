use crate::cursor_owner::{signal_group, TERM_GRACE, TERM_POLL};
use crate::process_tree::group_table;
use ralph_core::cursor_leftovers::socket_project_dir;
use ralph_core::worker::{is_worker_server, lsof_socket_paths, worker_server_groups, ServerRow};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Instant;

fn held_sockets(pid: u32) -> Vec<String> {
    Command::new("lsof")
        .args(["-nP", "-p", &pid.to_string(), "-a", "-U", "-Fn"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|output| lsof_socket_paths(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default()
}

fn group_running(group: u32) -> bool {
    group_table().iter().any(|row| row.pgid == group)
}

fn stop_group(group: u32) {
    signal_group(group, "TERM");
    let deadline = Instant::now() + TERM_GRACE;
    while Instant::now() < deadline && group_running(group) {
        thread::sleep(TERM_POLL);
    }
    if group_running(group) {
        signal_group(group, "KILL");
    }
}

pub(crate) fn sweep_worker_server(projects_base: &Path, real_worktree: &Path) -> Vec<u32> {
    let Some(socket_dir) = socket_project_dir(projects_base, &real_worktree.to_string_lossy())
    else {
        return Vec::new();
    };
    let table = group_table();
    let rows: Vec<ServerRow> = table
        .iter()
        .filter(|row| is_worker_server(&row.command))
        .map(|row| ServerRow {
            pid: row.pid,
            pgid: row.pgid,
            command: row.command.clone(),
            sockets: held_sockets(row.pid),
        })
        .collect();
    let groups = worker_server_groups(&rows, &table, &socket_dir);
    for group in &groups {
        stop_group(*group);
    }
    groups
}
