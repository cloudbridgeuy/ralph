use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseVersionError(pub String);

impl fmt::Display for ParseVersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid version '{}': expected x.y.z", self.0)
    }
}

impl std::error::Error for ParseVersionError {}

impl Version {
    pub fn bump_patch(self) -> Self {
        Self {
            patch: self.patch + 1,
            ..self
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

fn parse_component(part: &str) -> Option<u64> {
    let digits_only = !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    let no_leading_zero = part == "0" || !part.starts_with('0');
    if digits_only && no_leading_zero {
        part.parse().ok()
    } else {
        None
    }
}

impl FromStr for Version {
    type Err = ParseVersionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let error = || ParseVersionError(s.to_string());
        let parts: Vec<&str> = s.split('.').collect();
        let [major, minor, patch] = parts.as_slice() else {
            return Err(error());
        };
        Ok(Self {
            major: parse_component(major).ok_or_else(error)?,
            minor: parse_component(minor).ok_or_else(error)?,
            patch: parse_component(patch).ok_or_else(error)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_strict_semver() {
        let v: Version = "1.22.0".parse().unwrap();
        assert_eq!((v.major, v.minor, v.patch), (1, 22, 0));
        assert_eq!(v.to_string(), "1.22.0");
    }

    #[test]
    fn rejects_malformed_versions() {
        for bad in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "v1.2.3",
            "1.2.x",
            "01.2.3",
            "1.2.3-rc1",
            "1.2.3+b",
            "-1.2.3",
            "1..3",
            " 1.2.3",
            "99999999999999999999.0.0",
        ] {
            assert!(bad.parse::<Version>().is_err(), "{bad}");
        }
    }

    #[test]
    fn orders_numerically() {
        let a: Version = "0.9.10".parse().unwrap();
        let b: Version = "0.10.0".parse().unwrap();
        assert!(a < b);
    }

    #[test]
    fn bumps_patch() {
        let v: Version = "0.1.0".parse().unwrap();
        assert_eq!(v.bump_patch().to_string(), "0.1.1");
    }

    #[test]
    fn error_names_the_input() {
        let e = "abc".parse::<Version>().unwrap_err();
        assert_eq!(e.to_string(), "invalid version 'abc': expected x.y.z");
    }
}
