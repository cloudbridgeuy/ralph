use super::AcpAgent;
use crate::worker::PermissionMode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerRecord {
    pub chat_pid: u32,
    pub chat_start: String,
    pub group: u32,
    pub group_start: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<PermissionMode>,
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

pub fn group_live(leader_start_now: Option<&str>, recorded_start: &str, reclaimable: bool) -> bool {
    leader_start_now.map_or(reclaimable, |start| start == recorded_start)
}

pub fn owned_by(record: &OwnerRecord, pid: u32, instance: Option<&str>) -> bool {
    match (record.instance.as_deref(), instance) {
        (Some(theirs), Some(mine)) => theirs == mine,
        _ => record.chat_pid == pid,
    }
}

pub fn turn_token(record: &OwnerRecord, generation: u64) -> String {
    let without_instance = OwnerRecord {
        instance: None,
        ..record.clone()
    };
    format!("{}#{generation}", without_instance.render())
}

pub fn caller_owns_worker(owner_chat_pid: u32, caller_ancestry: &[u32]) -> bool {
    caller_ancestry.contains(&owner_chat_pid)
}

#[derive(Debug, Default)]
pub struct SessionRegistry {
    opened: u64,
    open: HashMap<PathBuf, String>,
    closed: HashSet<String>,
}

impl SessionRegistry {
    pub fn register(&mut self, process: &str, store: PathBuf) -> String {
        let instance = format!("{process}#{}", self.opened);
        self.opened += 1;
        self.open.insert(store, instance.clone());
        instance
    }

    pub fn unregister(&mut self, store: &Path, instance: &str) {
        if self.open.get(store).is_some_and(|open| open == instance) {
            self.open.remove(store);
        }
        self.closed.insert(instance.to_string());
    }

    pub fn owner_instance<'a>(&'a self, store: &Path, process: Option<&'a str>) -> Option<&'a str> {
        self.open.get(store).map(String::as_str).or(process)
    }

    pub fn closed_holder(&self, record: &OwnerRecord) -> bool {
        record
            .instance
            .as_ref()
            .is_some_and(|instance| self.closed.contains(instance))
    }
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
            effort: None,
            permissions: None,
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
        assert_eq!((parsed.effort, parsed.permissions), (None, None));
        let mut codex = record(Some("/store"));
        codex.effort = Some("high".into());
        codex.permissions = Some(PermissionMode::Yolo);
        assert!(codex.render().contains(r#""permissions":"yolo""#));
        assert_eq!(SessionRecord::parse(&codex.render()), Some(codex));
        assert!(!owned_json().contains("effort"));
        assert_eq!(SessionRecord::parse("nope"), None);
    }

    fn owned_json() -> String {
        record(Some("/store")).render()
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
    fn group_is_live_when_the_leader_matches_or_is_gone_with_a_reclaimable_group() {
        assert!(group_live(Some("t"), "t", false));
        assert!(!group_live(Some("reused"), "t", true));
        assert!(group_live(None, "t", true));
        assert!(!group_live(None, "t", false));
    }

    #[test]
    fn owner_record_round_trips_and_rejects_garbage() {
        let record = OwnerRecord {
            chat_pid: 1,
            chat_start: "Mon Oct 15 10:00:00 2026".into(),
            group: 2,
            group_start: "Mon Oct 15 10:00:01 2026".into(),
            instance: None,
        };
        assert_eq!(OwnerRecord::parse(&record.render()), Some(record));
        assert_eq!(OwnerRecord::parse("4242"), None);
    }

    const OLD_OWNER: &str = r#"{"chat_pid":1,"chat_start":"c","group":2,"group_start":"g"}"#;

    fn owner(chat_pid: u32, instance: Option<&str>) -> OwnerRecord {
        OwnerRecord {
            chat_pid,
            chat_start: "c".into(),
            group: 2,
            group_start: "g".into(),
            instance: instance.map(str::to_string),
        }
    }

    #[derive(Debug, PartialEq, Deserialize)]
    struct Main0475OwnerRecord {
        chat_pid: u32,
        chat_start: String,
        group: u32,
        group_start: String,
    }

    #[test]
    fn new_reader_parses_an_old_owner_record_and_renders_it_byte_for_byte() {
        assert_eq!(OwnerRecord::parse(OLD_OWNER), Some(owner(1, None)));
        assert_eq!(owner(1, None).render(), OLD_OWNER);
    }

    #[test]
    fn old_reader_parses_a_new_owner_record() {
        let new = owner(1, Some("1@c")).render();
        assert_eq!(
            new,
            r#"{"chat_pid":1,"chat_start":"c","group":2,"group_start":"g","instance":"1@c"}"#
        );
        assert_eq!(
            serde_json::from_str::<Main0475OwnerRecord>(&new).ok(),
            Some(Main0475OwnerRecord {
                chat_pid: 1,
                chat_start: "c".into(),
                group: 2,
                group_start: "g".into(),
            })
        );
    }

    #[test]
    fn owned_by_matches_the_instance_when_both_have_one_else_the_pid() {
        assert!(owned_by(&owner(1, Some("1@c")), 1, Some("1@c")));
        assert!(!owned_by(&owner(1, Some("1@c")), 1, Some("1@reused")));
        assert!(owned_by(&owner(1, None), 1, Some("1@c")));
        assert!(owned_by(&owner(1, Some("1@c")), 1, None));
        assert!(!owned_by(&owner(1, None), 9, Some("9@c")));
    }

    #[test]
    fn turn_token_of_an_old_record_keeps_the_old_bytes() {
        assert_eq!(turn_token(&owner(1, None), 3), format!("{OLD_OWNER}#3"));
    }

    #[test]
    fn turn_token_ignores_the_instance_so_it_survives_an_upgrade() {
        assert_eq!(
            turn_token(&owner(1, Some("1@c")), 3),
            turn_token(&owner(1, None), 3)
        );
    }

    #[test]
    fn caller_owns_worker_only_when_the_owner_is_an_ancestor() {
        assert!(caller_owns_worker(7078, &[71512, 71385, 7078, 1]));
        assert!(!caller_owns_worker(7078, &[71512, 71385, 9999, 1]));
        assert!(!caller_owns_worker(7078, &[]));
    }

    fn held_by(instance: Option<&str>) -> OwnerRecord {
        OwnerRecord {
            chat_pid: 7,
            chat_start: "s".to_string(),
            group: 8,
            group_start: "g".to_string(),
            instance: instance.map(str::to_string),
        }
    }

    #[test]
    fn each_opened_session_gets_the_next_number() {
        let mut sessions = SessionRegistry::default();
        assert_eq!(sessions.register("7@s", PathBuf::from("/a")), "7@s#0");
        assert_eq!(sessions.register("7@s", PathBuf::from("/b")), "7@s#1");
    }

    #[test]
    fn an_open_store_owns_as_its_session_and_other_stores_as_the_process() {
        let mut sessions = SessionRegistry::default();
        sessions.register("7@s", PathBuf::from("/a"));
        assert_eq!(
            sessions.owner_instance(Path::new("/a"), Some("7@s")),
            Some("7@s#0")
        );
        assert_eq!(
            sessions.owner_instance(Path::new("/b"), Some("7@s")),
            Some("7@s")
        );
    }

    #[test]
    fn a_reopened_store_reclaims_the_record_of_its_closed_session() {
        let mut sessions = SessionRegistry::default();
        let store = Path::new("/a");
        let first = sessions.register("7@s", store.to_path_buf());
        let record = held_by(Some(&first));
        sessions.unregister(store, &first);
        sessions.register("7@s", store.to_path_buf());
        let mine = owned_by(&record, 7, sessions.owner_instance(store, Some("7@s")));
        let facts = OwnerFacts {
            chat_alive: !sessions.closed_holder(&record),
            group_alive: true,
            mine,
        };
        assert!(!mine);
        assert_eq!(sweep_decision(facts), Sweep::Reclaim { kill_group: true });
    }

    #[test]
    fn a_late_close_of_the_old_session_keeps_its_successor_open() {
        let mut sessions = SessionRegistry::default();
        let store = Path::new("/a");
        let first = sessions.register("7@s", store.to_path_buf());
        let second = sessions.register("7@s", store.to_path_buf());
        sessions.unregister(store, &first);
        assert_eq!(
            sessions.owner_instance(store, Some("7@s")),
            Some(second.as_str())
        );
        assert!(!sessions.closed_holder(&held_by(Some(&second))));
        assert!(sessions.closed_holder(&held_by(Some(&first))));
    }

    #[test]
    fn records_of_no_closed_session_keep_process_liveness() {
        let mut sessions = SessionRegistry::default();
        let first = sessions.register("7@s", PathBuf::from("/a"));
        sessions.unregister(Path::new("/a"), &first);
        assert!(!sessions.closed_holder(&held_by(Some("7@s"))));
        assert!(!sessions.closed_holder(&held_by(Some("9@t#0"))));
        assert!(!sessions.closed_holder(&held_by(None)));
    }
}
