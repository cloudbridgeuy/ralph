use super::*;
use crate::wire::provider::SegmentSource;

const IMPLICIT_30M: CacheTier = CacheTier::Ttl { seconds: 1_800 };

struct ImplicitProvider {
    tier: CacheTier,
    min_tokens: Option<u64>,
}

impl Provider for ImplicitProvider {
    fn id(&self) -> ProviderId {
        Anthropic.id()
    }
    fn default_upstream(&self) -> &'static str {
        Anthropic.default_upstream()
    }
    fn base_url_hint(&self, listen: &str) -> String {
        Anthropic.base_url_hint(listen)
    }
    fn stored_response_headers(&self) -> crate::wire::headers::StoredHeaders {
        Anthropic.stored_response_headers()
    }
    fn secret_headers(&self) -> &'static [&'static str] {
        Anthropic.secret_headers()
    }
    fn cache_relevant_headers(&self) -> &'static [&'static str] {
        Anthropic.cache_relevant_headers()
    }
    fn cache_relevant_params(&self) -> &'static [&'static str] {
        Anthropic.cache_relevant_params()
    }
    fn classify(&self, call: &RequestHead) -> CallKind {
        Anthropic.classify(call)
    }
    fn request_facts(&self, body: &Value) -> RequestFacts {
        Anthropic.request_facts(body)
    }
    fn segments(&self, body: &Value) -> Vec<PromptSegment> {
        Anthropic.segments(body)
    }
    fn prefix_cache(&self, _body: &Value) -> PrefixCache {
        PrefixCache::Implicit {
            tier: self.tier,
            min_tokens: self.min_tokens,
        }
    }
    fn usage(&self, response: &ResponseView) -> ResponseUsage {
        Anthropic.usage(response)
    }
    fn rates(&self, model: &str) -> BillingRates {
        Anthropic.rates(model)
    }
    fn prices(&self) -> PriceTable {
        Anthropic.prices()
    }
    fn price_factor(&self, raw_usage: &Value, facts: &RequestFacts, model: &str) -> Option<f64> {
        Anthropic.price_factor(raw_usage, facts, model)
    }
}

fn implicit_view_with(
    a: &CallRecord,
    b: &CallRecord,
    tier: CacheTier,
    min_tokens: Option<u64>,
) -> DiffView {
    diff_view(&DiffInput {
        a: Some(a),
        b,
        session: &[a.clone(), b.clone()],
        provider: &ImplicitProvider { tier, min_tokens },
        base_picked: false,
    })
}

fn implicit_view(a: &CallRecord, b: &CallRecord) -> DiffView {
    implicit_view_with(a, b, IMPLICIT_30M, None)
}

fn unmarked_pair(b_usage: UsageOutcome, b_start: i64) -> (CallRecord, CallRecord) {
    (
        call(
            1,
            0,
            10,
            vec![blk("shared head", None), blk("a", None)],
            usage(5, 0),
        ),
        call(
            2,
            b_start,
            b_start + 10,
            vec![blk("shared head", None), blk("b", None)],
            b_usage,
        ),
    )
}

#[test]
fn implicit_without_breakpoints_serves_the_whole_shared_prefix() {
    let (a, b) = unmarked_pair(usage(0, 100), 20);
    let explanation = implicit_view(&a, &b).explanation;
    assert_eq!(
        explanation.verdict,
        Verdict::PrefixChanged {
            label: "msg[0].1 user/text".into()
        }
    );
    assert!(explanation.servable_bytes > 0);
    assert!(explanation.servable_tokens.unwrap() > 0);
}

#[test]
fn implicit_gap_beyond_its_tier_expires() {
    let (a, b) = unmarked_pair(written(0, 100), 2_000);
    assert_eq!(
        implicit_view(&a, &b).explanation.verdict,
        Verdict::TtlExpired {
            gap_secs: 1_990,
            ttl_secs: 1_800,
            since_seq: 1
        }
    );
}

#[test]
fn implicit_gap_within_its_tier_is_unexplained() {
    let (a, b) = unmarked_pair(written(0, 100), 400);
    assert!(matches!(
        implicit_view(&a, &b).explanation.verdict,
        Verdict::Unexplained { .. }
    ));
}

#[test]
fn implicit_unspecified_tier_never_expires_by_ttl() {
    let (a, b) = unmarked_pair(written(0, 100), 7_200);
    assert!(matches!(
        implicit_view_with(&a, &b, CacheTier::Unspecified, None)
            .explanation
            .verdict,
        Verdict::Unexplained { .. }
    ));
}

#[test]
fn implicit_short_shared_prefix_is_too_short_to_cache() {
    let (a, b) = unmarked_pair(written(0, 100), 400);
    let view = implicit_view_with(&a, &b, IMPLICIT_30M, Some(1_024));
    let servable = view.explanation.servable_tokens.unwrap();
    assert!(servable < 1_024);
    assert_eq!(
        view.explanation.verdict,
        Verdict::PrefixTooShort {
            servable_tokens: servable,
            min_tokens: 1_024
        }
    );
    assert!(verdict_text(&view).starts_with("prefix too short to cache"));
    let json = serde_json::to_value(&view).unwrap();
    assert_eq!(json["explanation"]["verdict"]["kind"], "prefix_too_short");
}

#[test]
fn implicit_prefix_at_or_above_the_minimum_keeps_the_other_verdicts() {
    let (a, b) = unmarked_pair(written(0, 100), 2_000);
    let view = implicit_view_with(&a, &b, IMPLICIT_30M, Some(10));
    assert!(matches!(
        view.explanation.verdict,
        Verdict::TtlExpired { .. }
    ));
}

#[test]
fn implicit_prefers_the_marked_breakpoint_tier() {
    let (a, mut b) = pair("a", "b", written(0, 100));
    b.started_at = t(400);
    assert_eq!(
        implicit_view(&a, &b).explanation.verdict,
        Verdict::TtlExpired {
            gap_secs: 390,
            ttl_secs: 300,
            since_seq: 1
        }
    );
}

#[test]
fn implicit_serves_past_the_last_marked_breakpoint() {
    let a = call(
        1,
        0,
        10,
        vec![blk("head", Some("5m")), blk("more", None), blk("a", None)],
        usage(5, 0),
    );
    let b = call(
        2,
        20,
        30,
        vec![blk("head", Some("5m")), blk("more", None), blk("b", None)],
        usage(0, 100),
    );
    let implicit = implicit_view(&a, &b).explanation;
    let explicit = view_of(&a, &b, &[]).explanation;
    assert!(implicit.servable_bytes > explicit.servable_bytes);
}

#[test]
fn explicit_providers_keep_marked_breakpoint_behaviour() {
    let (a, b) = unmarked_pair(usage(0, 7), 20);
    assert_eq!(
        view_of(&a, &b, &[]).explanation.verdict,
        Verdict::NoSharedBreakpoint
    );
}

fn seg(breakpoint: Option<CacheTier>) -> PromptSegment {
    PromptSegment {
        label: "s".into(),
        source: SegmentSource::Messages,
        text: "t".into(),
        content: "t".into(),
        breakpoint,
    }
}

#[test]
fn servable_breakpoint_explicit_takes_the_last_marked_one() {
    let segments = [seg(Some(FIVE_MIN)), seg(None), seg(None)];
    assert_eq!(
        servable_breakpoint(&segments, 3, PrefixCache::Explicit),
        Some(Breakpoint {
            index: 0,
            tier: FIVE_MIN
        })
    );
    assert_eq!(
        servable_breakpoint(&[seg(None)], 1, PrefixCache::Explicit),
        None
    );
}

#[test]
fn servable_breakpoint_implicit_spans_the_shared_prefix() {
    let implicit = PrefixCache::Implicit {
        tier: IMPLICIT_30M,
        min_tokens: None,
    };
    let segments = [seg(Some(FIVE_MIN)), seg(None), seg(None)];
    assert_eq!(
        servable_breakpoint(&segments, 3, implicit),
        Some(Breakpoint {
            index: 2,
            tier: FIVE_MIN
        })
    );
    assert_eq!(
        servable_breakpoint(&[seg(None), seg(None)], 2, implicit),
        Some(Breakpoint {
            index: 1,
            tier: IMPLICIT_30M
        })
    );
    assert_eq!(servable_breakpoint(&[seg(None)], 0, implicit), None);
}
