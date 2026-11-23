use ralph_core::chat_memory::Node;
use ralph_core::chat_thread::ThreadName;

const MAIN: &str = "main";
const TOKEN_KEY: &str = "token";
pub(crate) const DEFAULT_LIMIT: usize = 50;
pub(crate) const MAX_LIMIT: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Scope {
    Main,
    Thread(ThreadName),
}

impl Scope {
    pub(crate) fn label(&self) -> &str {
        match self {
            Scope::Main => MAIN,
            Scope::Thread(name) => name.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemoryQuery {
    pub thread: Scope,
    pub node: Option<Node>,
    pub after: usize,
    pub limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueryError {
    Encoding,
    UnknownKey,
    DuplicateKey,
    Thread,
    Node,
    After,
    Limit,
}

impl QueryError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            QueryError::Encoding => "query is not valid percent-encoded UTF-8",
            QueryError::UnknownKey => "unknown query key",
            QueryError::DuplicateKey => "duplicate query key",
            QueryError::Thread => "thread is not main or a valid thread name",
            QueryError::Node => "node is not a valid node id",
            QueryError::After => "after is not a non-negative integer",
            QueryError::Limit => "limit is not a positive integer",
        }
    }
}

#[derive(Default)]
struct Fields {
    thread: Option<String>,
    node: Option<String>,
    after: Option<String>,
    limit: Option<String>,
}

impl Fields {
    fn set(&mut self, key: &str, value: String) -> Result<(), QueryError> {
        let slot = match key {
            "thread" => &mut self.thread,
            "node" => &mut self.node,
            "after" => &mut self.after,
            "limit" => &mut self.limit,
            TOKEN_KEY => return Ok(()),
            _ => return Err(QueryError::UnknownKey),
        };
        if slot.is_some() {
            return Err(QueryError::DuplicateKey);
        }
        *slot = Some(value);
        Ok(())
    }
}

fn hex_value(byte: u8) -> Option<u8> {
    char::from(byte)
        .to_digit(16)
        .and_then(|digit| u8::try_from(digit).ok())
}

fn percent_decode(raw: &str) -> Result<String, QueryError> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let high = bytes.get(i + 1).copied().and_then(hex_value);
            let low = bytes.get(i + 2).copied().and_then(hex_value);
            let (Some(high), Some(low)) = (high, low) else {
                return Err(QueryError::Encoding);
            };
            out.push(high << 4 | low);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| QueryError::Encoding)
}

fn split_fields(raw: &str) -> Result<Fields, QueryError> {
    let mut fields = Fields::default();
    for segment in raw.split('&').filter(|segment| !segment.is_empty()) {
        let (key, value) = segment.split_once('=').unwrap_or((segment, ""));
        fields.set(&percent_decode(key)?, percent_decode(value)?)?;
    }
    Ok(fields)
}

fn parse_scope(value: Option<&str>) -> Result<Scope, QueryError> {
    match value {
        None | Some(MAIN) => Ok(Scope::Main),
        Some(name) => ThreadName::parse(name)
            .map(Scope::Thread)
            .map_err(|_| QueryError::Thread),
    }
}

fn digits(value: &str) -> Option<&str> {
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())).then_some(value)
}

fn parse_after(value: Option<&str>) -> Result<usize, QueryError> {
    value.map_or(Ok(0), |text| {
        digits(text)
            .and_then(|text| text.parse().ok())
            .ok_or(QueryError::After)
    })
}

fn parse_limit(value: Option<&str>) -> Result<usize, QueryError> {
    value.map_or(Ok(DEFAULT_LIMIT), |text| {
        let number = digits(text)
            .map(|text| text.parse::<usize>().unwrap_or(usize::MAX))
            .ok_or(QueryError::Limit)?;
        match number {
            0 => Err(QueryError::Limit),
            number => Ok(number.min(MAX_LIMIT)),
        }
    })
}

pub(crate) fn parse_query(raw: &str) -> Result<MemoryQuery, QueryError> {
    let fields = split_fields(raw)?;
    Ok(MemoryQuery {
        thread: parse_scope(fields.thread.as_deref())?,
        node: fields
            .node
            .as_deref()
            .map(|text| Node::parse(text).map_err(|_| QueryError::Node))
            .transpose()?,
        after: parse_after(fields.after.as_deref())?,
        limit: parse_limit(fields.limit.as_deref())?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(name: &str) -> Scope {
        Scope::Thread(ThreadName::parse(name).unwrap())
    }

    fn ok(raw: &str) -> MemoryQuery {
        parse_query(raw).unwrap()
    }

    #[test]
    fn empty_query_uses_defaults() {
        assert_eq!(
            ok(""),
            MemoryQuery {
                thread: Scope::Main,
                node: None,
                after: 0,
                limit: DEFAULT_LIMIT,
            }
        );
    }

    #[test]
    fn token_is_ignored() {
        assert_eq!(ok("token=abc"), ok(""));
        assert_eq!(ok("token=a&token=b"), ok(""));
        assert_eq!(ok("limit=3&token=abc").limit, 3);
    }

    #[test]
    fn thread_main_and_named() {
        assert_eq!(ok("thread=main").thread, Scope::Main);
        assert_eq!(ok("thread=plan-2").thread, thread("plan-2"));
    }

    #[test]
    fn thread_invalid_is_rejected() {
        for raw in [
            "thread=",
            "thread=Bad",
            "thread=-x",
            "thread=a_b",
            "thread=a%2Fb",
        ] {
            assert_eq!(parse_query(raw), Err(QueryError::Thread), "{raw}");
        }
    }

    #[test]
    fn node_forms() {
        assert_eq!(ok("node=5").node, Some(Node::new(5, 6).unwrap()));
        assert_eq!(ok("node=4-7").node, Some(Node::new(4, 8).unwrap()));
    }

    #[test]
    fn node_invalid_is_rejected() {
        for raw in [
            "node=",
            "node=x",
            "node=4-6",
            "node=7-4",
            "node=-1",
            "node=1-2-3",
        ] {
            assert_eq!(parse_query(raw), Err(QueryError::Node), "{raw}");
        }
    }

    #[test]
    fn after_parses_and_rejects() {
        assert_eq!(ok("after=0").after, 0);
        assert_eq!(ok("after=17").after, 17);
        for raw in [
            "after=",
            "after=-1",
            "after=1.5",
            "after=x",
            "after=99999999999999999999999",
        ] {
            assert_eq!(parse_query(raw), Err(QueryError::After), "{raw}");
        }
    }

    #[test]
    fn limit_clamps_and_rejects() {
        assert_eq!(ok("limit=1").limit, 1);
        assert_eq!(ok("limit=200").limit, MAX_LIMIT);
        assert_eq!(ok("limit=201").limit, MAX_LIMIT);
        assert_eq!(ok("limit=99999999999999999999999").limit, MAX_LIMIT);
        for raw in ["limit=0", "limit=", "limit=-3", "limit=ten", "limit=2.5"] {
            assert_eq!(parse_query(raw), Err(QueryError::Limit), "{raw}");
        }
    }

    #[test]
    fn unknown_and_duplicate_keys_are_rejected() {
        assert_eq!(parse_query("foo=1"), Err(QueryError::UnknownKey));
        assert_eq!(parse_query("Limit=1"), Err(QueryError::UnknownKey));
        assert_eq!(parse_query("limit"), Err(QueryError::Limit));
        assert_eq!(
            parse_query("limit=1&limit=2"),
            Err(QueryError::DuplicateKey)
        );
        assert_eq!(
            parse_query("thread=a&thread=b"),
            Err(QueryError::DuplicateKey)
        );
        assert_eq!(
            parse_query("lim%69t=1&limit=2"),
            Err(QueryError::DuplicateKey)
        );
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(ok("thread=pl%61n").thread, thread("plan"));
        assert_eq!(ok("node=4%2D7").node, Some(Node::new(4, 8).unwrap()));
        assert_eq!(ok("after=%31%30").after, 10);
    }

    #[test]
    fn plus_is_not_a_space() {
        assert_eq!(parse_query("thread=a+b"), Err(QueryError::Thread));
        assert_eq!(parse_query("a+b=1"), Err(QueryError::UnknownKey));
    }

    #[test]
    fn bad_encoding_is_rejected() {
        for raw in [
            "thread=%",
            "thread=%4",
            "thread=%zz",
            "thread=%ff",
            "thread=%c3%28",
            "%ff=1",
        ] {
            assert_eq!(parse_query(raw), Err(QueryError::Encoding), "{raw}");
        }
    }

    #[test]
    fn empty_segments_are_skipped() {
        assert_eq!(ok("&&limit=2&"), ok("limit=2"));
    }

    #[test]
    fn messages_are_short_and_never_echo_input() {
        let hostile = "<script>alert(1)</script>";
        for raw in [
            format!("thread={hostile}"),
            format!("node={hostile}"),
            format!("after={hostile}"),
            format!("limit={hostile}"),
            format!("{hostile}=1"),
        ] {
            let message = parse_query(&raw).unwrap_err().message();
            assert!(!message.is_empty() && message.len() < 60, "{message}");
            assert!(!message.contains("script"), "{message}");
        }
    }
}
