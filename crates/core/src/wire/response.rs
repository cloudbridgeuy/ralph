use std::fmt;

use super::decode::{decode_body, DecodeError};
use super::provider::ResponseView;
use super::sse::SseDecoder;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewError {
    Decode(DecodeError),
    Json(String),
}

impl fmt::Display for ViewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(error) => write!(f, "{error}"),
            Self::Json(message) => write!(f, "invalid json body: {message}"),
        }
    }
}

impl std::error::Error for ViewError {}

impl From<DecodeError> for ViewError {
    fn from(error: DecodeError) -> Self {
        Self::Decode(error)
    }
}

fn is_event_stream(content_type: Option<&str>) -> bool {
    content_type
        .and_then(|value| value.split(';').next())
        .is_some_and(|media| media.trim().eq_ignore_ascii_case("text/event-stream"))
}

pub fn view(
    content_type: Option<&str>,
    content_encoding: Option<&str>,
    body: &[u8],
) -> Result<ResponseView, ViewError> {
    let decoded = decode_body(content_encoding, body)?;
    if is_event_stream(content_type) {
        return Ok(ResponseView::Events(SseDecoder::default().push(&decoded)));
    }
    serde_json::from_slice(&decoded)
        .map(ResponseView::Json)
        .map_err(|error| ViewError::Json(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_body() {
        let view = view(Some("application/json"), None, br#"{"a":1}"#).unwrap();
        assert_eq!(view, ResponseView::Json(serde_json::json!({"a": 1})));
    }

    #[test]
    fn sse_body_with_charset() {
        let body = b"event: ping\ndata: {}\n\n";
        match view(Some("Text/Event-Stream; charset=utf-8"), None, body).unwrap() {
            ResponseView::Events(events) => {
                assert_eq!(events.len(), 1);
                assert_eq!(events[0].event.as_deref(), Some("ping"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unsupported_encoding_is_decode_error() {
        assert!(matches!(
            view(Some("application/json"), Some("br"), b"{}"),
            Err(ViewError::Decode(_))
        ));
    }

    #[test]
    fn invalid_json_is_json_error() {
        assert!(matches!(
            view(Some("application/json"), None, b"nope"),
            Err(ViewError::Json(_))
        ));
    }
}
