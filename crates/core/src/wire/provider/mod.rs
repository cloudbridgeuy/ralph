use serde::{Deserialize, Serialize};

use super::price::PriceTable;
use super::sse::SseEvent;
use super::usage::{BillingRates, CacheTier, UsageOutcome};

pub mod anthropic;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    Anthropic,
}

impl ProviderId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SegmentSource {
    Tools,
    System,
    Messages,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSegment {
    pub label: String,
    pub source: SegmentSource,
    pub text: String,
    pub content: String,
    pub breakpoint: Option<CacheTier>,
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
    fn cache_relevant_headers(&self) -> &'static [&'static str];
    fn cache_relevant_params(&self) -> &'static [&'static str];
    fn classify(&self, call: &RequestHead) -> CallKind;
    fn request_facts(&self, body: &serde_json::Value) -> RequestFacts;
    fn segments(&self, body: &serde_json::Value) -> Vec<PromptSegment>;
    fn usage(&self, response: &ResponseView) -> ResponseUsage;
    fn rates(&self, model: &str) -> BillingRates;
    fn prices(&self) -> PriceTable;
    fn price_factor(&self, raw_usage: &serde_json::Value, model: &str) -> Option<f64>;
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
pub(crate) fn assert_provider_contract(
    provider: &dyn Provider,
    cases: &[ContractCase],
    request: &serde_json::Value,
) {
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
    let segments = provider.segments(request);
    assert!(!segments.is_empty(), "no prompt segments");
    assert!(
        segments
            .windows(2)
            .all(|pair| pair[0].source <= pair[1].source),
        "segments not in tools, system, messages order"
    );
    assert!(
        segments.iter().any(|segment| segment.breakpoint.is_some()),
        "no cache breakpoint detected"
    );
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
