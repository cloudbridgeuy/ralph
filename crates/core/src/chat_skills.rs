use serde_json::{json, Value};
use std::borrow::Cow;
use std::path::{Path, PathBuf};

pub const PLUGIN_NAME: &str = "ralph";

pub struct SkillSources<'a> {
    pub cli: &'a [PathBuf],
    pub env: &'a [PathBuf],
    pub config: &'a [PathBuf],
}

pub struct SkillLinks {
    pub links: Vec<(String, PathBuf)>,
    pub dropped: Vec<PathBuf>,
}

fn expand_home(path: &Path, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix("~"), home) {
        (Ok(rest), Some(home)) => home.join(rest),
        _ => path.to_path_buf(),
    }
}

pub fn resolve_skills(sources: &SkillSources, home: Option<&Path>) -> Vec<PathBuf> {
    let chosen = [sources.cli, sources.env, sources.config]
        .into_iter()
        .find(|source| !source.is_empty())
        .unwrap_or_default();
    let mut resolved: Vec<PathBuf> = Vec::new();
    for path in chosen {
        let path = expand_home(path, home);
        if !resolved.contains(&path) {
            resolved.push(path);
        }
    }
    resolved
}

pub fn plan_links(dirs: &[PathBuf]) -> SkillLinks {
    let mut links: Vec<(String, PathBuf)> = Vec::new();
    let mut dropped = Vec::new();
    for dir in dirs {
        let name = dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        match name {
            Some(name) if !links.iter().any(|(taken, _)| *taken == name) => {
                links.push((name, dir.clone()));
            }
            _ => dropped.push(dir.clone()),
        }
    }
    SkillLinks { links, dropped }
}

pub struct ChatSkill {
    pub name: String,
    pub dir: PathBuf,
    pub skill_md: String,
}

fn strip_frontmatter(skill_md: &str) -> &str {
    let Some(rest) = skill_md
        .strip_prefix("---\n")
        .or_else(|| skill_md.strip_prefix("---\r\n"))
    else {
        return skill_md;
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        offset += line.len();
        if line.trim_end() == "---" {
            return &rest[offset..];
        }
    }
    skill_md
}

pub fn expand_slash_command<'a>(message: &'a str, skills: &[ChatSkill]) -> Cow<'a, str> {
    let Some(command) = message.strip_prefix('/') else {
        return Cow::Borrowed(message);
    };
    let (name, args) = command
        .split_once(char::is_whitespace)
        .unwrap_or((command, ""));
    let Some(skill) = skills.iter().find(|skill| skill.name == name) else {
        return Cow::Borrowed(message);
    };
    let args = args.trim();
    let body = strip_frontmatter(&skill.skill_md).trim();
    let mut expanded = format!("Base directory for this skill: {}\n\n", skill.dir.display());
    if body.contains("$ARGUMENTS") {
        expanded.push_str(&body.replace("$ARGUMENTS", args));
    } else {
        expanded.push_str(body);
        if !args.is_empty() {
            expanded.push_str(&format!("\n\nARGUMENTS: {args}"));
        }
    }
    expanded.push_str(&format!("\n\n{message}"));
    Cow::Owned(expanded)
}

pub fn plugin_manifest() -> Value {
    json!({ "name": PLUGIN_NAME })
}

pub fn build_plugin_args(dir: &Path) -> Vec<String> {
    vec![
        "--plugin-dir".to_string(),
        dir.to_string_lossy().into_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    fn resolve(cli: &[&str], env: &[&str], config: &[&str], home: Option<&str>) -> Vec<PathBuf> {
        resolve_skills(
            &SkillSources {
                cli: &paths(cli),
                env: &paths(env),
                config: &paths(config),
            },
            home.map(Path::new),
        )
    }

    #[test]
    fn cli_beats_env_beats_config() {
        assert_eq!(resolve(&["/c"], &["/e"], &["/f"], None), paths(&["/c"]));
        assert_eq!(resolve(&[], &["/e"], &["/f"], None), paths(&["/e"]));
        assert_eq!(resolve(&[], &[], &["/f"], None), paths(&["/f"]));
        assert!(resolve(&[], &[], &[], None).is_empty());
    }

    #[test]
    fn tilde_expands_only_with_home() {
        assert_eq!(
            resolve(&["~/a", "~", "~x/b"], &[], &[], Some("/h")),
            paths(&["/h/a", "/h", "~x/b"])
        );
        assert_eq!(resolve(&["~/a"], &[], &[], None), paths(&["~/a"]));
    }

    #[test]
    fn dedupe_keeps_first_after_expansion() {
        assert_eq!(
            resolve(&["/h/a", "/b", "~/a"], &[], &[], Some("/h")),
            paths(&["/h/a", "/b"])
        );
    }

    #[test]
    fn duplicate_basenames_are_dropped() {
        let plan = plan_links(&paths(&["/x/show", "/y/show", "/y/other", "/"]));
        assert_eq!(
            plan.links,
            vec![
                ("show".to_string(), PathBuf::from("/x/show")),
                ("other".to_string(), PathBuf::from("/y/other")),
            ]
        );
        assert_eq!(plan.dropped, paths(&["/y/show", "/"]));
    }

    fn skill(skill_md: &str) -> Vec<ChatSkill> {
        vec![ChatSkill {
            name: "show-me".to_string(),
            dir: PathBuf::from("/s/show-me"),
            skill_md: skill_md.to_string(),
        }]
    }

    #[test]
    fn args_append_without_placeholder() {
        let out = expand_slash_command("/show-me  x y ", &skill("---\nname: a\n---\nBody\n"));
        assert_eq!(
            out,
            "Base directory for this skill: /s/show-me\n\nBody\n\nARGUMENTS: x y\n\n/show-me  x y "
        );
    }

    #[test]
    fn args_substitute_placeholder() {
        let out = expand_slash_command("/show-me x", &skill("Do $ARGUMENTS then $ARGUMENTS"));
        assert_eq!(
            out,
            "Base directory for this skill: /s/show-me\n\nDo x then x\n\n/show-me x"
        );
    }

    #[test]
    fn no_args_no_frontmatter() {
        let out = expand_slash_command("/show-me", &skill("Body"));
        assert_eq!(
            out,
            "Base directory for this skill: /s/show-me\n\nBody\n\n/show-me"
        );
    }

    #[test]
    fn frontmatter_is_stripped() {
        let out = expand_slash_command("/show-me", &skill("---\nname: a\n---\nBody"));
        assert!(!out.contains("name: a"));
        assert!(out.contains("Body"));
    }

    #[test]
    fn non_matching_messages_pass_through() {
        let skills = skill("Body");
        for message in ["/other x", "/show-meX", "/", "plain text", " /show-me"] {
            assert_eq!(expand_slash_command(message, &skills), message);
        }
    }

    #[test]
    fn plugin_args_point_at_dir() {
        assert_eq!(build_plugin_args(Path::new("/p")), ["--plugin-dir", "/p"]);
        assert_eq!(plugin_manifest()["name"], "ralph");
    }
}
