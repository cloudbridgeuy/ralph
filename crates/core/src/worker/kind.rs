use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HarnessId {
    #[default]
    Claude,
    ClaudeTmux,
    Cursor,
    Opencode,
    Codex,
}

impl HarnessId {
    pub const ALL: [HarnessId; 5] = [
        Self::ClaudeTmux,
        Self::Opencode,
        Self::Cursor,
        Self::Codex,
        Self::Claude,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::ClaudeTmux => "claude-tmux",
            Self::Cursor => "cursor",
            Self::Opencode => "opencode",
            Self::Codex => "codex",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LiveHarness {
    pub cursor: bool,
    pub codex: bool,
    pub claude: bool,
    pub opencode: bool,
}

pub fn close_harness(recorded: Option<HarnessId>, live: LiveHarness) -> HarnessId {
    if live.cursor {
        HarnessId::Cursor
    } else if live.codex {
        HarnessId::Codex
    } else if live.claude {
        HarnessId::Claude
    } else if live.opencode {
        HarnessId::Opencode
    } else {
        recorded.unwrap_or_default()
    }
}

pub fn close_kind(live: LiveHarness) -> HarnessId {
    close_harness(Some(HarnessId::ClaudeTmux), live)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::turn_count;

    #[test]
    fn kinds_round_trip_with_stable_names() {
        for (kind, name) in [
            (HarnessId::ClaudeTmux, "claude-tmux"),
            (HarnessId::Cursor, "cursor"),
            (HarnessId::Opencode, "opencode"),
            (HarnessId::Codex, "codex"),
            (HarnessId::Claude, "claude"),
        ] {
            let json = format!("\"{name}\"");
            assert_eq!(serde_json::to_string(&kind).unwrap(), json);
            assert_eq!(serde_json::from_str::<HarnessId>(&json).unwrap(), kind);
        }
    }

    #[test]
    fn the_default_harness_is_headless_claude() {
        assert_eq!(HarnessId::default(), HarnessId::Claude);
    }

    #[test]
    fn invalid_kinds_are_rejected() {
        for json in [
            "null",
            "1",
            "{}",
            "\"unknown\"",
            "true",
            "false",
            "\"tmux\"",
            "\"claude-headless\"",
        ] {
            assert!(serde_json::from_str::<HarnessId>(json).is_err());
        }
    }

    fn probes(cursor: bool, codex: bool, claude: bool, opencode: bool) -> LiveHarness {
        LiveHarness {
            cursor,
            codex,
            claude,
            opencode,
        }
    }

    #[test]
    fn a_dead_claude_worker_with_a_record_keeps_its_harness() {
        let kind = close_harness(Some(HarnessId::Claude), probes(false, false, false, false));
        assert_eq!(kind, HarnessId::Claude);
        assert_eq!(turn_count(kind, &[0]), None);
    }

    #[test]
    fn without_a_record_the_live_order_holds() {
        assert_eq!(
            close_harness(None, probes(false, false, false, false)),
            HarnessId::default()
        );
        assert_eq!(
            close_harness(None, probes(true, true, true, true)),
            HarnessId::Cursor
        );
        assert_eq!(
            close_harness(None, probes(false, true, true, true)),
            HarnessId::Codex
        );
        assert_eq!(
            close_harness(None, probes(false, false, true, true)),
            HarnessId::Claude
        );
        assert_eq!(
            close_harness(None, probes(false, false, false, true)),
            HarnessId::Opencode
        );
    }

    #[test]
    fn a_dead_worker_with_no_probe_closes_as_claude_tmux() {
        assert_eq!(
            close_kind(probes(false, false, false, false)),
            HarnessId::ClaudeTmux
        );
        assert_eq!(
            close_kind(probes(false, false, true, false)),
            HarnessId::Claude
        );
    }

    #[test]
    fn a_live_probe_wins_over_the_record() {
        assert_eq!(
            close_harness(
                Some(HarnessId::ClaudeTmux),
                probes(false, true, false, false)
            ),
            HarnessId::Codex
        );
        assert_eq!(
            close_harness(Some(HarnessId::Claude), probes(true, false, false, false)),
            HarnessId::Cursor
        );
    }

    #[test]
    fn a_dead_worker_with_a_record_keeps_any_recorded_harness() {
        for kind in HarnessId::ALL {
            assert_eq!(
                close_harness(Some(kind), probes(false, false, false, false)),
                kind
            );
        }
    }
}
