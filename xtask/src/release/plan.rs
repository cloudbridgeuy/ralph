use super::error::{ReleaseError, Result};
use ralph_core::semver::Version;

pub fn parse_release_version(raw: &str) -> Result<Version> {
    raw.parse()
        .map_err(|_| ReleaseError::InvalidVersion(raw.to_string()))
}

pub fn ensure_newer(current: Version, new: Version) -> Result<()> {
    if new > current {
        Ok(())
    } else {
        Err(ReleaseError::NotNewer { current, new })
    }
}

pub fn tag_name(version: Version) -> String {
    format!("v{version}")
}

pub fn commit_message(version: Version) -> String {
    format!("chore(release): {}", tag_name(version))
}

pub fn push_command(branch: &str, version: Version) -> String {
    format!("git push origin {branch} {}", tag_name(version))
}

fn version_value(line: &str) -> Option<&str> {
    let (key, value) = line.split_once('=')?;
    (key.trim() == "version")
        .then(|| value.trim().trim_matches('"'))
        .filter(|value| !value.is_empty())
}

fn is_workspace_package_header(line: &str) -> Option<bool> {
    let trimmed = line.trim();
    trimmed
        .starts_with('[')
        .then(|| trimmed == "[workspace.package]")
}

fn workspace_version_line(text: &str) -> Option<usize> {
    let mut in_section = false;
    for (index, line) in text.lines().enumerate() {
        if let Some(is_target) = is_workspace_package_header(line) {
            in_section = is_target;
        } else if in_section && version_value(line).is_some() {
            return Some(index);
        }
    }
    None
}

pub fn read_workspace_version(text: &str) -> Result<Version> {
    let index = workspace_version_line(text).ok_or(ReleaseError::VersionLineNotFound)?;
    let raw = text
        .lines()
        .nth(index)
        .and_then(version_value)
        .ok_or(ReleaseError::VersionLineNotFound)?;
    parse_release_version(raw)
}

pub fn set_workspace_version(text: &str, new: Version) -> Result<String> {
    let index = workspace_version_line(text).ok_or(ReleaseError::VersionLineNotFound)?;
    let mut rewritten: Vec<String> = text.lines().map(str::to_string).collect();
    rewritten[index] = format!("version = \"{new}\"");
    let mut out = rewritten.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(raw: &str) -> Version {
        raw.parse().unwrap()
    }

    const MANIFEST: &str = "[workspace]\nmembers = [\"a\"]\n\n[workspace.dependencies]\nversion = \"9.9.9\"\n\n[workspace.package]\nedition = \"2021\"\nversion = \"0.1.0\"\nlicense = \"MIT\"\n\n[profile.dev]\nversion = \"7.7.7\"\n";

    #[test]
    fn parses_valid_and_rejects_invalid_versions() {
        assert_eq!(parse_release_version("1.2.3").unwrap(), v("1.2.3"));
        assert!(matches!(
            parse_release_version("1.2"),
            Err(ReleaseError::InvalidVersion(raw)) if raw == "1.2"
        ));
        assert!(parse_release_version("v1.2.3").is_err());
        assert!(parse_release_version("1.2.3-rc1").is_err());
    }

    #[test]
    fn requires_strictly_newer_version() {
        assert!(ensure_newer(v("0.1.0"), v("0.1.1")).is_ok());
        assert!(ensure_newer(v("0.9.9"), v("0.10.0")).is_ok());
        assert!(ensure_newer(v("0.1.0"), v("0.1.0")).is_err());
        assert!(ensure_newer(v("0.2.0"), v("0.1.9")).is_err());
    }

    #[test]
    fn builds_tag_commit_and_push_text() {
        assert_eq!(tag_name(v("1.2.3")), "v1.2.3");
        assert_eq!(commit_message(v("1.2.3")), "chore(release): v1.2.3");
        assert_eq!(
            push_command("main", v("1.2.3")),
            "git push origin main v1.2.3"
        );
    }

    #[test]
    fn reads_version_from_workspace_package_only() {
        assert_eq!(read_workspace_version(MANIFEST).unwrap(), v("0.1.0"));
    }

    #[test]
    fn rewrites_only_workspace_package_version() {
        let out = set_workspace_version(MANIFEST, v("0.2.0")).unwrap();
        assert_eq!(
            out,
            MANIFEST.replace("version = \"0.1.0\"", "version = \"0.2.0\"")
        );
        assert!(out.contains("version = \"9.9.9\""));
        assert!(out.contains("version = \"7.7.7\""));
    }

    #[test]
    fn preserves_missing_trailing_newline() {
        let text = "[workspace.package]\nversion = \"0.1.0\"";
        assert_eq!(
            set_workspace_version(text, v("0.1.1")).unwrap(),
            "[workspace.package]\nversion = \"0.1.1\""
        );
    }

    #[test]
    fn errors_when_version_line_is_missing() {
        let text = "[workspace.package]\nedition = \"2021\"\n\n[other]\nversion = \"1.0.0\"\n";
        assert!(matches!(
            set_workspace_version(text, v("1.0.1")),
            Err(ReleaseError::VersionLineNotFound)
        ));
        assert!(matches!(
            read_workspace_version("[workspace]\n"),
            Err(ReleaseError::VersionLineNotFound)
        ));
    }
}
