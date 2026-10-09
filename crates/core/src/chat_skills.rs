use serde_json::{json, Value};
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

pub fn disables_model_invocation(skill_md: &str) -> bool {
    let mut lines = skill_md.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return false;
    }
    lines
        .take_while(|line| line.trim_end() != "---")
        .filter_map(|line| line.split_once(':'))
        .any(|(key, value)| key.trim() == "disable-model-invocation" && value.trim() == "true")
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

    #[test]
    fn frontmatter_flag_detection() {
        assert!(disables_model_invocation(
            "---\nname: a\ndisable-model-invocation: true\n---\nbody"
        ));
        assert!(!disables_model_invocation(
            "---\ndisable-model-invocation: false\n---\n"
        ));
        assert!(!disables_model_invocation("---\nname: a\n---\nbody"));
        assert!(!disables_model_invocation("just text"));
        assert!(!disables_model_invocation(
            "---\nname: a\n---\ndisable-model-invocation: true\n"
        ));
        assert!(!disables_model_invocation(
            "disable-model-invocation: true\n---\n"
        ));
    }

    #[test]
    fn plugin_args_point_at_dir() {
        assert_eq!(build_plugin_args(Path::new("/p")), ["--plugin-dir", "/p"]);
        assert_eq!(plugin_manifest()["name"], "ralph");
    }
}
