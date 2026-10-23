use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SPAWN_RECORD_DIR: &str = "spawn";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tracker {
    Linear,
    Jira,
}

impl Tracker {
    fn name(self) -> &'static str {
        match self {
            Tracker::Linear => "linear",
            Tracker::Jira => "jira",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    pub key: String,
    pub tracker: Option<Tracker>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketKeys {
    #[serde(default)]
    pub linear: Vec<String>,
    #[serde(default)]
    pub jira: Vec<String>,
}

impl TicketKeys {
    fn tracker_of(&self, key: &str) -> Option<Tracker> {
        let (prefix, number) = key.rsplit_once('-')?;
        let numeric = !number.is_empty() && number.chars().all(|c| c.is_ascii_digit());
        if !numeric {
            None
        } else if self.linear.iter().any(|known| known == prefix) {
            Some(Tracker::Linear)
        } else if self.jira.iter().any(|known| known == prefix) {
            Some(Tracker::Jira)
        } else {
            None
        }
    }
}

pub fn resolve_ticket(id: &str, explicit: Option<&str>, keys: &TicketKeys) -> Option<Ticket> {
    let tracker = keys.tracker_of(explicit.unwrap_or(id));
    match explicit {
        Some(key) => Some(Ticket {
            key: key.to_string(),
            tracker,
        }),
        None => tracker.map(|tracker| Ticket {
            key: id.to_string(),
            tracker: Some(tracker),
        }),
    }
}

pub fn ticket_line(ticket: &Ticket) -> String {
    match ticket.tracker {
        Some(tracker) => format!("{} ({})", ticket.key, tracker.name()),
        None => ticket.key.clone(),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracker: Option<Tracker>,
    #[serde(default)]
    pub at: u64,
}

impl SpawnRecord {
    pub fn new(ticket: Option<Ticket>, at: u64) -> Self {
        Self {
            tracker: ticket.as_ref().and_then(|ticket| ticket.tracker),
            ticket: ticket.map(|ticket| ticket.key),
            at,
        }
    }

    pub fn ticket(&self) -> Option<Ticket> {
        self.ticket.as_ref().map(|key| Ticket {
            key: key.clone(),
            tracker: self.tracker,
        })
    }
}

pub fn spawn_record_path(store: &Path, id: &str) -> PathBuf {
    store.join(SPAWN_RECORD_DIR).join(id)
}

pub fn encode_spawn_record(record: &SpawnRecord) -> serde_json::Result<String> {
    serde_json::to_string(record)
}

pub fn decode_spawn_record(raw: &str) -> Option<SpawnRecord> {
    serde_json::from_str(raw).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> TicketKeys {
        TicketKeys {
            linear: vec!["GUZ".into()],
            jira: vec!["VP20".into(), "KODA".into()],
        }
    }

    fn ticket(key: &str, tracker: Option<Tracker>) -> Option<Ticket> {
        Some(Ticket {
            key: key.into(),
            tracker,
        })
    }

    #[test]
    fn an_id_with_a_linear_key_resolves_to_linear() {
        assert_eq!(
            resolve_ticket("GUZ-918", None, &keys()),
            ticket("GUZ-918", Some(Tracker::Linear))
        );
    }

    #[test]
    fn an_id_with_a_jira_key_resolves_to_jira() {
        assert_eq!(
            resolve_ticket("VP20-7", None, &keys()),
            ticket("VP20-7", Some(Tracker::Jira))
        );
        assert_eq!(
            resolve_ticket("KODA-1234", None, &keys()),
            ticket("KODA-1234", Some(Tracker::Jira))
        );
    }

    #[test]
    fn an_id_without_a_configured_key_has_no_ticket() {
        assert_eq!(resolve_ticket("FOO-1", None, &keys()), None);
        assert_eq!(resolve_ticket("fix-bug", None, &keys()), None);
        assert_eq!(
            resolve_ticket("GUZ-918", None, &TicketKeys::default()),
            None
        );
    }

    #[test]
    fn the_key_match_is_case_sensitive() {
        assert_eq!(resolve_ticket("guz-918", None, &keys()), None);
    }

    #[test]
    fn the_number_must_be_one_or_more_digits() {
        for id in ["GUZ-12a", "GUZ-", "GUZ", "GUZ-1-2a", "GUZ--1", "GUZ-+1"] {
            assert_eq!(resolve_ticket(id, None, &keys()), None, "{id}");
        }
    }

    #[test]
    fn the_key_must_match_whole() {
        assert_eq!(resolve_ticket("XGUZ-1", None, &keys()), None);
        assert_eq!(resolve_ticket("GU-1", None, &keys()), None);
    }

    #[test]
    fn an_explicit_ticket_wins_over_the_id() {
        assert_eq!(
            resolve_ticket("GUZ-1", Some("KODA-5"), &keys()),
            ticket("KODA-5", Some(Tracker::Jira))
        );
        assert_eq!(
            resolve_ticket("scratch", Some("GUZ-9"), &keys()),
            ticket("GUZ-9", Some(Tracker::Linear))
        );
    }

    #[test]
    fn an_explicit_unknown_ticket_is_kept_without_a_tracker() {
        assert_eq!(
            resolve_ticket("GUZ-1", Some("FOO-1"), &keys()),
            ticket("FOO-1", None)
        );
        assert_eq!(
            resolve_ticket("GUZ-1", Some("GUZ-1x"), &keys()),
            ticket("GUZ-1x", None)
        );
    }

    #[test]
    fn the_line_names_the_tracker_when_known() {
        assert_eq!(
            ticket_line(&Ticket {
                key: "GUZ-918".into(),
                tracker: Some(Tracker::Linear)
            }),
            "GUZ-918 (linear)"
        );
        assert_eq!(
            ticket_line(&Ticket {
                key: "KODA-1".into(),
                tracker: Some(Tracker::Jira)
            }),
            "KODA-1 (jira)"
        );
        assert_eq!(
            ticket_line(&Ticket {
                key: "FOO-1".into(),
                tracker: None
            }),
            "FOO-1"
        );
    }

    #[test]
    fn a_record_round_trips() {
        let record = SpawnRecord::new(ticket("GUZ-918", Some(Tracker::Linear)), 1_767_225_600);
        let encoded = encode_spawn_record(&record).unwrap();
        assert!(encoded.contains(r#""tracker":"linear""#), "{encoded}");
        assert_eq!(decode_spawn_record(&encoded), Some(record.clone()));
        assert_eq!(record.ticket(), ticket("GUZ-918", Some(Tracker::Linear)));
    }

    #[test]
    fn a_record_without_a_ticket_has_none() {
        let record = SpawnRecord::new(None, 5);
        assert_eq!(encode_spawn_record(&record).unwrap(), r#"{"at":5}"#);
        assert_eq!(record.ticket(), None);
    }

    #[test]
    fn a_record_with_missing_fields_decodes() {
        assert_eq!(
            decode_spawn_record("{}"),
            Some(SpawnRecord {
                ticket: None,
                tracker: None,
                at: 0
            })
        );
        assert_eq!(
            decode_spawn_record(r#"{"ticket":"FOO-1"}"#)
                .unwrap()
                .ticket(),
            ticket("FOO-1", None)
        );
    }

    #[test]
    fn unknown_fields_are_ignored_and_garbage_does_not_decode() {
        assert_eq!(
            decode_spawn_record(r#"{"at":9,"base_sha":"abc"}"#)
                .unwrap()
                .at,
            9
        );
        assert_eq!(decode_spawn_record("not json"), None);
        assert_eq!(decode_spawn_record(""), None);
    }

    #[test]
    fn the_record_sits_in_the_spawn_dir_under_the_worker_id() {
        assert_eq!(
            spawn_record_path(Path::new("/s"), "A"),
            PathBuf::from("/s/spawn/A")
        );
    }

    #[test]
    fn keys_default_to_empty_and_parse_from_toml_shaped_json() {
        assert_eq!(
            serde_json::from_str::<TicketKeys>("{}").unwrap(),
            TicketKeys::default()
        );
        let parsed: TicketKeys = serde_json::from_str(r#"{"jira":["KODA"]}"#).unwrap();
        assert_eq!(parsed.jira, ["KODA"]);
        assert!(parsed.linear.is_empty());
    }
}
