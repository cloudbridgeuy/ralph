use crate::worker::{Resume, SessionRecord};
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub struct ResumeFacts {
    pub owned: bool,
    pub worktree_exists: bool,
}

pub fn codex_resume_decision(
    id: &str,
    record: &SessionRecord,
    store: &Path,
    facts: ResumeFacts,
) -> Resume {
    let skip = |why: String| Resume::Skip(format!("codex session {id} not resumed: {why}"));
    if facts.owned {
        return skip("a running chat owns it".to_string());
    }
    match record.store.as_deref() {
        None => skip("the record has no owner".to_string()),
        Some(owner) if owner != store => skip(format!(
            "it belongs to the chat store {}, this chat uses {}",
            owner.display(),
            store.display()
        )),
        Some(_) if !facts.worktree_exists => {
            skip(format!("its worktree {} is gone", record.cwd.display()))
        }
        Some(_) => Resume::Go,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(store: Option<&str>) -> SessionRecord {
        SessionRecord {
            session_id: "t".into(),
            cwd: "/gone".into(),
            inbox: "/i".into(),
            store: store.map(Into::into),
            model: None,
            effort: None,
            permissions: None,
        }
    }

    const FREE: ResumeFacts = ResumeFacts {
        owned: false,
        worktree_exists: true,
    };

    #[test]
    fn resume_needs_a_free_worker_in_this_store_with_its_worktree() {
        let store = Path::new("/store");
        let go = codex_resume_decision("a", &record(Some("/store")), store, FREE);
        assert_eq!(go, Resume::Go);
        let cases = [
            (
                record(Some("/store")),
                ResumeFacts {
                    owned: true,
                    ..FREE
                },
                "running chat owns",
            ),
            (record(None), FREE, "no owner"),
            (record(Some("/other")), FREE, "/other"),
            (
                record(Some("/store")),
                ResumeFacts {
                    worktree_exists: false,
                    ..FREE
                },
                "worktree /gone is gone",
            ),
        ];
        for (rec, facts, why) in cases {
            match codex_resume_decision("a", &rec, store, facts) {
                Resume::Skip(message) => {
                    assert!(message.starts_with("codex session a not resumed: "));
                    assert!(message.contains(why), "{message}");
                }
                Resume::Go => panic!("must skip"),
            }
        }
    }
}
