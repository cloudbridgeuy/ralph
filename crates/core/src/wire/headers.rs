const HOP_BY_HOP: [&str; 6] = [
    "connection",
    "keep-alive",
    "transfer-encoding",
    "proxy-connection",
    "te",
    "upgrade",
];

const STORED_RESPONSE_HEADERS: [&str; 4] = [
    "request-id",
    "content-type",
    "content-encoding",
    "retry-after",
];

const STORED_RESPONSE_PREFIX: &str = "anthropic-ratelimit-";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Request,
    Response,
}

impl Direction {
    fn dropped(self) -> &'static str {
        match self {
            Self::Request => "host",
            Self::Response => "content-length",
        }
    }
}

pub fn forward_headers<V: Clone>(
    headers: &[(String, V)],
    direction: Direction,
) -> Vec<(String, V)> {
    headers
        .iter()
        .filter(|(name, _)| {
            !HOP_BY_HOP
                .iter()
                .chain([&direction.dropped()])
                .any(|dropped| dropped.eq_ignore_ascii_case(name))
        })
        .cloned()
        .collect()
}

pub fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

pub fn stored_response_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .filter(|(name, _)| {
            let lower = name.to_ascii_lowercase();
            STORED_RESPONSE_HEADERS.contains(&lower.as_str())
                || lower.starts_with(STORED_RESPONSE_PREFIX)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(name: &str, value: &str) -> (String, String) {
        (name.to_string(), value.to_string())
    }

    #[test]
    fn request_drops_hop_by_hop_and_host_only() {
        let input = vec![
            h("Host", "127.0.0.1:1"),
            h("Connection", "keep-alive"),
            h("Transfer-Encoding", "chunked"),
            h("Content-Length", "5"),
            h("accept-encoding", "gzip"),
            h("x-api-key", "k"),
        ];
        assert_eq!(
            forward_headers(&input, Direction::Request),
            vec![
                h("Content-Length", "5"),
                h("accept-encoding", "gzip"),
                h("x-api-key", "k")
            ]
        );
    }

    #[test]
    fn response_drops_content_length_and_keeps_host() {
        let input = vec![
            h("Content-Length", "5"),
            h("Keep-Alive", "timeout=5"),
            h("Upgrade", "x"),
            h("TE", "trailers"),
            h("Proxy-Connection", "x"),
            h("host", "h"),
            h("content-encoding", "gzip"),
        ];
        assert_eq!(
            forward_headers(&input, Direction::Response),
            vec![h("host", "h"), h("content-encoding", "gzip")]
        );
    }

    #[test]
    fn stored_headers_exclude_account_identifiers() {
        let input = vec![
            h("Request-Id", "r"),
            h("Content-Type", "text/event-stream"),
            h("content-encoding", "gzip"),
            h("Retry-After", "3"),
            h("anthropic-ratelimit-requests-limit", "50"),
            h("Anthropic-RateLimit-Tokens-Reset", "t"),
            h("anthropic-organization-id", "org"),
            h("anthropic-workspace-id", "ws"),
            h("set-cookie", "c"),
        ];
        let names: Vec<_> = stored_response_headers(&input)
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(
            names,
            [
                "Request-Id",
                "Content-Type",
                "content-encoding",
                "Retry-After",
                "anthropic-ratelimit-requests-limit",
                "Anthropic-RateLimit-Tokens-Reset"
            ]
        );
    }

    #[test]
    fn header_value_is_case_insensitive() {
        assert_eq!(
            header_value(&[h("Content-Type", "a")], "content-type"),
            Some("a")
        );
        assert_eq!(header_value(&[], "content-type"), None);
    }
}
