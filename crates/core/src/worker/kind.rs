use serde::{Deserialize, Deserializer, Serialize};

/// How a worker is hosted, independently of its agent/session metadata.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerKind {
    #[default]
    Tmux,
    Cursor,
    Opencode,
}

impl<'de> Deserialize<'de> for WorkerKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum Kind {
            Tmux,
            Cursor,
            Opencode,
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Compatible {
            Kind(Kind),
            Cursor(bool),
        }

        Ok(match Compatible::deserialize(deserializer)? {
            Compatible::Kind(Kind::Tmux) | Compatible::Cursor(false) => Self::Tmux,
            Compatible::Kind(Kind::Cursor) | Compatible::Cursor(true) => Self::Cursor,
            Compatible::Kind(Kind::Opencode) => Self::Opencode,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_with_stable_names() {
        for (kind, name) in [
            (WorkerKind::Tmux, "tmux"),
            (WorkerKind::Cursor, "cursor"),
            (WorkerKind::Opencode, "opencode"),
        ] {
            let json = format!("\"{name}\"");
            assert_eq!(serde_json::to_string(&kind).unwrap(), json);
            assert_eq!(serde_json::from_str::<WorkerKind>(&json).unwrap(), kind);
        }
    }

    #[test]
    fn legacy_cursor_flags_load() {
        assert_eq!(
            serde_json::from_str::<WorkerKind>("true").unwrap(),
            WorkerKind::Cursor
        );
        assert_eq!(
            serde_json::from_str::<WorkerKind>("false").unwrap(),
            WorkerKind::Tmux
        );
        assert_eq!(WorkerKind::default(), WorkerKind::Tmux);
    }

    #[test]
    fn invalid_kinds_are_rejected() {
        for json in ["null", "1", "{}", "\"unknown\""] {
            assert!(serde_json::from_str::<WorkerKind>(json).is_err());
        }
    }
}
