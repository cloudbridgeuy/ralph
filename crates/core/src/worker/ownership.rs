use super::{branch_name, OPENCODE_HARNESS};
use std::path::Path;

pub const MAIN_BRANCH: &str = "main";

pub fn project_slug(repo: &Path) -> String {
    let slug: String = repo
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    if slug.is_empty() {
        "repo".to_string()
    } else {
        slug
    }
}

pub fn session_name(project: &str, id: &str) -> String {
    format!("ralph-{project}-{id}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaggedSession {
    pub name: String,
    pub store: String,
    pub opencode: bool,
    pub id: String,
}

pub fn tagged_sessions(listing: &str) -> Vec<TaggedSession> {
    listing
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t').map(str::trim);
            let name = fields.next().filter(|name| !name.is_empty())?;
            Some(TaggedSession {
                name: name.to_string(),
                store: fields.next().unwrap_or_default().to_string(),
                opencode: fields.next() == Some(OPENCODE_HARNESS),
                id: fields.next().unwrap_or_default().to_string(),
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Owner {
    Missing,
    Mine(TaggedSession),
    Foreign(String),
}

pub fn session_owner(sessions: &[TaggedSession], store: &str, project: &str, id: &str) -> Owner {
    let names = [session_name(project, id), format!("ralph-{id}")];
    let found: Vec<&TaggedSession> = sessions
        .iter()
        .filter(|session| names.contains(&session.name))
        .collect();
    match found.iter().find(|session| session.store != store) {
        Some(foreign) => Owner::Foreign(foreign.store.clone()),
        None => found
            .first()
            .map_or(Owner::Missing, |mine| Owner::Mine((*mine).clone())),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseState {
    pub id: String,
    pub owner: Owner,
    pub dirty_worktree: Option<bool>,
    pub unmerged_commits: Option<usize>,
    pub opencode: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseStep {
    RemoveOpencodeSession,
    KillSession(String),
    RemoveWorktree { force: bool },
    DeleteBranch,
    PruneWorktrees,
    EvictOpencodeLocation,
    ClearReady,
}

pub fn close_plan(state: &CloseState, abandon: bool) -> Result<Vec<CloseStep>, String> {
    let id = &state.id;
    if let Owner::Foreign(store) = &state.owner {
        return Err(format!(
            "Worker {id} belongs to another chat (store {store}). Not closing it."
        ));
    }
    let nothing_left = state.owner == Owner::Missing
        && state.dirty_worktree.is_none()
        && state.unmerged_commits.is_none();
    if nothing_left {
        return Err(format!(
            "Worker {id} has no tmux session, worktree or branch to close."
        ));
    }
    if !abandon {
        if let Some(count @ 1..) = state.unmerged_commits {
            return Err(format!(
                "Branch {} has {count} commit(s) not in {MAIN_BRANCH}. Merge it first, or close with abandon to drop them.",
                branch_name(id)
            ));
        }
        if state.dirty_worktree == Some(true) {
            return Err(format!(
                "Worker {id}'s worktree has uncommitted changes. Commit them, or close with abandon to drop them."
            ));
        }
    }
    let mut steps = Vec::new();
    if state.opencode {
        steps.push(CloseStep::RemoveOpencodeSession);
    }
    if let Owner::Mine(session) = &state.owner {
        steps.push(CloseStep::KillSession(session.name.clone()));
    }
    if let Some(dirty) = state.dirty_worktree {
        steps.push(CloseStep::RemoveWorktree { force: dirty });
    }
    if state.unmerged_commits.is_some() {
        steps.push(CloseStep::DeleteBranch);
    }
    steps.push(CloseStep::PruneWorktrees);
    if state.opencode {
        steps.push(CloseStep::EvictOpencodeLocation);
    }
    steps.push(CloseStep::ClearReady);
    Ok(steps)
}

pub fn opencode_session_gone(stderr: &str) -> bool {
    stderr.contains("HTTP 404")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_slug_keeps_letters_digits_and_dashes() {
        assert_eq!(project_slug(Path::new("/x/ralph")), "ralph");
        assert_eq!(project_slug(Path::new("/x/my repo.v2:")), "myrepov2");
        assert_eq!(project_slug(Path::new("/x/mcp-tools")), "mcp-tools");
        assert_eq!(project_slug(Path::new("/x/...")), "repo");
        assert_eq!(project_slug(Path::new("/")), "repo");
    }

    #[test]
    fn session_names_differ_per_project_for_the_same_id() {
        assert_eq!(session_name("ralph", "GUZ-1"), "ralph-ralph-GUZ-1");
        assert_ne!(
            session_name("ralph", "GUZ-1"),
            session_name("mcptools", "GUZ-1")
        );
    }

    fn tagged(name: &str, store: &str) -> TaggedSession {
        TaggedSession {
            name: name.into(),
            store: store.into(),
            opencode: false,
            id: "A".into(),
        }
    }

    #[test]
    fn tagged_sessions_read_name_store_harness_and_id() {
        let sessions = tagged_sessions("ralph-p-A\t/s\topencode\tA\nmain\n\n");
        assert_eq!(
            sessions,
            vec![
                TaggedSession {
                    name: "ralph-p-A".into(),
                    store: "/s".into(),
                    opencode: true,
                    id: "A".into()
                },
                TaggedSession {
                    name: "main".into(),
                    store: "".into(),
                    opencode: false,
                    id: "".into()
                }
            ]
        );
    }

    #[test]
    fn session_owner_compares_the_store_tag() {
        let mine = tagged("ralph-p-A", "/s");
        assert_eq!(
            session_owner(std::slice::from_ref(&mine), "/s", "p", "A"),
            Owner::Mine(mine)
        );
        assert_eq!(
            session_owner(&[tagged("ralph-p-A", "/other")], "/s", "p", "A"),
            Owner::Foreign("/other".into())
        );
        assert_eq!(
            session_owner(&[tagged("ralph-A", "/other")], "/s", "p", "A"),
            Owner::Foreign("/other".into())
        );
        assert_eq!(
            session_owner(&[tagged("ralph-q-A", "/other")], "/s", "p", "A"),
            Owner::Missing
        );
        assert_eq!(
            session_owner(&[tagged("ralph-p-A", "")], "/s", "p", "A"),
            Owner::Foreign("".into())
        );
    }

    fn close_state(owner: Owner) -> CloseState {
        CloseState {
            id: "A".into(),
            owner,
            dirty_worktree: Some(false),
            unmerged_commits: Some(0),
            opencode: false,
        }
    }

    #[test]
    fn close_plan_runs_every_step_in_order_for_a_merged_opencode_worker() {
        let mut state = close_state(Owner::Mine(tagged("ralph-p-A", "/s")));
        state.opencode = true;
        assert_eq!(
            close_plan(&state, false),
            Ok(vec![
                CloseStep::RemoveOpencodeSession,
                CloseStep::KillSession("ralph-p-A".into()),
                CloseStep::RemoveWorktree { force: false },
                CloseStep::DeleteBranch,
                CloseStep::PruneWorktrees,
                CloseStep::EvictOpencodeLocation,
                CloseStep::ClearReady,
            ])
        );
    }

    #[test]
    fn close_plan_skips_what_is_already_gone() {
        let mut state = close_state(Owner::Missing);
        state.dirty_worktree = None;
        assert_eq!(
            close_plan(&state, false),
            Ok(vec![
                CloseStep::DeleteBranch,
                CloseStep::PruneWorktrees,
                CloseStep::ClearReady
            ])
        );
        state.unmerged_commits = None;
        assert!(close_plan(&state, true).is_err());
    }

    #[test]
    fn close_plan_refuses_foreign_workers_even_with_abandon() {
        let state = close_state(Owner::Foreign("/other".into()));
        let refusal = close_plan(&state, true).unwrap_err();
        assert!(refusal.contains("another chat"), "{refusal}");
    }

    #[test]
    fn close_plan_refuses_unmerged_or_dirty_work_unless_abandoned() {
        let mut unmerged = close_state(Owner::Missing);
        unmerged.unmerged_commits = Some(2);
        assert!(close_plan(&unmerged, false)
            .unwrap_err()
            .contains("2 commit(s) not in main"));
        assert!(close_plan(&unmerged, true).is_ok());

        let mut dirty = close_state(Owner::Missing);
        dirty.dirty_worktree = Some(true);
        assert!(close_plan(&dirty, false)
            .unwrap_err()
            .contains("uncommitted changes"));
        assert!(close_plan(&dirty, true)
            .unwrap()
            .contains(&CloseStep::RemoveWorktree { force: true }));
    }

    #[test]
    fn opencode_session_gone_on_404_only() {
        assert!(opencode_session_gone("HTTP 404 Not Found\n"));
        assert!(!opencode_session_gone("HTTP 500 Internal Server Error"));
    }
}
