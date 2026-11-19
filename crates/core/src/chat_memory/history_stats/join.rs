use super::config::StatsConfig;
use super::parse::{TimingTurn, TurnRow};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Join {
    pub timing_present: bool,
    pub total_turns: usize,
    pub joined: usize,
    pub no_history_calls: usize,
    pub unjoined_timing: usize,
    pub history_without_timing: usize,
}

fn nearest_timing(
    row: &TurnRow,
    timing: &[TimingTurn],
    taken: &[bool],
    window: i64,
) -> Option<usize> {
    let run = row.run.as_deref()?;
    timing
        .iter()
        .enumerate()
        .filter(|(slot, other)| !taken[*slot] && other.run.as_deref() == Some(run))
        .map(|(slot, other)| (slot, other.at.abs_diff(row.at)))
        .filter(|(_, distance)| i64::try_from(*distance).is_ok_and(|gap| gap <= window))
        .min_by_key(|(_, distance)| *distance)
        .map(|(slot, _)| slot)
}

pub fn join_turns(history: &[TurnRow], timing: &[TimingTurn], cfg: &StatsConfig) -> Join {
    let silent = |row: &&TurnRow| row.history_calls == Some(0);
    if timing.is_empty() {
        return Join {
            timing_present: false,
            total_turns: history.len(),
            no_history_calls: history.iter().filter(silent).count(),
            ..Join::default()
        };
    }
    let mut taken = vec![false; timing.len()];
    let mut joined = 0;
    let mut no_history_calls = 0;
    for row in history {
        let Some(slot) = nearest_timing(row, timing, &taken, cfg.join_secs) else {
            continue;
        };
        taken[slot] = true;
        joined += 1;
        if silent(&row) {
            no_history_calls += 1;
        }
    }
    Join {
        timing_present: true,
        total_turns: timing.len(),
        joined,
        no_history_calls,
        unjoined_timing: timing.len() - joined,
        history_without_timing: history.len() - joined,
    }
}
