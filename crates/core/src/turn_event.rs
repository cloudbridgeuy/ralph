use crate::stream::blocks::OutputBlock;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TurnEvent {
    Rule,
    Warning(String),
    Busy(Wait),
    Resume(Wait),
    Idle,
    Blocks(Vec<OutputBlock>),
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

    #[test]
    fn turn_events_round_trip_through_json() {
        let events = [
            TurnEvent::Rule,
            TurnEvent::Busy(Wait::Memory),
            TurnEvent::Resume(Wait::Tool),
            TurnEvent::Blocks(Vec::new()),
            TurnEvent::Stderr("x".into()),
        ];
        for event in events {
            let json = serde_json::to_string(&event).unwrap();
            assert_eq!(serde_json::from_str::<TurnEvent>(&json).unwrap(), event);
        }
    }
}
