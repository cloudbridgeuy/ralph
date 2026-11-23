use crate::render::render_markdown;
use crate::route::{error, json_ok, ApiRequest, ApiResponse};

pub(crate) const MAX_BODY: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BodyPlan {
    Skip,
    Read(usize),
    TooLarge,
}

pub(crate) fn plan_body(declared: Option<usize>, accepts: bool) -> BodyPlan {
    match declared {
        Some(length) if length > MAX_BODY => BodyPlan::TooLarge,
        _ if !accepts => BodyPlan::Skip,
        Some(0) => BodyPlan::Skip,
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

fn is_json(content_type: Option<&str>) -> bool {
    content_type
        .and_then(|value| value.split(';').next())
        .is_some_and(|essence| essence.trim().eq_ignore_ascii_case("application/json"))
}

fn parse_markdown(body: &[u8]) -> Option<String> {
    let mut value: serde_json::Value = serde_json::from_slice(body).ok()?;
    match value.get_mut("markdown")?.take() {
        serde_json::Value::String(markdown) => Some(markdown),
        _ => None,
    }
}

pub(crate) fn render(request: &ApiRequest) -> ApiResponse {
    if !is_json(request.content_type.as_deref()) {
        return error(415, "unsupported media type");
    }
    match parse_markdown(&request.body) {
        Some(markdown) => json_ok(serde_json::json!({ "html": render_markdown(&markdown) })),
        None => error(400, "bad request"),
    }
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
        assert_eq!(plan_body(Some(0), true), BodyPlan::Skip);
    }

    #[test]
    fn a_body_that_is_not_accepted_is_never_read() {
        assert_eq!(plan_body(Some(MAX_BODY), false), BodyPlan::Skip);
        assert_eq!(plan_body(Some(10), false), BodyPlan::Skip);
        assert_eq!(plan_body(None, false), BodyPlan::Skip);
    }

    #[test]
    fn a_declared_length_over_the_cap_is_refused_even_when_not_accepted() {
        assert_eq!(plan_body(Some(MAX_BODY + 1), false), BodyPlan::TooLarge);
    }

    #[test]
    fn length_at_the_cap_is_read_in_full() {
        assert_eq!(plan_body(Some(MAX_BODY), true), BodyPlan::Read(MAX_BODY));
    }

    #[test]
    fn length_over_the_cap_is_refused_before_reading() {
        assert_eq!(plan_body(Some(MAX_BODY + 1), true), BodyPlan::TooLarge);
        assert_eq!(plan_body(Some(usize::MAX), true), BodyPlan::TooLarge);
    }

    #[test]
    fn unknown_length_reads_one_byte_past_the_cap() {
        assert_eq!(plan_body(None, true), BodyPlan::Read(MAX_BODY + 1));
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

    fn json_request(content_type: Option<&str>, body: &[u8]) -> ApiRequest {
        let mut req = request(body.to_vec(), false);
        req.content_type = content_type.map(str::to_string);
        req
    }

    #[test]
    fn json_content_type_accepts_charset_and_any_case() {
        for value in [
            "application/json",
            "Application/JSON",
            "application/json; charset=utf-8",
            "application/json;charset=UTF-8",
        ] {
            assert!(is_json(Some(value)), "{value}");
        }
    }

    #[test]
    fn other_content_types_are_not_json() {
        for value in ["text/plain", "application/jsonx", "application/json5", ""] {
            assert!(!is_json(Some(value)), "{value}");
        }
        assert!(!is_json(None));
    }

    #[test]
    fn markdown_field_is_parsed() {
        assert_eq!(
            parse_markdown(br##"{"markdown":"# a","extra":1}"##),
            Some("# a".to_string())
        );
        assert_eq!(parse_markdown(br#"{"markdown":""}"#), Some(String::new()));
    }

    #[test]
    fn bad_bodies_do_not_parse() {
        for body in [
            &b""[..],
            b"nope",
            b"[]",
            b"{}",
            br#"{"markdown":null}"#,
            br#"{"markdown":5}"#,
            br#"{"markdown":["x"]}"#,
            b"{\"markdown\":\"\xff\"}",
        ] {
            assert_eq!(parse_markdown(body), None);
        }
    }

    #[test]
    fn render_returns_html_json() {
        let response = render(&json_request(
            Some("application/json"),
            br#"{"markdown":"hi"}"#,
        ));
        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, "application/json");
        let value: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(value["html"], "<r-prose><p>hi</p></r-prose>");
        for header in [
            ("Cache-Control", "no-store"),
            ("Referrer-Policy", "no-referrer"),
            ("X-Content-Type-Options", "nosniff"),
        ] {
            assert!(response.headers.contains(&header), "{header:?}");
        }
    }

    #[test]
    fn render_checks_content_type_before_body() {
        let response = render(&json_request(Some("text/plain"), b"nope"));
        assert_eq!(response.status, 415);
        assert_eq!(response.body, br#"{"error":"unsupported media type"}"#);
    }

    #[test]
    fn render_bad_body_is_400() {
        let response = render(&json_request(Some("application/json"), b"nope"));
        assert_eq!(response.status, 400);
        assert_eq!(response.body, br#"{"error":"bad request"}"#);
    }
}
