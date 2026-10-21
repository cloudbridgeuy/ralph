use serde::{Deserialize, Serialize};

use super::sse::SseEvent;
use super::usage::{BillingRates, UsageOutcome};

pub mod anthropic;

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

#[derive(Debug, Clone, PartialEq)]
pub struct ResponseUsage {
    pub raw: Option<serde_json::Value>,
    pub outcome: UsageOutcome,
    pub error: Option<String>,
}

pub trait Provider: Send + Sync {
    fn id(&self) -> ProviderId;
    fn default_upstream(&self) -> &'static str;
    fn base_url_env(&self) -> &'static str;
    fn secret_headers(&self) -> &'static [&'static str];
    fn classify(&self, call: &RequestHead) -> CallKind;
    fn request_facts(&self, body: &serde_json::Value) -> RequestFacts;
    fn usage(&self, response: &ResponseView) -> ResponseUsage;
    fn rates(&self, model: &str) -> BillingRates;
}

pub fn provider(id: ProviderId) -> Box<dyn Provider> {
    match id {
        ProviderId::Anthropic => Box::new(anthropic::Anthropic),
    }
}

#[cfg(test)]
pub(crate) struct ContractCase {
    pub name: &'static str,
    pub view: ResponseView,
    pub expected: UsageOutcome,
}

#[cfg(test)]
pub(crate) fn assert_provider_contract(provider: &dyn Provider, cases: &[ContractCase]) {
    use super::redact::{redact_headers, REDACTED};
    for case in cases {
        assert_eq!(
            provider.usage(&case.view).outcome,
            case.expected,
            "case {}",
            case.name
        );
    }
    assert!(
        cases.iter().any(|c| c.expected == UsageOutcome::Missing),
        "no case reports missing usage"
    );
    let names = provider
        .secret_headers()
        .iter()
        .chain(["authorization", "x-api-key"].iter());
    for name in names {
        let headers = [(name.to_string(), "secret".to_string())];
        let out = redact_headers(&headers, provider.secret_headers());
        assert_eq!(out[0].1, REDACTED, "header {name}");
    }
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
