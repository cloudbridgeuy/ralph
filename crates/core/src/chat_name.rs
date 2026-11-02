use std::path::Path;

const MAX_NAME_CHARS: usize = 32;
const FALLBACK_NAME: &str = "ralph";

#[derive(Debug, Clone, Copy)]
pub struct NameSources<'a> {
    pub flag: Option<&'a str>,
    pub config: Option<&'a str>,
    pub git_root: Option<&'a Path>,
    pub cwd: &'a Path,
}

pub fn sanitise_name(raw: &str) -> Option<String> {
    let stripped: String = raw.trim().chars().filter(|c| !c.is_control()).collect();
    let capped: String = stripped.chars().take(MAX_NAME_CHARS).collect();
    let name = capped.trim();
    (!name.is_empty()).then(|| name.to_string())
}

fn basename(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(sanitise_name)
}

pub fn resolve_name(sources: &NameSources) -> String {
    sources
        .flag
        .and_then(sanitise_name)
        .or_else(|| sources.config.and_then(sanitise_name))
        .or_else(|| sources.git_root.and_then(basename))
        .or_else(|| basename(sources.cwd))
        .unwrap_or_else(|| FALLBACK_NAME.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources<'a>(cwd: &'a Path) -> NameSources<'a> {
        NameSources {
            flag: None,
            config: None,
            git_root: None,
            cwd,
        }
    }

    #[test]
    fn sanitise_trims_and_strips_control_chars() {
        assert_eq!(
            sanitise_name("  my\x1b chat\n ").as_deref(),
            Some("my chat")
        );
        assert_eq!(sanitise_name("a\x07b").as_deref(), Some("ab"));
    }

    #[test]
    fn sanitise_rejects_empty_results() {
        assert_eq!(sanitise_name(""), None);
        assert_eq!(sanitise_name("  \t\x1b\n"), None);
    }

    #[test]
    fn sanitise_caps_at_32_chars_and_trims_again() {
        let long = "x".repeat(40);
        assert_eq!(sanitise_name(&long).unwrap().chars().count(), 32);
        let spaced = format!("{} tail", "y".repeat(31));
        assert_eq!(sanitise_name(&spaced), Some("y".repeat(31)));
    }

    #[test]
    fn sanitise_counts_multibyte_chars_not_bytes() {
        let wide = "é".repeat(40);
        assert_eq!(sanitise_name(&wide), Some("é".repeat(32)));
        let emoji = "🦀".repeat(33);
        assert_eq!(sanitise_name(&emoji), Some("🦀".repeat(32)));
    }

    #[test]
    fn flag_wins_over_every_other_source() {
        let cwd = Path::new("/w/cwd");
        let resolved = resolve_name(&NameSources {
            flag: Some("flagged"),
            config: Some("configured"),
            git_root: Some(Path::new("/w/root")),
            cwd,
        });
        assert_eq!(resolved, "flagged");
    }

    #[test]
    fn config_follows_flag() {
        let cwd = Path::new("/w/cwd");
        let resolved = resolve_name(&NameSources {
            config: Some("configured"),
            git_root: Some(Path::new("/w/root")),
            ..sources(cwd)
        });
        assert_eq!(resolved, "configured");
    }

    #[test]
    fn git_root_basename_follows_config() {
        let cwd = Path::new("/w/root/sub");
        let resolved = resolve_name(&NameSources {
            git_root: Some(Path::new("/w/root")),
            ..sources(cwd)
        });
        assert_eq!(resolved, "root");
    }

    #[test]
    fn cwd_basename_follows_git_root() {
        assert_eq!(resolve_name(&sources(Path::new("/w/cwd"))), "cwd");
    }

    #[test]
    fn ralph_is_the_last_resort() {
        assert_eq!(resolve_name(&sources(Path::new("/"))), "ralph");
    }

    #[test]
    fn empty_sources_fall_through() {
        let cwd = Path::new("/w/cwd");
        let resolved = resolve_name(&NameSources {
            flag: Some("  \x1b "),
            config: Some(""),
            git_root: Some(Path::new("/")),
            cwd,
        });
        assert_eq!(resolved, "cwd");
    }

    #[test]
    fn sources_are_sanitised_before_use() {
        let cwd = Path::new("/w/cwd");
        let resolved = resolve_name(&NameSources {
            flag: Some(" a\x1bb "),
            ..sources(cwd)
        });
        assert_eq!(resolved, "ab");
    }
}
