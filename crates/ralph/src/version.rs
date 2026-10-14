use ralph_core::semver::Version;
use std::sync::OnceLock;

pub struct BuildInfo<'a> {
    pub pkg_version: &'a str,
    pub sha: Option<&'a str>,
    pub dirty: bool,
    pub at_release_tag: bool,
}

pub fn display_version(info: &BuildInfo) -> String {
    if info.at_release_tag && !info.dirty {
        return info.pkg_version.to_string();
    }
    let next = info
        .pkg_version
        .parse::<Version>()
        .map(|v| v.bump_patch().to_string())
        .unwrap_or_else(|_| info.pkg_version.to_string());
    match info.sha {
        Some(sha) if info.dirty => format!("{next}-dev+{sha}.dirty"),
        Some(sha) => format!("{next}-dev+{sha}"),
        None => format!("{next}-dev"),
    }
}

pub fn current() -> &'static str {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(compute).as_str()
}

fn compute() -> String {
    let sha = option_env!("RALPH_GIT_SHA").filter(|sha| !sha.is_empty());
    display_version(&BuildInfo {
        pkg_version: env!("CARGO_PKG_VERSION"),
        sha,
        dirty: option_env!("RALPH_GIT_DIRTY") == Some("1"),
        at_release_tag: option_env!("RALPH_AT_RELEASE_TAG") == Some("1"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(sha: Option<&'static str>, dirty: bool, at_release_tag: bool) -> BuildInfo<'static> {
        BuildInfo {
            pkg_version: "0.1.0",
            sha,
            dirty,
            at_release_tag,
        }
    }

    #[test]
    fn clean_release_tag_reports_cargo_version() {
        assert_eq!(
            display_version(&info(Some("abc1234"), false, true)),
            "0.1.0"
        );
    }

    #[test]
    fn dirty_release_tag_is_dev() {
        assert_eq!(
            display_version(&info(Some("abc1234"), true, true)),
            "0.1.1-dev+abc1234.dirty"
        );
    }

    #[test]
    fn clean_dev_build() {
        assert_eq!(
            display_version(&info(Some("abc1234"), false, false)),
            "0.1.1-dev+abc1234"
        );
    }

    #[test]
    fn dirty_dev_build() {
        assert_eq!(
            display_version(&info(Some("abc1234"), true, false)),
            "0.1.1-dev+abc1234.dirty"
        );
    }

    #[test]
    fn no_git_info() {
        assert_eq!(display_version(&info(None, false, false)), "0.1.1-dev");
        assert_eq!(display_version(&info(None, true, false)), "0.1.1-dev");
    }

    #[test]
    fn unparseable_version_is_kept_as_next() {
        let odd = BuildInfo {
            pkg_version: "1.0.0-beta",
            sha: Some("abc1234"),
            dirty: false,
            at_release_tag: false,
        };
        assert_eq!(display_version(&odd), "1.0.0-beta-dev+abc1234");
    }

    #[test]
    fn carries_over_major_and_minor() {
        let v = BuildInfo {
            pkg_version: "2.5.9",
            sha: None,
            dirty: false,
            at_release_tag: false,
        };
        assert_eq!(display_version(&v), "2.5.10-dev");
    }
}
