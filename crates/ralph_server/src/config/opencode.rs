use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OpencodeSection {
    #[serde(default)]
    pub model: Option<String>,
}

#[cfg(test)]
mod tests {
    use crate::config::AppConfig;

    #[test]
    fn opencode_model_parses_and_defaults_to_unset() {
        let config: AppConfig =
            toml::from_str("[opencode]\nmodel = \"openai/gpt-5.6-luna\"\n").unwrap();
        assert_eq!(
            config.opencode.model.as_deref(),
            Some("openai/gpt-5.6-luna")
        );
        assert_eq!(AppConfig::new().opencode.model, None);
    }
}
