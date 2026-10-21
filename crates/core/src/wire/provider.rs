use serde::{Deserialize, Serialize};

use super::sse::SseEvent;
use super::usage::{BillingRates, UsageOutcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    Anthropic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallKind {
    Inference,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestHead {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestFacts {
    pub model: Option<String>,
    pub params: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResponseView {
    Json(serde_json::Value),
    Events(Vec<SseEvent>),
}

pub trait Provider: Send + Sync {
    fn id(&self) -> ProviderId;
    fn default_upstream(&self) -> &'static str;
    fn secret_headers(&self) -> &'static [&'static str];
    fn classify(&self, call: &RequestHead) -> CallKind;
    fn request_facts(&self, body: &serde_json::Value) -> RequestFacts;
    fn usage(&self, response: &ResponseView) -> UsageOutcome;
    fn rates(&self, model: &str) -> BillingRates;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_serialize_lowercase_and_snake() {
        assert_eq!(
            serde_json::to_string(&ProviderId::Anthropic).unwrap(),
            "\"anthropic\""
        );
        assert_eq!(
            serde_json::to_string(&CallKind::Inference).unwrap(),
            "\"inference\""
        );
    }
}
