use super::GroupRow;

const CURSOR_INSTALL_MARK: &str = "cursor-agent/versions/";

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
}
