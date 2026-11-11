use super::AppConfig;
use crate::warn::warn;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageSection {
    #[serde(default)]
    pub ledger: Option<PathBuf>,
}

impl UsageSection {
    pub fn validate(&self) -> Result<(), String> {
        match &self.ledger {
            Some(path) if !path.is_absolute() => Err(format!(
                "[usage] ledger must be an absolute path, got \"{}\"",
                path.display()
            )),
            _ => Ok(()),
        }
    }
}

const USAGE_FILE: &str = "usage.jsonl";

pub fn ledger_path(configured: Option<&Path>, config_dir: &Path) -> PathBuf {
    configured.map_or_else(|| config_dir.join(USAGE_FILE), Path::to_path_buf)
}

pub fn configured_ledger() -> Option<&'static Path> {
    static LEDGER: OnceLock<Option<PathBuf>> = OnceLock::new();
    LEDGER
        .get_or_init(|| match AppConfig::load() {
            Ok(config) => config.usage.ledger,
            Err(e) => {
                warn(format!("{e}; using the default usage ledger"));
                None
            }
        })
        .as_deref()
}

#[cfg(test)]
mod tests {
    use crate::config::ledger_path;
    use crate::config::{AppConfig, ConfigError};
    use std::path::{Path, PathBuf};

    #[test]
    fn ledger_path_defaults_to_config_dir_usage_jsonl() {
        assert_eq!(
            ledger_path(None, Path::new("/cfg/ralph")),
            PathBuf::from("/cfg/ralph/usage.jsonl")
        );
    }

    #[test]
    fn ledger_path_takes_the_configured_file() {
        assert_eq!(
            ledger_path(
                Some(Path::new("/data/ledger.jsonl")),
                Path::new("/cfg/ralph")
            ),
            PathBuf::from("/data/ledger.jsonl")
        );
    }

    #[test]
    fn usage_ledger_key_parses() {
        let config: AppConfig =
            toml::from_str("[usage]\nledger = \"/data/usage.jsonl\"\n").unwrap();
        assert_eq!(
            config.usage.ledger.as_deref(),
            Some(Path::new("/data/usage.jsonl"))
        );
        assert_eq!(AppConfig::new().usage.ledger, None);
    }

    #[test]
    fn relative_usage_ledger_is_rejected() {
        let relative: AppConfig = toml::from_str("[usage]\nledger = \"usage.jsonl\"\n").unwrap();
        assert!(relative.usage.validate().unwrap_err().contains("absolute"));
        let absolute: AppConfig = toml::from_str("[usage]\nledger = \"/data/u.jsonl\"\n").unwrap();
        assert_eq!(absolute.usage.validate(), Ok(()));
        let checked = AppConfig::parse_reporting_unknown("[usage]\nledger = \"u\"\n", "x");
        assert!(matches!(checked, Err(ConfigError::InvalidUsage { .. })));
    }
}
