use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use toml::Table;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MermaidStyle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub colors: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family_mono: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_direction: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_text_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Table::is_empty")]
    pub config: Table,
}
