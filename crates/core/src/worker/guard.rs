use std::path::{Path, PathBuf};

pub const HOOKS_DIR: &str = "ralph-schedule-hooks";
pub const DEFAULT_REMOTE: &str = "origin";
pub const REFUSED_URL_BASE: &str = "ralph-refused://";
pub const REFUSED_PREFIXES: [&str; 9] = [
    "/", "./", "../", "file://", "ssh://", "git@", "git://", "http://", "https://",
];
pub const UNSET_ALL_FLAG: &str = "--unset-all";
pub const DISALLOWED_TOOLS: [&str; 3] = ["Bash(git push:*)", "Bash(gh pr:*)", "Bash(gh api:*)"];
pub const PRE_PUSH_HOOK: &str = "pre-push";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookScript {
    pub name: String,
    pub content: String,
}

pub fn disallowed_tool_args() -> Vec<String> {
    std::iter::once("--disallowedTools")
        .chain(DISALLOWED_TOOLS)
        .map(str::to_string)
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteUrls {
    pub fetch: Vec<(String, String)>,
    pub push: Vec<(String, String)>,
}

pub fn hooks_dir_in(common_dir: &Path) -> PathBuf {
    common_dir.join(HOOKS_DIR)
}

pub fn parse_remote_urls(config_listing: &str) -> RemoteUrls {
    let mut urls = RemoteUrls::default();
    for line in config_listing.lines() {
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        let Some(rest) = key.strip_prefix("remote.") else {
            continue;
        };
        let value = value.to_string();
        if let Some(name) = rest.strip_suffix(".pushurl") {
            urls.push.push((name.to_string(), value));
        } else if let Some(name) = rest.strip_suffix(".url") {
            urls.fetch.push((name.to_string(), value));
        }
    }
    urls
}

pub fn refused_push_url(remote: &str) -> String {
    format!("{REFUSED_URL_BASE}{remote}")
}

fn guarded_remotes(remotes: &RemoteUrls) -> Vec<&str> {
    let mut names: Vec<&str> = remotes
        .fetch
        .iter()
        .chain(&remotes.push)
        .map(|(name, _)| name.as_str())
        .chain(std::iter::once(DEFAULT_REMOTE))
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

fn shared_push_urls(remotes: &RemoteUrls) -> Vec<&str> {
    let mut urls: Vec<&str> = remotes
        .push
        .iter()
        .map(|(_, url)| url.as_str())
        .filter(|url| !url.is_empty())
        .collect();
    urls.sort_unstable();
    urls.dedup();
    urls
}

fn rewritten_fetch(remotes: &RemoteUrls, push_urls: &[&str]) -> Option<String> {
    remotes.fetch.iter().find_map(|(name, url)| {
        push_urls
            .iter()
            .find(|push| url.starts_with(**push))
            .map(|push| {
                format!(
                    "remote {name} fetches from {url}, which starts with the shared push url {push}; refusing that push url would break fetching"
                )
            })
    })
}

pub fn guard_config_steps(
    hooks_dir: &Path,
    remotes: &RemoteUrls,
) -> Result<Vec<Vec<String>>, String> {
    let push_urls = shared_push_urls(remotes);
    if let Some(refusal) = rewritten_fetch(remotes, &push_urls) {
        return Err(refusal);
    }
    let step = |args: &[&str]| args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
    let mut steps = vec![
        step(&["config", "extensions.worktreeConfig", "true"]),
        vec![
            "config".to_string(),
            "--worktree".to_string(),
            "core.hooksPath".to_string(),
            hooks_dir.to_string_lossy().into_owned(),
        ],
    ];
    for remote in guarded_remotes(remotes) {
        steps.push(step(&[
            "config",
            "--worktree",
            &format!("remote.{remote}.pushurl"),
            &refused_push_url(remote),
        ]));
    }
    let instead_of = format!("url.{REFUSED_URL_BASE}.insteadOf");
    steps.push(step(&["config", "--worktree", UNSET_ALL_FLAG, &instead_of]));
    steps.extend(
        push_urls
            .iter()
            .map(|url| step(&["config", "--worktree", "--add", &instead_of, url])),
    );
    let key = format!("url.{REFUSED_URL_BASE}.pushInsteadOf");
    steps.push(step(&["config", "--worktree", UNSET_ALL_FLAG, &key]));
    steps.extend(
        REFUSED_PREFIXES
            .iter()
            .map(|prefix| step(&["config", "--worktree", "--add", &key, prefix])),
    );
    Ok(steps)
}

pub fn stale_hook_names(present: &[String], scripts: &[HookScript]) -> Vec<String> {
    present
        .iter()
        .filter(|name| !scripts.iter().any(|script| &script.name == *name))
        .cloned()
        .collect()
}

pub const TEMP_HOOK_MIN_AGE_SECS: u64 = 60;
const TEMP_HOOK_PREFIX: char = '.';
const TEMP_HOOK_SUFFIX: &str = ".tmp";

pub fn temp_hook_name(name: &str, pid: u32, sequence: u64) -> String {
    format!("{TEMP_HOOK_PREFIX}{name}.{pid}.{sequence}{TEMP_HOOK_SUFFIX}")
}

pub fn is_temp_hook(file_name: &str) -> bool {
    let Some(stem) = file_name
        .strip_prefix(TEMP_HOOK_PREFIX)
        .and_then(|rest| rest.strip_suffix(TEMP_HOOK_SUFFIX))
    else {
        return false;
    };
    let mut parts = stem.rsplitn(3, '.');
    let numeric = |part: Option<&str>| {
        part.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    };
    numeric(parts.next()) && numeric(parts.next()) && parts.next().is_some_and(|n| !n.is_empty())
}

pub fn abandoned_temp_hooks(entries: &[(String, u64)]) -> Vec<String> {
    entries
        .iter()
        .filter(|(name, age)| is_temp_hook(name) && *age >= TEMP_HOOK_MIN_AGE_SECS)
        .map(|(name, _)| name.clone())
        .collect()
}

pub fn refusing_pre_push() -> String {
    "#!/bin/sh\necho 'ralph: a scheduled worker never pushes.' >&2\nexit 1\n".to_string()
}

pub fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

pub fn forwarder_script(common_hooks: &Path, name: &str) -> String {
    let target = common_hooks.join(name);
    format!(
        "#!/bin/sh\nexec {} \"$@\"\n",
        shell_quote(&target.to_string_lossy())
    )
}

pub fn hook_scripts(common_hooks: &Path, existing: &[String]) -> Vec<HookScript> {
    let mut forwarded: Vec<&String> = existing
        .iter()
        .filter(|name| !name.ends_with(".sample") && name.as_str() != PRE_PUSH_HOOK)
        .collect();
    forwarded.sort();
    forwarded.dedup();
    std::iter::once(HookScript {
        name: PRE_PUSH_HOOK.to_string(),
        content: refusing_pre_push(),
    })
    .chain(forwarded.into_iter().map(|name| HookScript {
        name: name.clone(),
        content: forwarder_script(common_hooks, name),
    }))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(steps: &[&[&str]]) -> Vec<Vec<String>> {
        steps
            .iter()
            .map(|step| step.iter().map(|s| s.to_string()).collect())
            .collect()
    }

    fn remotes(fetch: &[(&str, &str)], push: &[(&str, &str)]) -> RemoteUrls {
        let owned = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(name, url)| (name.to_string(), url.to_string()))
                .collect()
        };
        RemoteUrls {
            fetch: owned(fetch),
            push: owned(push),
        }
    }

    fn pushinsteadof_steps() -> Vec<Vec<String>> {
        let key = "url.ralph-refused://.pushInsteadOf";
        std::iter::once(vec!["config", "--worktree", "--unset-all", key].into_iter())
            .map(|step| step.map(str::to_string).collect())
            .chain(REFUSED_PREFIXES.iter().map(|prefix| {
                ["config", "--worktree", "--add", key, prefix]
                    .map(str::to_string)
                    .to_vec()
            }))
            .collect()
    }

    #[test]
    fn config_steps_refuse_every_remote_and_every_shared_push_url() {
        let config = remotes(
            &[
                ("origin", "git@github.com:me/repo.git"),
                ("up", "https://example.com/up.git"),
            ],
            &[("origin", "git@push.example.com:me/repo.git")],
        );
        let steps = guard_config_steps(Path::new("/c/ralph-schedule-hooks"), &config).unwrap();
        let mut expected = strings(&[
            &["config", "extensions.worktreeConfig", "true"],
            &[
                "config",
                "--worktree",
                "core.hooksPath",
                "/c/ralph-schedule-hooks",
            ],
            &[
                "config",
                "--worktree",
                "remote.origin.pushurl",
                "ralph-refused://origin",
            ],
            &[
                "config",
                "--worktree",
                "remote.up.pushurl",
                "ralph-refused://up",
            ],
            &[
                "config",
                "--worktree",
                "--unset-all",
                "url.ralph-refused://.insteadOf",
            ],
            &[
                "config",
                "--worktree",
                "--add",
                "url.ralph-refused://.insteadOf",
                "git@push.example.com:me/repo.git",
            ],
        ]);
        expected.extend(pushinsteadof_steps());
        assert_eq!(steps, expected);
    }

    #[test]
    fn without_remotes_origin_is_still_refused_and_no_insteadof_is_added() {
        let steps = guard_config_steps(Path::new("/h"), &RemoteUrls::default()).unwrap();
        assert!(steps.contains(
            &strings(&[&[
                "config",
                "--worktree",
                "remote.origin.pushurl",
                "ralph-refused://origin"
            ]])[0]
        ));
        assert!(!steps.iter().any(|step| step.contains(&"--add".to_string())
            && step.contains(&"url.ralph-refused://.insteadOf".to_string())));
    }

    #[test]
    fn duplicate_push_urls_and_remote_names_are_planned_once() {
        let config = remotes(
            &[("origin", "a"), ("origin", "b")],
            &[("origin", "P"), ("up", "P"), ("odd", "")],
        );
        let steps = guard_config_steps(Path::new("/h"), &config).unwrap();
        let adds = steps
            .iter()
            .filter(|step| step.last().is_some_and(|last| last == "P"))
            .count();
        assert_eq!(adds, 1);
        let pushurls = steps
            .iter()
            .filter(|step| step.iter().any(|arg| arg.ends_with(".pushurl")))
            .count();
        assert_eq!(pushurls, 3);
    }

    #[test]
    fn a_fetch_url_under_a_shared_push_url_refuses_the_task() {
        let config = remotes(
            &[("up", "https://example.com/org/repo.git")],
            &[("origin", "https://example.com/org")],
        );
        let refusal = guard_config_steps(Path::new("/h"), &config).unwrap_err();
        assert_eq!(
            refusal,
            "remote up fetches from https://example.com/org/repo.git, which starts with the shared push url https://example.com/org; refusing that push url would break fetching"
        );
    }

    #[test]
    fn a_fetch_url_outside_every_push_url_is_allowed() {
        let config = remotes(
            &[("origin", "https://example.com/a.git")],
            &[("origin", "git@example.com:a.git")],
        );
        assert!(guard_config_steps(Path::new("/h"), &config).is_ok());
    }

    #[test]
    fn the_shared_config_step_comes_before_any_worktree_step() {
        let steps = guard_config_steps(Path::new("/h"), &RemoteUrls::default()).unwrap();
        assert!(!steps[0].contains(&"--worktree".to_string()));
        assert!(steps[1..]
            .iter()
            .all(|step| step.contains(&"--worktree".to_string())));
    }

    #[test]
    fn remote_urls_are_read_from_a_config_listing() {
        let listing = "remote.origin.url git@github.com:me/repo.git
remote.origin.pushurl git@push.example.com:me/repo.git
remote.my.fork.url https://example.com/fork.git
remote.origin.fetch +refs/heads/*:refs/remotes/origin/*
garbage
";
        assert_eq!(
            parse_remote_urls(listing),
            remotes(
                &[
                    ("origin", "git@github.com:me/repo.git"),
                    ("my.fork", "https://example.com/fork.git")
                ],
                &[("origin", "git@push.example.com:me/repo.git")]
            )
        );
        assert_eq!(parse_remote_urls(""), RemoteUrls::default());
    }

    #[test]
    fn the_hooks_dir_lives_in_the_common_git_dir() {
        assert_eq!(
            hooks_dir_in(Path::new("/repo/.git")),
            Path::new("/repo/.git/ralph-schedule-hooks")
        );
    }

    #[test]
    fn forwarders_without_a_matching_real_hook_are_stale() {
        let scripts = hook_scripts(Path::new("/c/hooks"), &["commit-msg".to_string()]);
        let present: Vec<String> = ["pre-push", "commit-msg", "pre-commit"]
            .map(str::to_string)
            .to_vec();
        assert_eq!(stale_hook_names(&present, &scripts), ["pre-commit"]);
        assert!(stale_hook_names(&[], &scripts).is_empty());
    }

    #[test]
    fn temp_hook_names_round_trip_through_the_matcher() {
        assert_eq!(temp_hook_name("pre-push", 42, 7), ".pre-push.42.7.tmp");
        assert!(is_temp_hook(&temp_hook_name("pre-push", 42, 7)));
        assert!(is_temp_hook(".a.b.1.2.tmp"));
    }

    #[test]
    fn only_temp_shaped_names_match() {
        for name in [
            "pre-push",
            ".pre-push",
            ".pre-push.tmp",
            ".pre-push.42.tmp",
            ".pre-push.x.7.tmp",
            ".pre-push.42.x.tmp",
            "..42.7.tmp",
            "pre-push.42.7.tmp",
            ".pre-push.42.7",
            ".pre-push.42.7.tmp.bak",
            ".pre-push.42..tmp",
        ] {
            assert!(!is_temp_hook(name), "{name}");
        }
    }

    #[test]
    fn only_old_temp_hooks_are_abandoned() {
        let entries: Vec<(String, u64)> = [
            (".pre-push.1.1.tmp", 59),
            (".pre-push.1.2.tmp", 60),
            (".commit-msg.1.3.tmp", 3600),
            ("pre-push", 9999),
            (".notes", 9999),
        ]
        .iter()
        .map(|(name, age)| (name.to_string(), *age))
        .collect();
        assert_eq!(
            abandoned_temp_hooks(&entries),
            [".pre-push.1.2.tmp", ".commit-msg.1.3.tmp"]
        );
        assert!(abandoned_temp_hooks(&[]).is_empty());
    }

    #[test]
    fn disallowed_tools_block_push_pr_and_api() {
        assert_eq!(
            disallowed_tool_args(),
            [
                "--disallowedTools",
                "Bash(git push:*)",
                "Bash(gh pr:*)",
                "Bash(gh api:*)"
            ]
        );
    }

    #[test]
    fn the_refusing_pre_push_script_exits_nonzero() {
        assert_eq!(
            refusing_pre_push(),
            "#!/bin/sh\necho 'ralph: a scheduled worker never pushes.' >&2\nexit 1\n"
        );
    }

    #[test]
    fn a_forwarder_execs_the_repo_hook_with_its_arguments() {
        assert_eq!(
            forwarder_script(Path::new("/repo/.git/hooks"), "pre-commit"),
            "#!/bin/sh\nexec '/repo/.git/hooks/pre-commit' \"$@\"\n"
        );
    }

    #[test]
    fn quoting_survives_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
        assert_eq!(shell_quote("plain"), "'plain'");
        assert_eq!(
            forwarder_script(Path::new("/it's/hooks"), "commit-msg"),
            "#!/bin/sh\nexec '/it'\\''s/hooks/commit-msg' \"$@\"\n"
        );
    }

    #[test]
    fn hook_scripts_refuse_push_and_forward_every_other_real_hook() {
        let existing: Vec<String> = [
            "pre-commit",
            "pre-push",
            "commit-msg",
            "pre-commit.sample",
            "pre-push.sample",
            "commit-msg",
        ]
        .map(str::to_string)
        .to_vec();
        let scripts = hook_scripts(Path::new("/c/hooks"), &existing);
        let names: Vec<&str> = scripts.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["pre-push", "commit-msg", "pre-commit"]);
        assert_eq!(scripts[0].content, refusing_pre_push());
        assert_eq!(
            scripts[1].content,
            forwarder_script(Path::new("/c/hooks"), "commit-msg")
        );
    }

    #[test]
    fn without_repo_hooks_only_the_refusing_pre_push_remains() {
        let scripts = hook_scripts(Path::new("/c/hooks"), &[]);
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].name, "pre-push");
    }
}
