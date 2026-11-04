use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessId {
    #[default]
    Claude,
    Cursor,
    Opencode,
    Codex,
    #[serde(rename = "claude-headless")]
    ClaudeHeadless,
}

impl HarnessId {
    pub const ALL: [HarnessId; 5] = [
        Self::Claude,
        Self::Opencode,
        Self::Cursor,
        Self::Codex,
        Self::ClaudeHeadless,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Cursor => "cursor",
            Self::Opencode => "opencode",
            Self::Codex => "codex",
            Self::ClaudeHeadless => "claude-headless",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LiveHarness {
    pub cursor: bool,
    pub codex: bool,
    pub claude_headless: bool,
    pub opencode: bool,
}

pub fn close_harness(recorded: Option<HarnessId>, live: LiveHarness) -> HarnessId {
    if live.cursor {
        HarnessId::Cursor
    } else if live.codex {
        HarnessId::Codex
    } else if live.claude_headless {
        HarnessId::ClaudeHeadless
    } else if live.opencode {
        HarnessId::Opencode
    } else {
        recorded.unwrap_or_default()
    }
}

impl<'de> Deserialize<'de> for HarnessId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum Kind {
            Claude,
            Tmux,
            Cursor,
            Opencode,
            Codex,
            #[serde(rename = "claude-headless")]
            ClaudeHeadless,
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Compatible {
            Kind(Kind),
            Cursor(bool),
        }

        Ok(match Compatible::deserialize(deserializer)? {
            Compatible::Kind(Kind::Claude | Kind::Tmux) | Compatible::Cursor(false) => Self::Claude,
            Compatible::Kind(Kind::Cursor) | Compatible::Cursor(true) => Self::Cursor,
            Compatible::Kind(Kind::Opencode) => Self::Opencode,
            Compatible::Kind(Kind::Codex) => Self::Codex,
            Compatible::Kind(Kind::ClaudeHeadless) => Self::ClaudeHeadless,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::turn_count;

    #[test]
    fn kinds_round_trip_with_stable_names() {
        for (kind, name) in [
            (HarnessId::Claude, "claude"),
            (HarnessId::Cursor, "cursor"),
            (HarnessId::Opencode, "opencode"),
            (HarnessId::Codex, "codex"),
            (HarnessId::ClaudeHeadless, "claude-headless"),
        ] {
            let json = format!("\"{name}\"");
            assert_eq!(serde_json::to_string(&kind).unwrap(), json);
            assert_eq!(serde_json::from_str::<HarnessId>(&json).unwrap(), kind);
        }
    }

    #[test]
    fn legacy_cursor_flags_load() {
        assert_eq!(
            serde_json::from_str::<HarnessId>("true").unwrap(),
            HarnessId::Cursor
        );
        assert_eq!(
            serde_json::from_str::<HarnessId>("false").unwrap(),
            HarnessId::Claude
        );
        assert_eq!(
            serde_json::from_str::<HarnessId>("\"tmux\"").unwrap(),
            HarnessId::Claude
        );
        assert_eq!(HarnessId::default(), HarnessId::Claude);
    }

    #[test]
    fn invalid_kinds_are_rejected() {
        for json in ["null", "1", "{}", "\"unknown\""] {
            assert!(serde_json::from_str::<HarnessId>(json).is_err());
        }
    }

    fn probes(cursor: bool, codex: bool, claude_headless: bool, opencode: bool) -> LiveHarness {
        LiveHarness {
            cursor,
            codex,
            claude_headless,
            opencode,
        }
    }

    #[test]
    fn a_dead_claude_headless_worker_with_a_record_keeps_its_harness() {
        let kind = close_harness(
            Some(HarnessId::ClaudeHeadless),
            probes(false, false, false, false),
        );
        assert_eq!(kind, HarnessId::ClaudeHeadless);
        assert_eq!(turn_count(kind, &[0]), None);
    }

    #[test]
    fn without_a_record_the_live_order_holds() {
        assert_eq!(
            close_harness(None, probes(false, false, false, false)),
            HarnessId::Claude
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
            HarnessId::ClaudeHeadless
        );
        assert_eq!(
            close_harness(None, probes(false, false, false, true)),
            HarnessId::Opencode
        );
    }

    #[test]
    fn a_live_probe_wins_over_the_record() {
        assert_eq!(
            close_harness(Some(HarnessId::Claude), probes(false, true, false, false)),
            HarnessId::Codex
        );
        assert_eq!(
            close_harness(
                Some(HarnessId::ClaudeHeadless),
                probes(true, false, false, false)
            ),
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
