#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wait {
    Response,
    Memory,
    Thinking,
    Tool,
}

impl Wait {
    pub fn after_gap(pending_tools: usize) -> Wait {
        if pending_tools > 0 {
            Wait::Tool
        } else {
            Wait::Thinking
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnEvent {
    Rule,
    Warning(String),
    Busy(Wait),
    Resume(Wait),
    Idle,
    /// Stopgap: text the stream processor already rendered for the terminal.
    /// Step 3b replaces it with data events (OutputBlock / ParsedChunk) that the client renders.
    Output(String),
    Stderr(String),
}

pub trait TurnSink: Send + Sync {
    fn emit(&self, event: TurnEvent);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gap_with_no_pending_tool_is_thinking() {
        assert_eq!(Wait::after_gap(0), Wait::Thinking);
    }

    #[test]
    fn a_gap_with_a_pending_tool_is_a_tool_wait() {
        assert_eq!(Wait::after_gap(2), Wait::Tool);
    }
}
