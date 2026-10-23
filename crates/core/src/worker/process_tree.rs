pub fn parse_ps(output: &str) -> Vec<(u32, u32)> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let parent = fields.next()?.parse().ok()?;
            Some((pid, parent))
        })
        .collect()
}

pub fn descendants(rows: &[(u32, u32)], root: u32) -> Vec<u32> {
    let mut found = vec![root];
    let mut next = 0;
    while next < found.len() {
        let parent = found[next];
        next += 1;
        for (pid, ppid) in rows {
            if *ppid == parent && !found.contains(pid) {
                found.push(*pid);
            }
        }
    }
    found.remove(0);
    found
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tracked {
    pub pid: u32,
    pub start: String,
}

pub fn still_running(snapshot: &[Tracked], start_of: impl Fn(u32) -> Option<String>) -> Vec<u32> {
    snapshot
        .iter()
        .filter(|tracked| start_of(tracked.pid).is_some_and(|start| start == tracked.start))
        .map(|tracked| tracked.pid)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PS_OUTPUT: &str = "    1     0\n  100     1\n  200   100\n  300   200\n  301   200\n  400     1\n  500   300\nbogus line\n";

    #[test]
    fn parse_ps_reads_pid_and_parent_rows() {
        let rows = parse_ps(PS_OUTPUT);
        assert_eq!(rows.len(), 7);
        assert_eq!(rows[2], (200, 100));
        assert_eq!(parse_ps(""), Vec::new());
    }

    #[test]
    fn descendants_walk_the_whole_tree_without_the_root() {
        let rows = parse_ps(PS_OUTPUT);
        assert_eq!(descendants(&rows, 100), vec![200, 300, 301, 500]);
        assert_eq!(descendants(&rows, 301), Vec::<u32>::new());
        assert_eq!(descendants(&rows, 999), Vec::<u32>::new());
    }

    #[test]
    fn descendants_survive_a_parent_cycle() {
        let rows = vec![(2, 1), (3, 2), (2, 3)];
        assert_eq!(descendants(&rows, 1), vec![2, 3]);
    }

    #[test]
    fn only_pids_with_an_unchanged_start_time_are_still_running() {
        let tracked = |pid, start: &str| Tracked {
            pid,
            start: start.to_string(),
        };
        let snapshot = vec![tracked(10, "a"), tracked(11, "b"), tracked(12, "c")];
        let now = |pid| match pid {
            10 => Some("a".to_string()),
            11 => Some("reused".to_string()),
            _ => None,
        };
        assert_eq!(still_running(&snapshot, now), vec![10]);
    }
}
