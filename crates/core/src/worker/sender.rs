use super::{WorkerId, WorkerIdError};
use crate::chat_thread::{ThreadName, THREAD_REPORT_PREFIX};
use crate::inbox_post::{validate_source, EXTERNAL_REPORT_PREFIX};
use crate::schedule::SCHEDULE_ID_PREFIX;
use crate::watch::WATCH_ID_PREFIX;
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WatchId(String);

impl WatchId {
    pub fn parse(text: &str) -> Result<Self, SenderError> {
        [WATCH_ID_PREFIX, SCHEDULE_ID_PREFIX]
            .into_iter()
            .filter_map(|prefix| text.strip_prefix(prefix))
            .any(|name| !name.is_empty())
            .then(|| Self(text.to_string()))
            .ok_or_else(|| SenderError::Watch(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceName(String);

impl SourceName {
    pub fn parse(text: &str) -> Result<Self, SenderError> {
        validate_source(text)
            .map(|()| Self(text.to_string()))
            .map_err(SenderError::External)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Sender {
    Worker(WorkerId),
    Watch(WatchId),
    Thread(ThreadName),
    External(SourceName),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SenderError {
    Worker(WorkerIdError),
    Watch(String),
    Thread(String),
    External(String),
}

impl fmt::Display for SenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Worker(error) => error.fmt(f),
            Self::Watch(text) => write!(f, "Watch id '{text}' has no name after its prefix."),
            Self::Thread(reason) | Self::External(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for SenderError {}

impl Sender {
    pub fn parse(text: &str) -> Result<Self, SenderError> {
        if let Some(name) = text.strip_prefix(THREAD_REPORT_PREFIX) {
            return ThreadName::parse(name)
                .map(Self::Thread)
                .map_err(SenderError::Thread);
        }
        if let Some(source) = text.strip_prefix(EXTERNAL_REPORT_PREFIX) {
            return SourceName::parse(source).map(Self::External);
        }
        if text.starts_with(WATCH_ID_PREFIX) || text.starts_with(SCHEDULE_ID_PREFIX) {
            return WatchId::parse(text).map(Self::Watch);
        }
        WorkerId::parse(text)
            .map(Self::Worker)
            .map_err(SenderError::Worker)
    }

    pub fn worker(&self) -> Option<&WorkerId> {
        match self {
            Self::Worker(id) => Some(id),
            Self::Watch(_) | Self::Thread(_) | Self::External(_) => None,
        }
    }
}

impl fmt::Display for Sender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Worker(id) => f.write_str(id.as_str()),
            Self::Watch(id) => f.write_str(id.as_str()),
            Self::Thread(name) => write!(f, "{THREAD_REPORT_PREFIX}{name}"),
            Self::External(source) => write!(f, "{EXTERNAL_REPORT_PREFIX}{}", source.as_str()),
        }
    }
}

impl Serialize for Sender {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Sender {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SenderVisitor;

        impl Visitor<'_> for SenderVisitor {
            type Value = Sender;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a report sender")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Sender::parse(value).map_err(E::custom)
            }
        }

        deserializer.deserialize_str(SenderVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::WorkerId;
    use std::collections::HashMap;

    #[test]
    fn sender_round_trips_each_prefix() {
        for raw in [
            "w1",
            "GUZ-1911",
            "watch:ci",
            "schedule:nightly",
            "thread:side",
            "external:ci",
        ] {
            let sender = Sender::parse(raw).unwrap();
            assert_eq!(sender.to_string(), raw);
            let encoded = serde_json::to_string(&sender).unwrap();
            assert_eq!(encoded, format!("\"{raw}\""));
            assert_eq!(serde_json::from_str::<Sender>(&encoded).unwrap(), sender);
        }
    }

    #[test]
    fn sender_parse_picks_the_variant_by_prefix() {
        assert!(matches!(Sender::parse("w1"), Ok(Sender::Worker(_))));
        assert!(matches!(Sender::parse("watch:ci"), Ok(Sender::Watch(_))));
        assert!(matches!(Sender::parse("schedule:n"), Ok(Sender::Watch(_))));
        assert!(matches!(
            Sender::parse("thread:side"),
            Ok(Sender::Thread(_))
        ));
        assert!(matches!(
            Sender::parse("external:ci"),
            Ok(Sender::External(_))
        ));
    }

    #[test]
    fn malformed_senders_are_refused() {
        for raw in [
            "",
            "bad id!",
            "watch:",
            "schedule:",
            "thread:",
            "thread:Bad",
            "thread:main",
            "external:",
            "external:Bad",
        ] {
            assert!(Sender::parse(raw).is_err(), "raw={raw:?}");
            assert!(serde_json::from_value::<Sender>(serde_json::json!(raw)).is_err());
        }
    }

    #[test]
    fn worker_returns_the_id_only_for_worker_senders() {
        let worker = WorkerId::parse("w1").unwrap();
        assert_eq!(Sender::parse("w1").unwrap().worker(), Some(&worker));
        for raw in ["watch:ci", "schedule:n", "thread:side", "external:ci"] {
            assert_eq!(Sender::parse(raw).unwrap().worker(), None, "raw={raw:?}");
        }
    }

    #[test]
    fn worker_id_keys_a_hash_map() {
        let states: HashMap<WorkerId, u8> = [(WorkerId::parse("w1").unwrap(), 1)].into();
        assert_eq!(states.get(&WorkerId::parse("w1").unwrap()), Some(&1));
    }
}
