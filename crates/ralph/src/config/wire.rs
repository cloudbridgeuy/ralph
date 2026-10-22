use ralph_core::wire::price::PriceOverride;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WireSection {
    #[serde(default)]
    pub prices: BTreeMap<String, PriceOverride>,
}

#[cfg(test)]
mod tests {
    use crate::config::AppConfig;

    #[test]
    fn prices_parse_and_default_empty() {
        assert!(AppConfig::new().wire.prices.is_empty());
        let config: AppConfig =
            toml::from_str("[wire.prices.\"claude-haiku-5-5\"]\ninput = 0.1\noutput = 0.5\n")
                .unwrap();
        let price = &config.wire.prices["claude-haiku-5-5"];
        assert_eq!((price.input, price.output), (0.1, Some(0.5)));
        assert_eq!(price.cache_read, None);
    }

    #[test]
    fn unknown_price_key_is_rejected() {
        assert!(toml::from_str::<AppConfig>("[wire.prices.x]\ninput = 1\nbogus = 2\n").is_err());
    }

    #[test]
    fn negative_price_fails_load_and_zero_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[wire.prices.m]\ninput = 0\n").unwrap();
        assert!(AppConfig::load_from(&path).unwrap().wire.prices["m"].input == 0.0);
        std::fs::write(&path, "[wire.prices.m]\ninput = -1\n").unwrap();
        let err = AppConfig::load_from(&path).unwrap_err().to_string();
        assert!(err.contains('m'), "{err}");
        assert!(err.contains("input"), "{err}");
    }
}
