use crate::wire::price::{ModelPrice, PriceEntry, TierPrice};
use crate::wire::usage::{THIRTY_MIN, TWELVE_HOURS};

// OpenAI Standard-tier prices (USD per million tokens) from
// https://developers.openai.com/api/docs/pricing, checked 2026-10-28.
// Codex sends no cache TTL, so its writes are assumed to be 30m writes.
// The 12h write price is derived as 2.5x input, from the OpenAI prompt caching
// guide, which bills 12h ttl writes at 2.5x input and 30m writes at 1.25x.

pub(crate) const AS_OF: &str = "2026-10-28";

fn entry(model: &str, [input, cache_read, output, write_30m]: [f64; 4]) -> PriceEntry {
    PriceEntry {
        model: model.to_string(),
        above_prompt_tokens: None,
        price: ModelPrice {
            input,
            cache_read,
            cache_writes: vec![
                TierPrice {
                    tier: THIRTY_MIN,
                    usd_per_mtok: write_30m,
                },
                TierPrice {
                    tier: TWELVE_HOURS,
                    usd_per_mtok: input * 2.5,
                },
            ],
            output,
        },
    }
}

pub(crate) fn entries() -> Vec<PriceEntry> {
    vec![
        entry("gpt-6-astra", [10.0, 1.0, 50.0, 12.5]),
        entry("gpt-6.1-sol", [2.0, 0.1, 10.0, 2.5]),
        entry("gpt-6.1-terra", [1.0, 0.05, 5.0, 1.25]),
        entry("gpt-6-luna", [0.1, 0.01, 0.5, 0.125]),
    ]
}
