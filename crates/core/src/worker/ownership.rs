use super::{branch_name, OPENCODE_HARNESS};
use std::path::{Component, Path, PathBuf};

pub const FALLBACK_BASE_BRANCH: &str = "main";
const ORIGIN_PREFIX: &str = "refs/remotes/origin/";

pub fn base_config_key(id: &str) -> String {
    format!("branch.{}.ralphBase", branch_name(id))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base {
    pub branch: String,
    pub warning: Option<String>,
}

pub fn base_branch(
    configured: Result<Option<&str>, &str>,
    recorded: Option<&str>,
    origin_head: Option<&str>,
) -> Base {
    let (configured, config_error) = match configured {
        Ok(configured) => (configured, None),
        Err(error) => (None, Some(error)),
    };
    let origin_default = origin_head.map(|head| {
        head.trim()
            .strip_prefix(ORIGIN_PREFIX)
            .unwrap_or(head.trim())
    });
    let branch = [configured, recorded, origin_default]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|branch| !branch.is_empty())
        .unwrap_or(FALLBACK_BASE_BRANCH)
        .to_string();
    let warning = config_error.map(|error| {
        format!(
            "Warning: [chat] worker_base was skipped because the ralph config did not load ({error}). Unmerged commits were checked against {branch} instead."
        )
    });
    Base { branch, warning }
}

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
    Foreign(TaggedSession),
}

fn normalized(path: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(component);
                }
            }
            other => out.push(other),
        }
    }
    out
}

pub fn same_store(tag: &str, store: &str) -> bool {
    !tag.trim().is_empty()
        && !store.trim().is_empty()
        && normalized(tag.trim()) == normalized(store.trim())
}

pub fn session_owner(sessions: &[TaggedSession], store: &str, project: &str, id: &str) -> Owner {
    let names = [session_name(project, id), format!("ralph-{id}")];
    let found: Vec<&TaggedSession> = sessions
        .iter()
        .filter(|session| names.contains(&session.name))
        .collect();
    match found
        .iter()
        .find(|session| !same_store(&session.store, store))
    {
        Some(foreign) => Owner::Foreign((*foreign).clone()),
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
    pub base: String,
    pub unmerged_commits: Option<usize>,
    pub opencode: bool,
    pub markers: bool,
    pub cursor: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseStep {
    RemoveOpencodeSession,
    KillSession(String),
    TerminateCursor,
    RemoveWorktree { force: bool },
    DeleteBranch,
    PruneWorktrees,
    EvictOpencodeLocation,
    ClearMarkers,
}

pub fn close_plan(
    state: &CloseState,
    abandon: bool,
    force: bool,
) -> Result<Vec<CloseStep>, String> {
    let id = &state.id;
    if let (Owner::Foreign(session), false) = (&state.owner, force) {
        return Err(format!(
            "Worker {id} belongs to another chat (store {}). Not closing it.",
            session.store
        ));
    }
    let nothing_left = state.owner == Owner::Missing
        && state.dirty_worktree.is_none()
        && state.unmerged_commits.is_none();
    if nothing_left && state.markers {
        return Ok(vec![CloseStep::ClearMarkers]);
    }
    if nothing_left {
        return Err(format!(
            "Worker {id} has no tmux session, worktree or branch to close."
        ));
    }
    if !abandon {
        if let Some(count @ 1..) = state.unmerged_commits {
            return Err(format!(
                "Branch {} has {count} commit(s) not in {}. Merge it first, or close with abandon to drop them.",
                branch_name(id),
                state.base
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
    if state.cursor {
        steps.push(CloseStep::TerminateCursor);
    } else if let Owner::Mine(session) | Owner::Foreign(session) = &state.owner {
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
    steps.push(CloseStep::ClearMarkers);
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
            Owner::Foreign(tagged("ralph-p-A", "/other"))
        );
        assert_eq!(
            session_owner(&[tagged("ralph-A", "/other")], "/s", "p", "A"),
            Owner::Foreign(tagged("ralph-A", "/other"))
        );
        assert_eq!(
            session_owner(&[tagged("ralph-q-A", "/other")], "/s", "p", "A"),
            Owner::Missing
        );
        assert_eq!(
            session_owner(&[tagged("ralph-p-A", "")], "/s", "p", "A"),
            Owner::Foreign(tagged("ralph-p-A", ""))
        );
    }

    fn close_state(owner: Owner) -> CloseState {
        CloseState {
            id: "A".into(),
            owner,
            dirty_worktree: Some(false),
            base: "main".into(),
            unmerged_commits: Some(0),
            opencode: false,
            markers: false,
            cursor: false,
        }
    }

    #[test]
    fn close_plan_runs_every_step_in_order_for_a_merged_opencode_worker() {
        let mut state = close_state(Owner::Mine(tagged("ralph-p-A", "/s")));
        state.opencode = true;
        assert_eq!(
            close_plan(&state, false, false),
            Ok(vec![
                CloseStep::RemoveOpencodeSession,
                CloseStep::KillSession("ralph-p-A".into()),
                CloseStep::RemoveWorktree { force: false },
                CloseStep::DeleteBranch,
                CloseStep::PruneWorktrees,
                CloseStep::EvictOpencodeLocation,
                CloseStep::ClearMarkers,
            ])
        );
    }

    #[test]
    fn close_plan_terminates_a_cursor_worker_instead_of_killing_a_session() {
        let mut state = close_state(Owner::Mine(tagged("ralph-p-A", "/s")));
        state.cursor = true;
        let steps = close_plan(&state, false, false).unwrap();
        assert_eq!(steps[0], CloseStep::TerminateCursor);
        assert!(!steps.iter().any(|s| matches!(s, CloseStep::KillSession(_))));
    }

    #[test]
    fn close_plan_skips_what_is_already_gone() {
        let mut state = close_state(Owner::Missing);
        state.dirty_worktree = None;
        assert_eq!(
            close_plan(&state, false, false),
            Ok(vec![
                CloseStep::DeleteBranch,
                CloseStep::PruneWorktrees,
                CloseStep::ClearMarkers
            ])
        );
        state.unmerged_commits = None;
        assert!(close_plan(&state, true, false).is_err());
    }

    #[test]
    fn close_plan_clears_markers_when_nothing_else_is_left() {
        let mut state = close_state(Owner::Missing);
        state.dirty_worktree = None;
        state.unmerged_commits = None;
        state.markers = true;
        assert_eq!(
            close_plan(&state, false, false),
            Ok(vec![CloseStep::ClearMarkers])
        );
    }

    #[test]
    fn close_plan_markers_do_not_bypass_refusals() {
        let mut foreign = close_state(Owner::Foreign(tagged("ralph-p-A", "/other")));
        foreign.markers = true;
        assert!(close_plan(&foreign, true, false).is_err());

        let mut unmerged = close_state(Owner::Missing);
        unmerged.unmerged_commits = Some(1);
        unmerged.markers = true;
        assert!(close_plan(&unmerged, false, false).is_err());

        let mut dirty = close_state(Owner::Missing);
        dirty.dirty_worktree = Some(true);
        dirty.markers = true;
        assert!(close_plan(&dirty, false, false).is_err());
    }

    #[test]
    fn close_plan_refuses_foreign_workers_even_with_abandon() {
        let state = close_state(Owner::Foreign(tagged("ralph-p-A", "/other")));
        let refusal = close_plan(&state, true, false).unwrap_err();
        assert!(refusal.contains("another chat"), "{refusal}");
    }

    #[test]
    fn close_plan_with_force_kills_the_foreign_session_but_keeps_other_refusals() {
        let mut state = close_state(Owner::Foreign(tagged("ralph-p-A", "/other")));
        assert_eq!(
            close_plan(&state, false, true).unwrap(),
            vec![
                CloseStep::KillSession("ralph-p-A".into()),
                CloseStep::RemoveWorktree { force: false },
                CloseStep::DeleteBranch,
                CloseStep::PruneWorktrees,
                CloseStep::ClearMarkers,
            ]
        );
        state.unmerged_commits = Some(1);
        assert!(close_plan(&state, false, true).is_err());
        assert!(close_plan(&state, true, true).is_ok());
        state.unmerged_commits = Some(0);
        state.dirty_worktree = Some(true);
        assert!(close_plan(&state, false, true).is_err());
    }

    #[test]
    fn same_store_ignores_lexical_differences() {
        assert!(same_store("/x/chat", "/x/chat"));
        assert!(same_store("/x/chat/", "/x/chat"));
        assert!(same_store("/x//chat", "/x/chat"));
        assert!(same_store("/x/./chat", "/x/chat"));
        assert!(same_store("/x/a/../chat", "/x/chat"));
        assert!(!same_store("/x/chat", "/x/other"));
        assert!(!same_store("", "/x/chat"));
        assert!(!same_store("/x/chat", " "));
        assert!(!same_store("", ""));
    }

    #[test]
    fn close_plan_refuses_unmerged_or_dirty_work_unless_abandoned() {
        let mut unmerged = close_state(Owner::Missing);
        unmerged.unmerged_commits = Some(2);
        assert!(close_plan(&unmerged, false, false)
            .unwrap_err()
            .contains("2 commit(s) not in main"));
        assert!(close_plan(&unmerged, true, false).is_ok());

        let mut dirty = close_state(Owner::Missing);
        dirty.dirty_worktree = Some(true);
        assert!(close_plan(&dirty, false, false)
            .unwrap_err()
            .contains("uncommitted changes"));
        assert!(close_plan(&dirty, true, false)
            .unwrap()
            .contains(&CloseStep::RemoveWorktree { force: true }));
    }

    fn branch(
        configured: Option<&str>,
        recorded: Option<&str>,
        origin_head: Option<&str>,
    ) -> String {
        base_branch(Ok(configured), recorded, origin_head).branch
    }

    #[test]
    fn base_branch_prefers_config_then_spawn_base_then_origin_head_then_main() {
        let origin = Some("refs/remotes/origin/trunk\n");
        assert_eq!(branch(Some("dev"), Some("feat"), origin), "dev");
        assert_eq!(branch(None, Some("feat"), origin), "feat");
        assert_eq!(branch(Some(" "), None, origin), "trunk");
        assert_eq!(branch(None, None, None), "main");
        assert_eq!(branch(None, Some(""), Some("")), "main");
    }

    #[test]
    fn base_branch_without_a_config_file_does_not_warn() {
        assert_eq!(
            base_branch(Ok(None), Some("feat"), None),
            Base {
                branch: "feat".into(),
                warning: None
            }
        );
    }

    #[test]
    fn base_branch_uses_a_valid_worker_base_without_warning() {
        assert_eq!(
            base_branch(Ok(Some("develop")), Some("feat"), None),
            Base {
                branch: "develop".into(),
                warning: None
            }
        );
    }

    #[test]
    fn base_branch_warns_and_falls_back_when_the_config_fails_to_load() {
        let error = "Failed to parse config file at /c/config.toml: expected `=`";
        let base = base_branch(Err(error), Some("feat"), Some("refs/remotes/origin/trunk"));
        assert_eq!(base.branch, "feat");
        let warning = base.warning.unwrap();
        assert!(warning.contains("worker_base was skipped"), "{warning}");
        assert!(warning.contains(error), "{warning}");
        assert!(
            warning.ends_with("checked against feat instead."),
            "{warning}"
        );
        assert_eq!(base_branch(Err(error), None, None).branch, "main");
    }

    #[test]
    fn base_config_key_lives_under_the_worker_branch() {
        assert_eq!(base_config_key("GUZ-1"), "branch.worker/GUZ-1.ralphBase");
    }

    #[test]
    fn close_plan_names_the_base_branch_in_the_unmerged_refusal() {
        let mut state = close_state(Owner::Missing);
        state.base = "develop".into();
        state.unmerged_commits = Some(1);
        assert!(close_plan(&state, false, false)
            .unwrap_err()
            .contains("1 commit(s) not in develop"));
    }

    #[test]
    fn opencode_session_gone_on_404_only() {
        assert!(opencode_session_gone("HTTP 404 Not Found\n"));
        assert!(!opencode_session_gone("HTTP 500 Internal Server Error"));
    }
}
