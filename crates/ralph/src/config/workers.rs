use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrProvider {
    #[default]
    Github,
    None,
}

fn default_lookup_timeout_ms() -> u64 {
    5000
}

fn default_cache_secs() -> u64 {
    60
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkersSection {
    #[serde(default)]
    pub pr_provider: PrProvider,
    #[serde(default = "default_lookup_timeout_ms")]
    pub lookup_timeout_ms: u64,
    #[serde(default = "default_cache_secs")]
    pub cache_secs: u64,
}

impl Default for WorkersSection {
    fn default() -> Self {
        Self {
            pr_provider: PrProvider::default(),
            lookup_timeout_ms: default_lookup_timeout_ms(),
            cache_secs: default_cache_secs(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    #[test]
    fn defaults_apply_without_a_table_and_for_missing_keys() {
        let expected = WorkersSection {
            pr_provider: PrProvider::Github,
            lookup_timeout_ms: 5000,
            cache_secs: 60,
        };
        assert_eq!(AppConfig::new().workers, expected);
        let empty: AppConfig = toml::from_str("[workers]\n").unwrap();
        assert_eq!(empty.workers, expected);
    }

    #[test]
    fn every_key_parses() {
        let config: AppConfig = toml::from_str(
            "[workers]\npr_provider = \"none\"\nlookup_timeout_ms = 250\ncache_secs = 0\n",
        )
        .unwrap();
        assert_eq!(
            config.workers,
            WorkersSection {
                pr_provider: PrProvider::None,
                lookup_timeout_ms: 250,
                cache_secs: 0,
            }
        );
        let github: AppConfig = toml::from_str("[workers]\npr_provider = \"github\"\n").unwrap();
        assert_eq!(github.workers.pr_provider, PrProvider::Github);
    }

    #[test]
    fn an_unknown_provider_or_a_wrong_type_is_rejected() {
        assert!(toml::from_str::<AppConfig>("[workers]\npr_provider = \"gitlab\"\n").is_err());
        assert!(toml::from_str::<AppConfig>("[workers]\ncache_secs = -1\n").is_err());
        assert!(toml::from_str::<AppConfig>("[workers]\nlookup_timeout_ms = \"5\"\n").is_err());
    }

    #[test]
    fn an_unknown_key_is_ignored_like_the_other_sections() {
        let config: AppConfig = toml::from_str("[workers]\nbogus = 1\ncache_secs = 9\n").unwrap();
        assert_eq!(config.workers.cache_secs, 9);
    }
}
