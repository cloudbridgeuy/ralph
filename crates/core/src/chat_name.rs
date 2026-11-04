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

const FORMAT_CHARS: &[(u32, u32)] = &[
    (0x00AD, 0x00AD),
    (0x0600, 0x0605),
    (0x061C, 0x061C),
    (0x06DD, 0x06DD),
    (0x070F, 0x070F),
    (0x0890, 0x0891),
    (0x08E2, 0x08E2),
    (0x180E, 0x180E),
    (0x200B, 0x200F),
    (0x202A, 0x202E),
    (0x2060, 0x2064),
    (0x2066, 0x206F),
    (0xFEFF, 0xFEFF),
    (0xFFF9, 0xFFFB),
    (0x110BD, 0x110BD),
    (0x110CD, 0x110CD),
    (0x13430, 0x1343F),
    (0x1BCA0, 0x1BCA3),
    (0x1D173, 0x1D17A),
    (0xE0001, 0xE0001),
    (0xE0020, 0xE007F),
];

pub(crate) fn is_format_char(c: char) -> bool {
    let code = c as u32;
    FORMAT_CHARS
        .iter()
        .any(|&(first, last)| (first..=last).contains(&code))
}

pub fn sanitise_name(raw: &str) -> Option<String> {
    let stripped: String = raw
        .trim()
        .chars()
        .filter(|c| !c.is_control() && !is_format_char(*c))
        .collect();
    let capped: String = stripped.chars().take(MAX_NAME_CHARS).collect();
    let name = capped.trim();
    (!name.is_empty()).then(|| name.to_string())
}

pub fn git_root_candidates<'a>(cwd: &'a Path, home: Option<&Path>) -> Vec<&'a Path> {
    match home.filter(|home| cwd.starts_with(home)) {
        Some(home) => cwd.ancestors().take_while(|dir| *dir != home).collect(),
        None => cwd.ancestors().collect(),
    }
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

    fn paths<'a>(list: &'a [&'a str]) -> Vec<&'a Path> {
        list.iter().map(Path::new).collect()
    }

    #[test]
    fn candidates_under_home_exclude_home_and_above() {
        let got = git_root_candidates(Path::new("/h/u/work/x"), Some(Path::new("/h/u")));
        assert_eq!(got, paths(&["/h/u/work/x", "/h/u/work"]));
    }

    #[test]
    fn candidates_under_home_keep_a_repo_inside_home() {
        let got = git_root_candidates(Path::new("/h/u/repo/src"), Some(Path::new("/h/u")));
        assert!(got.contains(&Path::new("/h/u/repo")));
        assert!(!got.contains(&Path::new("/h/u")));
    }

    #[test]
    fn candidates_at_home_are_empty() {
        let got = git_root_candidates(Path::new("/h/u"), Some(Path::new("/h/u")));
        assert!(got.is_empty());
    }

    #[test]
    fn candidates_outside_home_are_all_ancestors() {
        let got = git_root_candidates(Path::new("/tmp/x"), Some(Path::new("/h/u")));
        assert_eq!(got, paths(&["/tmp/x", "/tmp", "/"]));
    }

    #[test]
    fn candidates_without_home_are_all_ancestors() {
        let got = git_root_candidates(Path::new("/tmp/x"), None);
        assert_eq!(got, paths(&["/tmp/x", "/tmp", "/"]));
    }

    #[test]
    fn sanitise_removes_unicode_format_chars() {
        assert_eq!(sanitise_name("a\u{202E}b").as_deref(), Some("ab"));
        assert_eq!(sanitise_name("a\u{200B}b").as_deref(), Some("ab"));
        assert_eq!(sanitise_name("a\u{200D}b").as_deref(), Some("ab"));
    }

    #[test]
    fn sanitise_rejects_names_made_only_of_format_chars() {
        assert_eq!(sanitise_name("\u{202E}\u{200B}\u{200D}"), None);
    }

    #[test]
    fn sanitise_cap_counts_chars_kept_after_format_removal() {
        let padded: String = "x\u{200B}".repeat(40);
        assert_eq!(sanitise_name(&padded), Some("x".repeat(32)));
    }
}
