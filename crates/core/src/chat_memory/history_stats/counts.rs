use super::parse::{Call, Outcome, Source, Tool};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCounts {
    pub tool: Tool,
    pub total: usize,
    pub mcp: usize,
    pub slash: usize,
    pub cli: usize,
    pub other_source: usize,
    pub ok: usize,
    pub empty: usize,
    pub error: usize,
    pub other_outcome: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    pub tools: Vec<ToolCounts>,
}

pub fn percent(part: usize, whole: usize) -> u64 {
    if whole == 0 {
        return 0;
    }
    let part = u128::try_from(part).unwrap_or(u128::MAX);
    let whole = u128::try_from(whole).unwrap_or(u128::MAX);
    u64::try_from((part.saturating_mul(100) + whole / 2) / whole).unwrap_or(u64::MAX)
}

impl ToolCounts {
    pub fn empty_pct(&self) -> u64 {
        percent(self.empty, self.total)
    }

    pub fn error_pct(&self) -> u64 {
        percent(self.error, self.total)
    }
}

const TOOLS: [Tool; 4] = [Tool::Read, Tool::Zoom, Tool::Recall, Tool::Search];

impl ToolCounts {
    fn new(tool: Tool) -> ToolCounts {
        ToolCounts {
            tool,
            total: 0,
            mcp: 0,
            slash: 0,
            cli: 0,
            other_source: 0,
            ok: 0,
            empty: 0,
            error: 0,
            other_outcome: 0,
        }
    }

    fn add(&mut self, call: &Call) {
        self.total += 1;
        match call.source {
            Source::Mcp => self.mcp += 1,
            Source::Slash => self.slash += 1,
            Source::Cli => self.cli += 1,
            Source::Unknown => self.other_source += 1,
        }
        match call.outcome {
            Outcome::Ok => self.ok += 1,
            Outcome::Empty => self.empty += 1,
            Outcome::Error => self.error += 1,
            Outcome::Unknown => self.other_outcome += 1,
        }
    }
}

pub fn count_calls(calls: &[Call]) -> Counts {
    let mut tools: Vec<ToolCounts> = TOOLS.iter().map(|tool| ToolCounts::new(*tool)).collect();
    let mut other = ToolCounts::new(Tool::Unknown);
    for call in calls {
        match tools.iter_mut().find(|row| row.tool == call.tool) {
            Some(row) => row.add(call),
            None => other.add(call),
        }
    }
    if other.total > 0 {
        tools.push(other);
    }
    Counts { tools }
}
