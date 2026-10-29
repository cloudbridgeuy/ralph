use crate::wire::price::{ModelPrice, PriceEntry, TierPrice};
use crate::wire::usage::CacheTier;

// OpenAI Standard-tier prices (USD per million tokens) from
// https://developers.openai.com/api/docs/pricing, checked 2026-10-28.
// Codex sends no cache TTL, so its writes are assumed to be 30m writes.
pub(super) const THIRTY_MIN: CacheTier = CacheTier::Ttl { seconds: 1_800 };

fn entry(model: &str, [input, cache_read, output, write_30m]: [f64; 4]) -> PriceEntry {
    PriceEntry {
        model: model.to_string(),
        above_prompt_tokens: None,
        price: ModelPrice {
            input,
            cache_read,
            cache_writes: vec![TierPrice {
                tier: THIRTY_MIN,
                usd_per_mtok: write_30m,
            }],
            output,
        },
    }
}

pub(super) fn entries() -> Vec<PriceEntry> {
    vec![
        entry("gpt-6-astra", [10.0, 1.0, 50.0, 12.5]),
        entry("gpt-6.1-sol", [2.0, 0.1, 10.0, 2.5]),
        entry("gpt-6.1-terra", [1.0, 0.05, 5.0, 1.25]),
        entry("gpt-6-luna", [0.1, 0.01, 0.5, 0.125]),
    ]
}
