use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CacheTier {
    Ttl { seconds: u64 },
    Unspecified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheWrite {
    pub tier: CacheTier,
    pub tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_uncached: u64,
    pub cache_read: u64,
    pub cache_writes: Vec<CacheWrite>,
    pub output: u64,
    pub reasoning: Option<u64>,
    pub other: Vec<(String, u64)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "usage", rename_all = "snake_case")]
pub enum UsageOutcome {
    Reported(Usage),
    Partial(Usage),
    Missing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TierRate {
    pub tier: CacheTier,
    pub multiplier: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BillingRates {
    pub cache_read: f64,
    pub cache_writes: Vec<TierRate>,
    pub output: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnratedTier(pub CacheTier);

impl fmt::Display for UnratedTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "no billing rate for cache tier {:?}", self.0)
    }
}

impl std::error::Error for UnratedTier {}

pub fn input_equivalent(usage: &Usage, rates: &BillingRates) -> Result<f64, UnratedTier> {
    let writes = usage.cache_writes.iter().try_fold(0.0, |sum, write| {
        rates
            .cache_writes
            .iter()
            .find(|rate| rate.tier == write.tier)
            .map(|rate| sum + write.tokens as f64 * rate.multiplier)
            .ok_or(UnratedTier(write.tier))
    })?;
    Ok(usage.input_uncached as f64
        + usage.cache_read as f64 * rates.cache_read
        + writes
        + usage.output as f64 * rates.output)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIVE_MIN: CacheTier = CacheTier::Ttl { seconds: 300 };
    const ONE_HOUR: CacheTier = CacheTier::Ttl { seconds: 3600 };

    fn anthropic() -> BillingRates {
        BillingRates {
            cache_read: 0.1,
            cache_writes: vec![
                TierRate {
                    tier: FIVE_MIN,
                    multiplier: 1.25,
                },
                TierRate {
                    tier: ONE_HOUR,
                    multiplier: 2.0,
                },
            ],
            output: 5.0,
        }
    }

    fn usage(
        input_uncached: u64,
        cache_read: u64,
        cache_writes: Vec<CacheWrite>,
        output: u64,
    ) -> Usage {
        Usage {
            input_uncached,
            cache_read,
            cache_writes,
            output,
            reasoning: None,
            other: vec![],
        }
    }

    #[test]
    fn one_hour_write_alone() {
        let u = usage(
            0,
            0,
            vec![CacheWrite {
                tier: ONE_HOUR,
                tokens: 58_999,
            }],
            0,
        );
        assert_eq!(input_equivalent(&u, &anthropic()), Ok(117_998.0));
    }

    #[test]
    fn mixed_case() {
        let u = usage(
            100,
            1000,
            vec![
                CacheWrite {
                    tier: FIVE_MIN,
                    tokens: 400,
                },
                CacheWrite {
                    tier: ONE_HOUR,
                    tokens: 100,
                },
            ],
            20,
        );
        assert_eq!(
            input_equivalent(&u, &anthropic()),
            Ok(100.0 + 100.0 + 500.0 + 200.0 + 100.0)
        );
    }

    #[test]
    fn unrated_tier_errors() {
        let u = usage(
            0,
            0,
            vec![CacheWrite {
                tier: CacheTier::Unspecified,
                tokens: 1,
            }],
            0,
        );
        assert_eq!(
            input_equivalent(&u, &anthropic()),
            Err(UnratedTier(CacheTier::Unspecified))
        );
    }

    #[test]
    fn outcome_round_trips() {
        let u = usage(
            1,
            2,
            vec![CacheWrite {
                tier: ONE_HOUR,
                tokens: 3,
            }],
            4,
        );
        for outcome in [
            UsageOutcome::Reported(u.clone()),
            UsageOutcome::Partial(u),
            UsageOutcome::Missing,
        ] {
            let json = serde_json::to_string(&outcome).unwrap();
            assert_eq!(
                serde_json::from_str::<UsageOutcome>(&json).unwrap(),
                outcome
            );
        }
    }
}
