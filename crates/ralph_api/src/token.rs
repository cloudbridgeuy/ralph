use std::fs::File;
use std::io::Read;

const TOKEN_BYTES: usize = 16;
const BEARER_SCHEME: &str = "Bearer";
const QUERY_KEY: &str = "token";
const COOKIE_NAME: &str = "ralph_token";

#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(..)")
    }
}

impl Token {
    pub fn generate() -> Result<Token, String> {
        let mut bytes = [0u8; TOKEN_BYTES];
        File::open("/dev/urandom")
            .and_then(|mut random| random.read_exact(&mut bytes))
            .map_err(|e| format!("could not read random bytes: {e}"))?;
        Ok(Token(hex(&bytes)))
    }

    #[cfg(test)]
    pub(crate) fn from_value(value: &str) -> Token {
        Token(value.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn matches(&self, candidate: &str) -> bool {
        let (own, other) = (self.0.as_bytes(), candidate.as_bytes());
        !own.is_empty()
            && own.len() == other.len()
            && own.iter().zip(other).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
    }
}

pub fn extract_tokens(authorization: Option<&str>, query: &str, cookies: &[&str]) -> Vec<String> {
    from_bearer(authorization)
        .or_else(|| from_query(query))
        .map(|value| vec![value.to_string()])
        .unwrap_or_else(|| from_cookies(cookies))
}

fn non_empty(value: &str) -> Option<&str> {
    Some(value).filter(|v| !v.is_empty())
}

fn from_bearer(authorization: Option<&str>) -> Option<&str> {
    let (scheme, value) = authorization?.trim().split_once(' ')?;
    scheme
        .eq_ignore_ascii_case(BEARER_SCHEME)
        .then_some(value.trim())
        .and_then(non_empty)
}

fn from_query(query: &str) -> Option<&str> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == QUERY_KEY)
        .and_then(|(_, value)| non_empty(value))
}

fn from_cookies(cookies: &[&str]) -> Vec<String> {
    cookies
        .iter()
        .flat_map(|header| header.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .filter(|(name, _)| *name == COOKIE_NAME)
        .map(|(_, value)| unquote(value.trim()))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(value)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract(authorization: Option<&str>, query: &str, cookie: Option<&str>) -> Option<String> {
        let cookies: Vec<&str> = cookie.into_iter().collect();
        extract_tokens(authorization, query, &cookies)
            .into_iter()
            .next()
    }

    fn token(value: &str) -> Token {
        Token(value.to_string())
    }

    #[test]
    fn token_is_32_lowercase_hex() {
        let generated = Token::generate().unwrap();
        assert_eq!(generated.as_str().len(), 32);
        assert!(generated
            .as_str()
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
    }

    #[test]
    fn two_tokens_differ() {
        assert_ne!(Token::generate().unwrap(), Token::generate().unwrap());
    }

    #[test]
    fn eq_accepts_same_token() {
        assert!(token("abc123").matches("abc123"));
    }

    #[test]
    fn eq_rejects_wrong_same_length() {
        assert!(!token("abc123").matches("abc124"));
    }

    #[test]
    fn eq_rejects_different_length() {
        assert!(!token("abc123").matches("abc12"));
        assert!(!token("abc123").matches("abc1234"));
    }

    #[test]
    fn eq_rejects_empty() {
        assert!(!token("abc123").matches(""));
        assert!(!token("").matches(""));
    }

    #[test]
    fn extract_bearer() {
        assert_eq!(
            extract(Some("Bearer t0k"), "", None),
            Some("t0k".to_string())
        );
    }

    #[test]
    fn extract_query() {
        assert_eq!(
            extract(None, "a=1&token=t0k&b=2", None),
            Some("t0k".to_string())
        );
    }

    #[test]
    fn extract_cookie() {
        assert_eq!(
            extract(None, "", Some("ralph_token=t0k")),
            Some("t0k".to_string())
        );
    }

    #[test]
    fn extract_prefers_header_over_query() {
        assert_eq!(
            extract(Some("Bearer h"), "token=q", Some("ralph_token=c")),
            Some("h".to_string())
        );
        assert_eq!(
            extract(None, "token=q", Some("ralph_token=c")),
            Some("q".to_string())
        );
    }

    #[test]
    fn extract_none_when_absent() {
        assert_eq!(extract(None, "", None), None);
        assert_eq!(extract(Some("Basic abc"), "x=1", None), None);
    }

    #[test]
    fn extract_ignores_other_cookies() {
        assert_eq!(
            extract(None, "", Some("a=1; ralph_token=t0k; b=2")),
            Some("t0k".to_string())
        );
        assert_eq!(extract(None, "", Some("other_ralph_token=x")), None);
    }

    #[test]
    fn extract_odd_query_values() {
        assert_eq!(extract(None, "token=", None), None);
        assert_eq!(extract(None, "token", None), None);
        assert_eq!(extract(None, "token=a=b", None), Some("a=b".to_string()));
        assert_eq!(extract(None, "xtoken=a", None), None);
        assert_eq!(extract(Some("Bearer "), "", None), None);
    }

    #[test]
    fn extract_bearer_scheme_is_case_insensitive() {
        assert_eq!(
            extract(Some("bearer t0k"), "", None),
            Some("t0k".to_string())
        );
        assert_eq!(
            extract(Some("BEARER t0k"), "", None),
            Some("t0k".to_string())
        );
    }

    #[test]
    fn extract_bearer_value_is_trimmed() {
        assert_eq!(
            extract(Some("Bearer   t0k  "), "", None),
            Some("t0k".to_string())
        );
        assert_eq!(extract(Some("Bearer    "), "", None), None);
    }

    #[test]
    fn extract_query_is_not_url_decoded() {
        let extracted = extract(None, "token=a%62c", None).unwrap();
        assert_eq!(extracted, "a%62c");
        assert!(!token("abc").matches(&extracted));
    }

    #[test]
    fn extract_tokens_collects_every_ralph_token_cookie() {
        assert_eq!(
            extract_tokens(None, "", &["a=1; ralph_token=w", "ralph_token=t0k"]),
            vec!["w".to_string(), "t0k".to_string()]
        );
        assert_eq!(
            extract_tokens(None, "", &["ralph_token=w; ralph_token=t0k"]),
            vec!["w".to_string(), "t0k".to_string()]
        );
    }

    #[test]
    fn extract_tokens_strips_cookie_quotes() {
        assert_eq!(
            extract_tokens(None, "", &["ralph_token=\"t0k\""]),
            vec!["t0k".to_string()]
        );
    }

    #[test]
    fn extract_tokens_header_and_query_stay_single_and_ahead_of_cookies() {
        assert_eq!(
            extract_tokens(Some("Bearer h"), "token=q", &["ralph_token=c"]),
            vec!["h".to_string()]
        );
        assert_eq!(
            extract_tokens(None, "token=q", &["ralph_token=c"]),
            vec!["q".to_string()]
        );
    }

    #[test]
    fn extract_tokens_skips_empty_cookie_values() {
        assert!(extract_tokens(None, "", &["ralph_token=", "ralph_token=\"\""]).is_empty());
    }
}
