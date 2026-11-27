use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudeMode {
    AcceptEdits,
    Auto,
    BypassPermissions,
    Default,
    DontAsk,
    Manual,
    Plan,
}

const CLAUDE_MODES: [ClaudeMode; 7] = [
    ClaudeMode::AcceptEdits,
    ClaudeMode::Auto,
    ClaudeMode::BypassPermissions,
    ClaudeMode::Default,
    ClaudeMode::DontAsk,
    ClaudeMode::Manual,
    ClaudeMode::Plan,
];

impl ClaudeMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AcceptEdits => "acceptEdits",
            Self::Auto => "auto",
            Self::BypassPermissions => "bypassPermissions",
            Self::Default => "default",
            Self::DontAsk => "dontAsk",
            Self::Manual => "manual",
            Self::Plan => "plan",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownClaudeMode(String);

impl fmt::Display for UnknownClaudeMode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "unknown claude permission mode {:?}; use one of {}",
            self.0,
            CLAUDE_MODES.map(ClaudeMode::as_str).join(", ")
        )
    }
}

impl std::error::Error for UnknownClaudeMode {}

impl FromStr for ClaudeMode {
    type Err = UnknownClaudeMode;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        CLAUDE_MODES
            .into_iter()
            .find(|mode| mode.as_str() == value)
            .ok_or_else(|| UnknownClaudeMode(value.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_parses_from_its_own_name_only() {
        for mode in CLAUDE_MODES {
            assert_eq!(mode.as_str().parse(), Ok(mode));
        }
        for value in ["", "Auto", "auto ", "yolo", "auto; id"] {
            assert!(value.parse::<ClaudeMode>().is_err(), "{value}");
        }
    }
}
