use super::project_slug;
use super::worktree::{BRANCH_PREFIX, WORKTREE_PREFIX};
use std::path::{Component, Path, PathBuf};

const BARE_LINE: &str = "bare";
const FNV_OFFSET: u32 = 0x811c_9dc5;
const FNV_PRIME: u32 = 0x0100_0193;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub bare: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetRequest<'a> {
    pub repo: Option<&'a str>,
    pub lane: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetFacts {
    pub cwd: PathBuf,
    pub cwd_repo_top: Option<PathBuf>,
    pub repo_top: Option<PathBuf>,
    pub entries: Vec<WorktreeEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnTarget {
    Cwd {
        repo: PathBuf,
    },
    Repo {
        repo: PathBuf,
    },
    Lane {
        repo: PathBuf,
        worktree: PathBuf,
        branch: Option<String>,
    },
}

pub fn parse_worktree_entries(porcelain: &str) -> Vec<WorktreeEntry> {
    porcelain
        .split("\n\n")
        .filter_map(|block| {
            let mut lines = block.lines();
            let path = lines.next()?.strip_prefix(WORKTREE_PREFIX)?;
            let rest: Vec<&str> = lines.collect();
            Some(WorktreeEntry {
                path: PathBuf::from(path),
                branch: rest
                    .iter()
                    .find_map(|line| line.strip_prefix(BRANCH_PREFIX))
                    .map(String::from),
                bare: rest.contains(&BARE_LINE),
            })
        })
        .collect()
}

pub fn qualified_project_slug(repo: &Path) -> String {
    let hash = repo
        .to_string_lossy()
        .bytes()
        .fold(FNV_OFFSET, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(FNV_PRIME)
        });
    format!("{}-{hash:08x}", project_slug(repo))
}

pub fn resolve_target(request: &TargetRequest, facts: &TargetFacts) -> Result<SpawnTarget, String> {
    match (
        non_blank("repo", request.repo)?,
        non_blank("lane", request.lane)?,
    ) {
        (None, None) => facts
            .cwd_repo_top
            .clone()
            .map(|repo| SpawnTarget::Cwd { repo })
            .ok_or_else(|| {
                format!(
                    "The chat cwd {} is not a git repository: pass repo=<path>.",
                    facts.cwd.display()
                )
            }),
        (Some(repo), None) => Ok(SpawnTarget::Repo {
            repo: repo_top(repo, facts)?,
        }),
        (Some(repo), Some(lane)) => lane_target(repo_top(repo, facts)?, lane, facts),
        (None, Some(lane)) => match (Path::new(lane).is_absolute(), facts.repo_top.as_deref()) {
            (true, Some(top)) => lane_target(inside_cwd(top, facts)?, lane, facts),
            _ => Err(format!(
                "lane {lane} needs repo=<path> unless it is the absolute path of a worktree."
            )),
        },
    }
}

fn non_blank<'a>(name: &str, value: Option<&'a str>) -> Result<Option<&'a str>, String> {
    match value.map(str::trim) {
        Some("") => Err(format!("{name} must not be empty.")),
        other => Ok(other),
    }
}

fn inside_cwd(path: &Path, facts: &TargetFacts) -> Result<PathBuf, String> {
    let absolute = facts.cwd.join(path);
    if absolute.starts_with(&facts.cwd)
        && !absolute
            .components()
            .any(|part| part == Component::ParentDir)
    {
        Ok(absolute)
    } else {
        Err(format!(
            "{} is outside the chat cwd {}.",
            path.display(),
            facts.cwd.display()
        ))
    }
}

fn repo_top(repo: &str, facts: &TargetFacts) -> Result<PathBuf, String> {
    let requested = inside_cwd(Path::new(repo), facts)?;
    let top = facts
        .repo_top
        .as_deref()
        .ok_or_else(|| format!("{} is not a git work tree.", requested.display()))?;
    inside_cwd(top, facts)
}

fn lane_target(repo: PathBuf, lane: &str, facts: &TargetFacts) -> Result<SpawnTarget, String> {
    let by_path = Path::new(lane).is_absolute();
    let found = facts.entries.iter().find(|entry| {
        if by_path {
            entry.path == Path::new(lane)
        } else {
            entry.branch.as_deref() == Some(lane)
        }
    });
    match found {
        Some(entry) if entry.bare => Err(format!(
            "{} is a bare repository, not a lane.",
            entry.path.display()
        )),
        Some(entry) => Ok(SpawnTarget::Lane {
            repo,
            worktree: entry.path.clone(),
            branch: entry.branch.clone(),
        }),
        None => Err(format!(
            "No lane {lane} in {}. Known lanes: {}.",
            repo.display(),
            known_lanes(&facts.entries)
        )),
    }
}

fn known_lanes(entries: &[WorktreeEntry]) -> String {
    entries
        .iter()
        .filter(|entry| !entry.bare)
        .map(|entry| match &entry.branch {
            Some(branch) => format!("{} ({branch})", entry.path.display()),
            None => entry.path.display().to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORCELAIN: &str = "worktree /p/r\nHEAD abc\nbranch refs/heads/main\n\nworktree /p/r/.worktrees/feat\nHEAD def\nbranch refs/heads/feat\n\nworktree /p/bare\nbare\n\nworktree /p/r/.worktrees/det\nHEAD 123\ndetached\n";

    fn entry(path: &str, branch: Option<&str>, bare: bool) -> WorktreeEntry {
        WorktreeEntry {
            path: PathBuf::from(path),
            branch: branch.map(String::from),
            bare,
        }
    }

    fn facts(cwd: &str, repo_top: Option<&str>) -> TargetFacts {
        TargetFacts {
            cwd: PathBuf::from(cwd),
            cwd_repo_top: Some(PathBuf::from(cwd)),
            repo_top: repo_top.map(PathBuf::from),
            entries: parse_worktree_entries(PORCELAIN),
        }
    }

    fn request<'a>(repo: Option<&'a str>, lane: Option<&'a str>) -> TargetRequest<'a> {
        TargetRequest { repo, lane }
    }

    #[test]
    fn parses_multi_entry_porcelain() {
        assert_eq!(
            parse_worktree_entries(PORCELAIN),
            vec![
                entry("/p/r", Some("main"), false),
                entry("/p/r/.worktrees/feat", Some("feat"), false),
                entry("/p/bare", None, true),
                entry("/p/r/.worktrees/det", None, false),
            ]
        );
    }

    #[test]
    fn no_params_targets_cwd() {
        assert_eq!(
            resolve_target(&request(None, None), &facts("/p/r", None)),
            Ok(SpawnTarget::Cwd {
                repo: PathBuf::from("/p/r")
            })
        );
    }

    #[test]
    fn non_repo_cwd_without_repo_names_cwd_and_asks_for_repo() {
        let mut facts = facts("/p/plain", None);
        facts.cwd_repo_top = None;
        let error = resolve_target(&request(None, None), &facts).unwrap_err();
        assert!(error.contains("/p/plain"), "{error}");
        assert!(error.contains("pass repo=<path>"), "{error}");
    }

    #[test]
    fn repo_only_targets_repo() {
        assert_eq!(
            resolve_target(&request(Some("/p/r"), None), &facts("/p", Some("/p/r"))),
            Ok(SpawnTarget::Repo {
                repo: PathBuf::from("/p/r")
            })
        );
    }

    #[test]
    fn relative_repo_resolves_against_cwd() {
        assert_eq!(
            resolve_target(&request(Some("r"), None), &facts("/p", Some("/p/r"))),
            Ok(SpawnTarget::Repo {
                repo: PathBuf::from("/p/r")
            })
        );
    }

    #[test]
    fn repo_outside_cwd_is_refused() {
        let error =
            resolve_target(&request(Some("/q/r"), None), &facts("/p", Some("/q/r"))).unwrap_err();
        assert!(error.contains("/q/r"), "{error}");
    }

    #[test]
    fn repo_that_is_not_a_work_tree_is_refused() {
        let error = resolve_target(&request(Some("/p/x"), None), &facts("/p", None)).unwrap_err();
        assert!(error.contains("/p/x"), "{error}");
    }

    #[test]
    fn lane_by_branch_keeps_its_branch() {
        assert_eq!(
            resolve_target(
                &request(Some("/p/r"), Some("feat")),
                &facts("/p", Some("/p/r"))
            ),
            Ok(SpawnTarget::Lane {
                repo: PathBuf::from("/p/r"),
                worktree: PathBuf::from("/p/r/.worktrees/feat"),
                branch: Some("feat".into()),
            })
        );
    }

    #[test]
    fn lane_by_absolute_path_without_repo() {
        assert_eq!(
            resolve_target(
                &request(None, Some("/p/r/.worktrees/feat")),
                &facts("/p", Some("/p/r"))
            ),
            Ok(SpawnTarget::Lane {
                repo: PathBuf::from("/p/r"),
                worktree: PathBuf::from("/p/r/.worktrees/feat"),
                branch: Some("feat".into()),
            })
        );
    }

    #[test]
    fn lane_name_without_repo_needs_repo() {
        let error =
            resolve_target(&request(None, Some("feat")), &facts("/p", Some("/p/r"))).unwrap_err();
        assert!(error.contains("repo"), "{error}");
    }

    #[test]
    fn unknown_lane_lists_known_lanes() {
        let error = resolve_target(
            &request(Some("/p/r"), Some("nope")),
            &facts("/p", Some("/p/r")),
        )
        .unwrap_err();
        assert!(error.contains("nope"), "{error}");
        assert!(error.contains("/p/r/.worktrees/feat"), "{error}");
        assert!(error.contains("feat"), "{error}");
        assert!(!error.contains("/p/bare"), "{error}");
    }

    #[test]
    fn bare_entry_is_never_a_lane() {
        let error = resolve_target(
            &request(Some("/p/r"), Some("/p/bare")),
            &facts("/p", Some("/p/r")),
        )
        .unwrap_err();
        assert!(error.contains("/p/bare"), "{error}");
    }

    #[test]
    fn branchless_entry_is_reachable_by_path() {
        assert_eq!(
            resolve_target(
                &request(Some("/p/r"), Some("/p/r/.worktrees/det")),
                &facts("/p", Some("/p/r"))
            ),
            Ok(SpawnTarget::Lane {
                repo: PathBuf::from("/p/r"),
                worktree: PathBuf::from("/p/r/.worktrees/det"),
                branch: None,
            })
        );
    }

    #[test]
    fn empty_strings_are_refused() {
        let facts = facts("/p", Some("/p/r"));
        assert!(resolve_target(&request(Some(" "), None), &facts).is_err());
        assert!(resolve_target(&request(Some("/p/r"), Some("")), &facts).is_err());
    }

    #[test]
    fn branches_layout_resolves_lanes() {
        let mut facts = facts("/p", Some("/p/vntana-model-ops"));
        facts.entries = parse_worktree_entries(
            "worktree /p/vntana-model-ops/branches/main\nHEAD a\nbranch refs/heads/main\n\nworktree /p/vntana-model-ops/branches/feat-x\nHEAD b\nbranch refs/heads/feat-x\n",
        );
        assert_eq!(
            resolve_target(&request(Some("vntana-model-ops"), Some("feat-x")), &facts),
            Ok(SpawnTarget::Lane {
                repo: PathBuf::from("/p/vntana-model-ops"),
                worktree: PathBuf::from("/p/vntana-model-ops/branches/feat-x"),
                branch: Some("feat-x".into()),
            })
        );
    }

    #[test]
    fn qualified_slug_is_stable_and_distinct() {
        let a = Path::new("/p/a/vntana-model-ops");
        let b = Path::new("/p/b/vntana-model-ops");
        assert_eq!(qualified_project_slug(a), "vntana-model-ops-0ca42700");
        assert_ne!(qualified_project_slug(a), qualified_project_slug(b));
        assert!(qualified_project_slug(b).starts_with(&project_slug(b)));
    }
}
