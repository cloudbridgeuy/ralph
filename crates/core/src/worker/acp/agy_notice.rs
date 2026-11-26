#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgyNotice {
    Quota { reset_secs: Option<u64> },
    ApiError { attempt: Option<u32> },
    Safety,
    Other,
}

pub fn agy_notice(text: &str) -> Option<AgyNotice> {
    let body = text.trim_start().strip_prefix('⚠')?.trim_start();
    Some(if body.starts_with("Individual quota reached") {
        AgyNotice::Quota {
            reset_secs: reset_secs(body),
        }
    } else if body.starts_with("API error") {
        AgyNotice::ApiError {
            attempt: attempt(body),
        }
    } else if body.contains("content safety filters") {
        AgyNotice::Safety
    } else {
        AgyNotice::Other
    })
}

fn reset_secs(body: &str) -> Option<u64> {
    let (_, rest) = body.split_once("Resets in ")?;
    let duration = rest.split_whitespace().next()?.trim_end_matches('.');
    let mut total = 0u64;
    let mut digits = String::new();
    for ch in duration.chars() {
        let unit = match ch {
            'h' => 3600,
            'm' => 60,
            's' => 1,
            _ if ch.is_ascii_digit() => {
                digits.push(ch);
                continue;
            }
            _ => return None,
        };
        total = total.checked_add(digits.parse::<u64>().ok()?.checked_mul(unit)?)?;
        digits.clear();
    }
    (digits.is_empty() && total > 0).then_some(total)
}

fn attempt(body: &str) -> Option<u32> {
    let (_, rest) = body.split_once("(attempt ")?;
    rest.split_once(')')?.0.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_notice_parses_reset_3h0m45s() {
        let text = "\n⚠ Individual quota reached. Please upgrade your subscription to increase your limits. Resets in 3h0m45s.\n";
        assert_eq!(
            agy_notice(text),
            Some(AgyNotice::Quota {
                reset_secs: Some(10845)
            })
        );
    }

    #[test]
    fn quota_notice_without_reset_has_none() {
        assert_eq!(
            agy_notice("⚠ Individual quota reached."),
            Some(AgyNotice::Quota { reset_secs: None })
        );
    }

    #[test]
    fn api_error_notice_parses_attempt() {
        let text =
            "\n⚠ API error (attempt 1): request failed: Post \"https://x\": operation timed out\n";
        assert_eq!(
            agy_notice(text),
            Some(AgyNotice::ApiError { attempt: Some(1) })
        );
    }

    #[test]
    fn safety_notice_is_safety() {
        let text = "\n⚠ Your previous response was blocked by content safety filters: x";
        assert_eq!(agy_notice(text), Some(AgyNotice::Safety));
    }

    #[test]
    fn unknown_warning_is_other() {
        assert_eq!(agy_notice("⚠ something new"), Some(AgyNotice::Other));
    }

    #[test]
    fn plain_text_is_not_a_notice() {
        assert_eq!(agy_notice("summary ⚠ not first"), None);
        assert_eq!(agy_notice(""), None);
    }
}
