use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageSection {
    #[serde(skip)]
    pub ledger: Option<PathBuf>,
}

impl UsageSection {
    pub fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::config::{AppConfig, ConfigError};
    use std::path::Path;

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
