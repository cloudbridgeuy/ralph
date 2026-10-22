use super::AcpAgent;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerRecord {
    pub chat_pid: u32,
    pub chat_start: String,
    pub group: u32,
    pub group_start: String,
}

impl OwnerRecord {
    pub fn parse(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }

    pub fn render(&self) -> String {
        json!(self).to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: String,
    pub cwd: PathBuf,
    pub inbox: PathBuf,
    #[serde(default)]
    pub store: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl SessionRecord {
    pub fn parse(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }

    pub fn render(&self) -> String {
        json!(self).to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resume {
    Go,
    Skip(String),
}

pub fn resume_decision(
    agent: &AcpAgent,
    id: &str,
    record: &SessionRecord,
    store: &Path,
    loads_session: bool,
) -> Resume {
    let skip =
        |why: String| Resume::Skip(format!("{} session {id} not resumed: {why}", agent.name));
    match record.store.as_deref() {
        None => skip("the record has no owner".to_string()),
        Some(owner) if owner != store => skip(format!(
            "it belongs to the chat store {}, this chat uses {}",
            owner.display(),
            store.display()
        )),
        Some(_) if !loads_session => skip("the agent does not support session/load".to_string()),
        Some(_) => Resume::Go,
    }
}

pub fn session_saved(turn: Result<&str, &str>, updated: bool) -> bool {
    turn.is_ok() && updated
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerFacts {
    pub chat_alive: bool,
    pub group_alive: bool,
    pub mine: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sweep {
    Reclaim { kill_group: bool },
    Wait,
    Busy,
}

pub fn sweep_decision(facts: OwnerFacts) -> Sweep {
    match (facts.chat_alive, facts.mine) {
        (true, true) => Sweep::Wait,
        (true, false) => Sweep::Busy,
        (false, _) => Sweep::Reclaim {
            kill_group: facts.group_alive,
        },
    }
}

pub fn agent_held(facts: OwnerFacts) -> bool {
    facts.group_alive && !matches!(sweep_decision(facts), Sweep::Reclaim { .. })
}

pub fn caller_owns_worker(owner_chat_pid: u32, caller_ancestry: &[u32]) -> bool {
    caller_ancestry.contains(&owner_chat_pid)
}

#[cfg(test)]
mod tests {
    use super::super::CURSOR_AGENT;
    use super::*;

    fn record(store: Option<&str>) -> SessionRecord {
        SessionRecord {
            session_id: "s1".to_string(),
            cwd: PathBuf::from("/w"),
            inbox: PathBuf::from("/store/inbox"),
            store: store.map(PathBuf::from),
            model: None,
        }
    }

    #[test]
    fn session_record_round_trips_and_old_records_have_no_owner() {
        let owned = record(Some("/store"));
        assert_eq!(SessionRecord::parse(&owned.render()), Some(owned));
        let mut with_model = record(Some("/store"));
        with_model.model = Some("gpt-5".into());
        assert_eq!(SessionRecord::parse(&with_model.render()), Some(with_model));
        let bare = r#"{"session_id":"s1","cwd":"/w","inbox":"/i"}"#;
        let parsed = SessionRecord::parse(bare).unwrap();
        assert_eq!(parsed.store, None);
        assert_eq!(parsed.model, None);
        assert_eq!(SessionRecord::parse("nope"), None);
    }

    #[test]
    fn resume_requires_this_store_and_the_load_capability() {
        let store = Path::new("/store");
        assert_eq!(
            resume_decision(&CURSOR_AGENT, "a", &record(Some("/store")), store, true),
            Resume::Go
        );
        for (rec, loads, why) in [
            (record(None), true, "no owner"),
            (record(Some("/other")), true, "/other"),
            (record(Some("/store")), false, "session/load"),
        ] {
            match resume_decision(&CURSOR_AGENT, "a", &rec, store, loads) {
                Resume::Skip(message) => {
                    assert!(message.starts_with("cursor session a not resumed: "));
                    assert!(message.contains(why), "{message}");
                }
                Resume::Go => panic!("must skip"),
            }
        }
    }

    #[test]
    fn session_is_saved_only_after_an_answered_turn_that_sent_an_update() {
        assert!(session_saved(Ok("cancelled"), true));
        assert!(session_saved(Ok("end_turn"), true));
        assert!(!session_saved(Ok("cancelled"), false));
        assert!(!session_saved(Err("boom"), true));
    }

    fn facts(chat_alive: bool, group_alive: bool, mine: bool) -> OwnerFacts {
        OwnerFacts {
            chat_alive,
            group_alive,
            mine,
        }
    }

    #[test]
    fn live_owner_chat_is_waited_on_when_mine_and_busy_when_foreign() {
        assert_eq!(sweep_decision(facts(true, true, true)), Sweep::Wait);
        assert_eq!(sweep_decision(facts(true, true, false)), Sweep::Busy);
        assert_eq!(sweep_decision(facts(true, false, false)), Sweep::Busy);
    }

    #[test]
    fn dead_owner_chat_is_reclaimed_and_the_group_killed_only_if_it_is_the_same_process() {
        assert_eq!(
            sweep_decision(facts(false, true, false)),
            Sweep::Reclaim { kill_group: true }
        );
        assert_eq!(
            sweep_decision(facts(false, false, false)),
            Sweep::Reclaim { kill_group: false }
        );
    }

    #[test]
    fn agent_is_held_only_while_the_owner_chat_and_the_group_are_alive() {
        assert!(!agent_held(facts(false, true, false)));
        assert!(agent_held(facts(true, true, true)));
        assert!(agent_held(facts(true, true, false)));
        assert!(!agent_held(facts(true, false, false)));
    }

    #[test]
    fn owner_record_round_trips_and_rejects_garbage() {
        let record = OwnerRecord {
            chat_pid: 1,
            chat_start: "Mon Oct 15 10:00:00 2026".into(),
            group: 2,
            group_start: "Mon Oct 15 10:00:01 2026".into(),
        };
        assert_eq!(OwnerRecord::parse(&record.render()), Some(record));
        assert_eq!(OwnerRecord::parse("4242"), None);
    }

    #[test]
    fn caller_owns_worker_only_when_the_owner_is_an_ancestor() {
        assert!(caller_owns_worker(7078, &[71512, 71385, 7078, 1]));
        assert!(!caller_owns_worker(7078, &[71512, 71385, 9999, 1]));
        assert!(!caller_owns_worker(7078, &[]));
    }
}
