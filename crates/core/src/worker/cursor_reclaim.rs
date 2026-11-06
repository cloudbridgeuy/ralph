use super::GroupRow;
use std::path::Path;

const CURSOR_INSTALL_MARK: &str = "cursor-agent/versions/";
const WORKER_SERVER_SUFFIX: &str = "worker-server";
const WORKER_SOCKET: &str = "worker.sock";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerRow {
    pub pid: u32,
    pub pgid: u32,
    pub command: String,
    pub sockets: Vec<String>,
}

pub fn is_worker_server(command: &str) -> bool {
    command.contains(CURSOR_INSTALL_MARK) && command.trim_end().ends_with(WORKER_SERVER_SUFFIX)
}

pub fn worker_server_groups(rows: &[ServerRow], socket_dir: &Path) -> Vec<u32> {
    let mut groups: Vec<u32> = Vec::new();
    for row in rows {
        let holds_socket = row.sockets.iter().any(|socket| {
            Path::new(socket)
                .file_name()
                .is_some_and(|name| name == WORKER_SOCKET)
                && Path::new(socket).parent() == Some(socket_dir)
        });
        if row.pgid > 1
            && is_worker_server(&row.command)
            && holds_socket
            && !groups.contains(&row.pgid)
        {
            groups.push(row.pgid);
        }
    }
    groups
}

pub fn lsof_socket_paths(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.strip_prefix('n'))
        .map(str::to_string)
        .collect()
}

pub fn cursor_group_reclaimable(rows: &[GroupRow], group: u32) -> bool {
    let members: Vec<&GroupRow> = rows.iter().filter(|row| row.pgid == group).collect();
    !members.is_empty()
        && members
            .iter()
            .filter(|row| !members.iter().any(|parent| parent.pid == row.ppid))
            .all(|root| root.command.contains(CURSOR_INSTALL_MARK))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::{group_live, sweep_decision, OwnerFacts, Sweep};

    const CURSOR_NODE: &str = "/u/.local/share/cursor-agent/versions/2026.10.22-27dcf5f/node /u/.local/share/cursor-agent/versions/2026.10.22-27dcf5f/index.js worker-server";

    fn row(pid: u32, ppid: u32, pgid: u32, command: &str) -> GroupRow {
        GroupRow {
            pid,
            ppid,
            pgid,
            command: command.to_string(),
        }
    }

    fn probe_rows() -> Vec<GroupRow> {
        vec![
            row(1, 0, 1, "/sbin/launchd"),
            row(45373, 1, 44233, CURSOR_NODE),
            row(
                45577,
                45373,
                44233,
                "npm exec typescript-language-server --stdio",
            ),
            row(
                45772,
                45577,
                44233,
                "node /opt/homebrew/bin/typescript-language-server --stdio",
            ),
            row(
                45928,
                45772,
                44233,
                "node tsserver.js --serverMode partialSemantic",
            ),
            row(45929, 45772, 44233, "node tsserver.js"),
            row(45930, 45929, 44233, "node typingsInstaller.js"),
            row(50000, 1, 50000, "/usr/bin/vim"),
        ]
    }

    #[test]
    fn orphaned_cursor_group_with_language_server_children_is_reclaimable() {
        assert!(cursor_group_reclaimable(&probe_rows(), 44233));
    }

    #[test]
    fn group_with_a_root_outside_cursor_is_not_reclaimable() {
        let mut rows = probe_rows();
        rows.push(row(46000, 1, 44233, "/usr/bin/vim"));
        assert!(!cursor_group_reclaimable(&rows, 44233));
        assert!(!cursor_group_reclaimable(&probe_rows(), 50000));
    }

    #[test]
    fn empty_group_is_not_reclaimable() {
        assert!(!cursor_group_reclaimable(&probe_rows(), 44233 + 1));
        assert!(!cursor_group_reclaimable(&[], 44233));
    }

    #[test]
    fn dead_owner_chat_with_a_dead_leader_kills_its_orphaned_cursor_group() {
        let alive = group_live(None, "t", cursor_group_reclaimable(&probe_rows(), 44233));
        let facts = OwnerFacts {
            chat_alive: false,
            group_alive: alive,
            mine: false,
        };
        assert_eq!(sweep_decision(facts), Sweep::Reclaim { kill_group: true });
    }

    const SOCKET_DIR: &str = "/h/.cursor/projects/p-wt";

    fn server(pid: u32, pgid: u32, command: &str, sockets: &[&str]) -> ServerRow {
        ServerRow {
            pid,
            pgid,
            command: command.to_string(),
            sockets: sockets.iter().map(|socket| socket.to_string()).collect(),
        }
    }

    fn selected(rows: &[ServerRow]) -> Vec<u32> {
        worker_server_groups(rows, Path::new(SOCKET_DIR))
    }

    #[test]
    fn server_holding_the_worktree_socket_selects_its_group() {
        let sockets = ["/h/.cursor/projects/p-wt/worker.sock"];
        assert_eq!(
            selected(&[server(45373, 44233, CURSOR_NODE, &sockets)]),
            [44233]
        );
    }

    #[test]
    fn server_on_another_socket_dir_is_not_selected() {
        let other = ["/h/.cursor/projects/p-wt2/worker.sock"];
        assert!(selected(&[server(1, 44233, CURSOR_NODE, &other)]).is_empty());
        assert!(selected(&[server(1, 44233, CURSOR_NODE, &[])]).is_empty());
    }

    #[test]
    fn wrong_command_or_foreign_process_is_not_selected() {
        let socket = ["/h/.cursor/projects/p-wt/worker.sock"];
        let no_suffix = CURSOR_NODE.replace("worker-server", "serve");
        let foreign = "/usr/bin/node /x/index.js worker-server";
        assert!(selected(&[server(1, 44233, &no_suffix, &socket)]).is_empty());
        assert!(selected(&[server(1, 44233, foreign, &socket)]).is_empty());
        assert!(selected(&[server(1, 44233, "/usr/bin/vim", &socket)]).is_empty());
    }

    #[test]
    fn empty_rows_select_nothing() {
        assert!(selected(&[]).is_empty());
    }

    #[test]
    fn system_groups_are_never_selected() {
        let socket = ["/h/.cursor/projects/p-wt/worker.sock"];
        assert!(selected(&[server(1, 0, CURSOR_NODE, &socket)]).is_empty());
        assert!(selected(&[server(1, 1, CURSOR_NODE, &socket)]).is_empty());
    }

    #[test]
    fn two_servers_in_one_group_select_it_once() {
        let socket = ["/h/.cursor/projects/p-wt/worker.sock"];
        let rows = [
            server(2, 44233, CURSOR_NODE, &socket),
            server(3, 44233, CURSOR_NODE, &socket),
        ];
        assert_eq!(selected(&rows), [44233]);
    }

    #[test]
    fn lsof_output_lists_socket_paths() {
        let output = "p45373\nf12\nn/h/.cursor/projects/p-wt/worker.sock\nf13\nn->0xabc\n";
        assert_eq!(
            lsof_socket_paths(output),
            ["/h/.cursor/projects/p-wt/worker.sock", "->0xabc"]
        );
        assert!(lsof_socket_paths("").is_empty());
    }
}
