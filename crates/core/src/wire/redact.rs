pub const REDACTED: &str = "<redacted>";

pub const GENERIC_SECRET_HEADERS: [&str; 5] = [
    "authorization",
    "proxy-authorization",
    "x-api-key",
    "cookie",
    "set-cookie",
];

pub fn redact_headers(headers: &[(String, String)], extra: &[&str]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            let secret = GENERIC_SECRET_HEADERS
                .iter()
                .chain(extra)
                .any(|s| s.eq_ignore_ascii_case(name));
            let value = if secret { REDACTED } else { value };
            (name.clone(), value.to_string())
        })
        .collect()
}

pub fn redact_metadata_user_id(body: &mut serde_json::Value) {
    if let Some(user_id) = body.pointer_mut("/metadata/user_id") {
        *user_id = serde_json::Value::String(REDACTED.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(name: &str, value: &str) -> (String, String) {
        (name.to_string(), value.to_string())
    }

    #[test]
    fn generic_names_redacted() {
        for name in GENERIC_SECRET_HEADERS {
            let out = redact_headers(&[h(name, "s")], &[]);
            assert_eq!(out, vec![h(name, REDACTED)]);
        }
    }

    #[test]
    fn mixed_case_and_extra() {
        let out = redact_headers(
            &[h("X-Api-Key", "s"), h("X-Custom", "s"), h("Accept", "json")],
            &["x-custom"],
        );
        assert_eq!(
            out,
            vec![
                h("X-Api-Key", REDACTED),
                h("X-Custom", REDACTED),
                h("Accept", "json")
            ]
        );
    }
}
