use crate::route::{error, ApiRequest, ApiResponse};

pub(crate) const MAX_BODY: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BodyPlan {
    Skip,
    Read(usize),
    TooLarge,
}

pub(crate) fn plan_body(declared: Option<usize>) -> BodyPlan {
    match declared {
        Some(0) => BodyPlan::Skip,
        Some(length) if length > MAX_BODY => BodyPlan::TooLarge,
        Some(length) => BodyPlan::Read(length),
        None => BodyPlan::Read(MAX_BODY + 1),
    }
}

pub(crate) fn exceeds_cap(read: usize) -> bool {
    read > MAX_BODY
}

pub(crate) fn reject_oversize(request: &ApiRequest) -> Option<ApiResponse> {
    (request.body_too_large || exceeds_cap(request.body.len())).then(|| error(413, "too large"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(body: Vec<u8>, body_too_large: bool) -> ApiRequest {
        ApiRequest {
            method: "POST".to_string(),
            path: "/x".to_string(),
            query: String::new(),
            host: None,
            authorization: None,
            cookies: vec![],
            body,
            content_type: None,
            body_too_large,
        }
    }

    #[test]
    fn zero_length_is_skipped() {
        assert_eq!(plan_body(Some(0)), BodyPlan::Skip);
    }

    #[test]
    fn length_at_the_cap_is_read_in_full() {
        assert_eq!(plan_body(Some(MAX_BODY)), BodyPlan::Read(MAX_BODY));
    }

    #[test]
    fn length_over_the_cap_is_refused_before_reading() {
        assert_eq!(plan_body(Some(MAX_BODY + 1)), BodyPlan::TooLarge);
        assert_eq!(plan_body(Some(usize::MAX)), BodyPlan::TooLarge);
    }

    #[test]
    fn unknown_length_reads_one_byte_past_the_cap() {
        assert_eq!(plan_body(None), BodyPlan::Read(MAX_BODY + 1));
    }

    #[test]
    fn bytes_read_are_checked_against_the_cap() {
        assert!(!exceeds_cap(MAX_BODY));
        assert!(exceeds_cap(MAX_BODY + 1));
    }

    #[test]
    fn small_body_is_not_rejected() {
        assert_eq!(reject_oversize(&request(vec![0; 10], false)), None);
    }

    #[test]
    fn declared_oversize_is_413_json() {
        let response = reject_oversize(&request(vec![], true)).unwrap();
        assert_eq!(response.status, 413);
        assert_eq!(response.body, br#"{"error":"too large"}"#);
    }

    #[test]
    fn oversize_bytes_read_are_413() {
        let response = reject_oversize(&request(vec![0; MAX_BODY + 1], false)).unwrap();
        assert_eq!(response.status, 413);
    }
}
