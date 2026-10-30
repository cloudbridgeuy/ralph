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
}
