use std::path::Path;

pub const HOOKS_DIR: &str = "schedule-hooks";
pub const REFUSED_PUSH_URL: &str = "ralph-refused://origin";
pub const REFUSED_URL_BASE: &str = "ralph-refused://";
pub const REFUSED_PREFIXES: [&str; 2] = ["git@github.com:", "https://github.com/"];
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

pub fn guard_config_steps(hooks_dir: &Path) -> Vec<Vec<String>> {
    let step = |args: &[&str]| args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
    let mut steps = vec![
        step(&["config", "extensions.worktreeConfig", "true"]),
        vec![
            "config".to_string(),
            "--worktree".to_string(),
            "core.hooksPath".to_string(),
            hooks_dir.to_string_lossy().into_owned(),
        ],
        step(&[
            "config",
            "--worktree",
            "remote.origin.pushurl",
            REFUSED_PUSH_URL,
        ]),
    ];
    let key = format!("url.{REFUSED_URL_BASE}.pushInsteadOf");
    steps.extend(REFUSED_PREFIXES.iter().map(|prefix| {
        step(&[
            "config",
            "--worktree",
            "--replace-all",
            &key,
            prefix,
            &anchored_pattern(prefix),
        ])
    }));
    steps
}

pub fn anchored_pattern(text: &str) -> String {
    let escaped: String = text
        .chars()
        .flat_map(|c| {
            let prefix = "\\.[]{}()*+?^$|".contains(c).then_some('\\');
            prefix.into_iter().chain(std::iter::once(c))
        })
        .collect();
    format!("^{escaped}$")
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

    #[test]
    fn config_steps_are_exactly_the_plan() {
        assert_eq!(
            guard_config_steps(Path::new("/store/schedule-hooks")),
            strings(&[
                &["config", "extensions.worktreeConfig", "true"],
                &[
                    "config",
                    "--worktree",
                    "core.hooksPath",
                    "/store/schedule-hooks"
                ],
                &[
                    "config",
                    "--worktree",
                    "remote.origin.pushurl",
                    "ralph-refused://origin"
                ],
                &[
                    "config",
                    "--worktree",
                    "--replace-all",
                    "url.ralph-refused://.pushInsteadOf",
                    "git@github.com:",
                    "^git@github\\.com:$"
                ],
                &[
                    "config",
                    "--worktree",
                    "--replace-all",
                    "url.ralph-refused://.pushInsteadOf",
                    "https://github.com/",
                    "^https://github\\.com/$"
                ],
            ])
        );
    }

    #[test]
    fn anchored_pattern_escapes_regex_characters() {
        assert_eq!(anchored_pattern("a.b"), "^a\\.b$");
        assert_eq!(
            anchored_pattern("x[1]*+?^$|(){}\\"),
            "^x\\[1\\]\\*\\+\\?\\^\\$\\|\\(\\)\\{\\}\\\\$"
        );
        assert_eq!(anchored_pattern("plain"), "^plain$");
    }

    #[test]
    fn the_shared_config_step_comes_before_any_worktree_step() {
        let steps = guard_config_steps(Path::new("/h"));
        assert!(!steps[0].contains(&"--worktree".to_string()));
        assert!(steps[1..]
            .iter()
            .all(|step| step.contains(&"--worktree".to_string())));
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
