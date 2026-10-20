use std::time::Duration;

pub const OUTAGE_BACKOFF_SECS: [u64; 2] = [2, 8];

pub const OUTAGE_MARKERS: [&str; 16] = [
    "overloaded",
    "api error: 5",
    "api error: 429",
    "rate limit",
    "rate_limit",
    "529",
    "503",
    "502",
    "500 internal",
    "econnreset",
    "econnrefused",
    "etimedout",
    "socket hang up",
    "connection error",
    "network error",
    "request timed out",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Timeout,
    Outage,
    Refusal,
}

impl FailureKind {
    pub fn label(self) -> &'static str {
        match self {
            FailureKind::Timeout => "timeout",
            FailureKind::Outage => "outage",
            FailureKind::Refusal => "refusal",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FailureInput<'a> {
    pub timed_out: bool,
    pub exit_code: i32,
    pub text: &'a str,
}

pub fn classify(input: &FailureInput) -> FailureKind {
    if input.timed_out {
        return FailureKind::Timeout;
    }
    if input.exit_code == 0 {
        return FailureKind::Refusal;
    }
    let lower = input.text.to_lowercase();
    if OUTAGE_MARKERS.iter().any(|marker| lower.contains(marker)) {
        FailureKind::Outage
    } else {
        FailureKind::Refusal
    }
}

pub fn backoff(attempt: usize) -> Option<Duration> {
    OUTAGE_BACKOFF_SECS
        .get(attempt)
        .copied()
        .map(Duration::from_secs)
}

pub fn retry_delay(kind: FailureKind, attempt: usize) -> Option<Duration> {
    (kind == FailureKind::Outage)
        .then(|| backoff(attempt))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(timed_out: bool, exit_code: i32, text: &str) -> FailureKind {
        classify(&FailureInput {
            timed_out,
            exit_code,
            text,
        })
    }

    #[test]
    fn timeout_wins_over_text() {
        assert_eq!(kind(true, -1, ""), FailureKind::Timeout);
        assert_eq!(kind(true, 1, "overloaded"), FailureKind::Timeout);
    }

    #[test]
    fn each_outage_marker_family_is_an_outage() {
        let texts = [
            "Overloaded",
            "API Error: 500 {\"type\":\"error\"}",
            "API Error: 529",
            "API Error: 429 too many",
            "Rate limit reached",
            "rate_limit_error",
            "HTTP 503 Service Unavailable",
            "502 Bad Gateway",
            "500 Internal Server Error",
            "connect ECONNRESET",
            "connect ECONNREFUSED 127.0.0.1",
            "ETIMEDOUT",
            "socket hang up",
            "Connection error.",
            "Network Error",
            "Request timed out",
        ];
        assert_eq!(texts.len(), OUTAGE_MARKERS.len());
        for text in texts {
            assert_eq!(kind(false, 1, text), FailureKind::Outage, "{text}");
        }
    }

    #[test]
    fn refusals_and_unknown_errors_are_not_outages() {
        assert_eq!(
            kind(false, 1, "I cannot help with that request"),
            FailureKind::Refusal
        );
        assert_eq!(kind(false, 1, ""), FailureKind::Refusal);
        assert_eq!(kind(false, 2, "panic: bad things"), FailureKind::Refusal);
    }

    #[test]
    fn exit_zero_is_never_an_outage() {
        assert_eq!(kind(false, 0, "overloaded"), FailureKind::Refusal);
    }

    #[test]
    fn backoff_follows_the_schedule() {
        assert_eq!(backoff(0), Some(Duration::from_secs(2)));
        assert_eq!(backoff(1), Some(Duration::from_secs(8)));
        assert_eq!(backoff(2), None);
    }

    #[test]
    fn only_outages_are_retried() {
        assert_eq!(
            retry_delay(FailureKind::Outage, 0),
            Some(Duration::from_secs(2))
        );
        assert_eq!(
            retry_delay(FailureKind::Outage, 1),
            Some(Duration::from_secs(8))
        );
        assert_eq!(retry_delay(FailureKind::Outage, 2), None);
        assert_eq!(retry_delay(FailureKind::Timeout, 0), None);
        assert_eq!(retry_delay(FailureKind::Refusal, 0), None);
    }

    #[test]
    fn labels_name_each_kind() {
        assert_eq!(FailureKind::Timeout.label(), "timeout");
        assert_eq!(FailureKind::Outage.label(), "outage");
        assert_eq!(FailureKind::Refusal.label(), "refusal");
    }
}
