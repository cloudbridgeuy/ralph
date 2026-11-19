#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatsConfig {
    pub gap_secs: u64,
    pub few_hits: u64,
    pub join_secs: i64,
    pub chain_min: usize,
}

impl Default for StatsConfig {
    fn default() -> StatsConfig {
        StatsConfig {
            gap_secs: 120,
            few_hits: 2,
            join_secs: 2,
            chain_min: 3,
        }
    }
}
