use super::EXCLUDE_MAGIC;

fn exclude_path(exclude: &str) -> Option<&str> {
    exclude.strip_prefix(EXCLUDE_MAGIC)
}

pub const CHECK_IGNORE_ARGS: [&str; 3] = ["check-ignore", "-z", "--stdin"];

pub fn check_ignore_payload(excludes: &[String]) -> Option<String> {
    let paths: String = excludes
        .iter()
        .filter_map(|exclude| exclude_path(exclude))
        .map(|path| format!("{path}\0"))
        .collect();
    (!paths.is_empty()).then_some(paths)
}

pub fn without_ignored(excludes: &[String], ignored: &str) -> Vec<String> {
    let ignored: Vec<&str> = ignored.split('\0').collect();
    excludes
        .iter()
        .filter(|exclude| !exclude_path(exclude).is_some_and(|path| ignored.contains(&path)))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_ignore_payload_lists_the_exclude_paths_nul_terminated() {
        let excludes = vec![
            ":(exclude,literal,top).claude/memory".to_string(),
            ":(exclude,literal,top)snapshots/s.git".to_string(),
        ];
        assert_eq!(
            check_ignore_payload(&excludes).as_deref(),
            Some(".claude/memory\0snapshots/s.git\0")
        );
    }

    #[test]
    fn check_ignore_payload_is_none_without_excludes() {
        assert_eq!(check_ignore_payload(&[]), None);
    }

    #[test]
    fn without_ignored_drops_the_excludes_git_reports_as_ignored() {
        let excludes = vec![
            ":(exclude,literal,top).claude/memory".to_string(),
            ":(exclude,literal,top)snapshots/s.git".to_string(),
        ];
        assert_eq!(
            without_ignored(&excludes, ".claude/memory\0"),
            [":(exclude,literal,top)snapshots/s.git"]
        );
        assert_eq!(without_ignored(&excludes, ""), excludes);
        assert!(without_ignored(&excludes, ".claude/memory\0snapshots/s.git\0").is_empty());
    }
}
