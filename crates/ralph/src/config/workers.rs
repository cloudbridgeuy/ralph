use ralph_core::workers_view::Layout;
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
    pub view: Layout,
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
            view: Layout::default(),
            pr_provider: PrProvider::default(),
            lookup_timeout_ms: default_lookup_timeout_ms(),
            cache_secs: default_cache_secs(),
        }
    }
}

impl WorkersSection {
    pub fn validate(&self) -> Result<(), String> {
        if self.cache_secs == 0 {
            return Err("[workers] cache_secs must be at least 1".to_string());
        }
        if self.lookup_timeout_ms == 0 {
            return Err("[workers] lookup_timeout_ms must be at least 1".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    #[test]
    fn defaults_apply_without_a_table_and_for_missing_keys() {
        let expected = WorkersSection {
            view: Layout::Table,
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
            "[workers]\npr_provider = \"none\"\nlookup_timeout_ms = 250\ncache_secs = 1\n",
        )
        .unwrap();
        assert_eq!(
            config.workers,
            WorkersSection {
                view: Layout::Table,
                pr_provider: PrProvider::None,
                lookup_timeout_ms: 250,
                cache_secs: 1,
            }
        );
        let github: AppConfig = toml::from_str("[workers]\npr_provider = \"github\"\n").unwrap();
        assert_eq!(github.workers.pr_provider, PrProvider::Github);
    }

    #[test]
    fn table_and_list_layouts_parse() {
        for (value, expected) in [("table", Layout::Table), ("list", Layout::List)] {
            let config: AppConfig =
                toml::from_str(&format!("[workers]\nview = \"{value}\"\n")).unwrap();
            assert_eq!(config.workers.view, expected);
        }
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

    #[test]
    fn zero_values_are_invalid_and_the_minimum_is_valid() {
        assert_eq!(WorkersSection::default().validate(), Ok(()));
        let zero_cache = WorkersSection {
            cache_secs: 0,
            ..WorkersSection::default()
        };
        assert!(zero_cache.validate().unwrap_err().contains("cache_secs"));
        let zero_timeout = WorkersSection {
            lookup_timeout_ms: 0,
            ..WorkersSection::default()
        };
        assert!(zero_timeout
            .validate()
            .unwrap_err()
            .contains("lookup_timeout_ms"));
        let minimum = WorkersSection {
            cache_secs: 1,
            lookup_timeout_ms: 1,
            ..WorkersSection::default()
        };
        assert_eq!(minimum.validate(), Ok(()));
    }

    #[test]
    fn load_rejects_invalid_values_and_names_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        for (body, key) in [
            ("[workers]\ncache_secs = 0\n", "cache_secs"),
            ("[workers]\nlookup_timeout_ms = 0\n", "lookup_timeout_ms"),
            ("[workers]\nview = \"grid\"\n", "view"),
        ] {
            std::fs::write(&path, body).unwrap();
            let err = AppConfig::load_layers(&path, None).unwrap_err().to_string();
            assert!(err.contains(key), "{err}");
        }
        std::fs::write(&path, "[workers]\ncache_secs = 1\n").unwrap();
        assert_eq!(
            AppConfig::load_layers(&path, None)
                .unwrap()
                .workers
                .cache_secs,
            1
        );
    }

    #[test]
    fn invalid_workers_in_the_store_name_the_store_file() {
        let dir = tempfile::tempdir().unwrap();
        let global = dir.path().join("global.toml");
        let store = dir.path().join("store.toml");
        std::fs::write(&global, "[workers]\ncache_secs = 5\n").unwrap();
        std::fs::write(&store, "[workers]\ncache_secs = 0\n").unwrap();
        let err = AppConfig::load_layers(&global, Some(&store))
            .unwrap_err()
            .to_string();
        assert!(err.contains(&store.display().to_string()), "{err}");
        assert!(err.starts_with("Invalid config at "), "{err}");
        assert!(err.contains("cache_secs"), "{err}");
    }

    #[test]
    fn invalid_workers_in_the_global_name_the_global_file() {
        let dir = tempfile::tempdir().unwrap();
        let global = dir.path().join("global.toml");
        let store = dir.path().join("store.toml");
        std::fs::write(&global, "[workers]\ncache_secs = 0\n").unwrap();
        std::fs::write(&store, "[theme]\nname = \"x\"\n").unwrap();
        let err = AppConfig::load_layers(&global, Some(&store))
            .unwrap_err()
            .to_string();
        assert!(err.contains(&global.display().to_string()), "{err}");
    }
}
