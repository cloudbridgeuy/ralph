use super::at;
use ralph_core::watch::WatchKind;
use ralph_core::watch_store::SavedWatch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchShape {
    Plain,
    Timeout,
    Wake,
    Persistent,
}

#[cfg(test)]
impl WatchShape {
    pub const ALL: [WatchShape; 4] = [Self::Plain, Self::Timeout, Self::Wake, Self::Persistent];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchEntry {
    pub shape: WatchShape,
    pub saved: SavedWatch,
}

fn watch(name: &str, command: &str, started: i64) -> SavedWatch {
    SavedWatch {
        id: format!("watch-{name}"),
        command: command.to_string(),
        follow_up: None,
        kind: WatchKind::Normal,
        timeout_secs: None,
        started_at: at(started),
        timeout_deadline: None,
        wake_deadline: None,
    }
}

pub fn watch_specs() -> Vec<WatchEntry> {
    let entry = |shape, saved| WatchEntry { shape, saved };
    vec![
        entry(
            WatchShape::Plain,
            watch("plain", "tail -f demo.log", -3_600),
        ),
        entry(
            WatchShape::Timeout,
            SavedWatch {
                timeout_secs: Some(1_800),
                timeout_deadline: Some(at(1_200)),
                ..watch("timeout", "sh demo-poll.sh", -600)
            },
        ),
        entry(
            WatchShape::Wake,
            SavedWatch {
                follow_up: Some("Demo follow-up: read the log.".to_string()),
                wake_deadline: Some(at(900)),
                ..watch("wake", "tail -f demo-wake.log", -1_200)
            },
        ),
        entry(
            WatchShape::Persistent,
            SavedWatch {
                kind: WatchKind::Persistent,
                ..watch("persistent", "tail -f demo-keep.log", -7_200)
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_watch_shape_has_one_watch() {
        let specs = watch_specs();
        for shape in WatchShape::ALL {
            assert_eq!(
                specs.iter().filter(|entry| entry.shape == shape).count(),
                1,
                "{shape:?}"
            );
        }
        assert_eq!(specs.len(), WatchShape::ALL.len());
    }

    #[test]
    fn watches_have_the_shape_of_their_kind() {
        for entry in watch_specs() {
            let saved = &entry.saved;
            match entry.shape {
                WatchShape::Plain => {
                    assert_eq!(saved.kind, WatchKind::Normal);
                    assert!(saved.timeout_deadline.is_none() && saved.wake_deadline.is_none());
                }
                WatchShape::Timeout => {
                    assert!(saved.timeout_secs.is_some() && saved.timeout_deadline.is_some());
                }
                WatchShape::Wake => assert!(saved.wake_deadline.is_some()),
                WatchShape::Persistent => assert_eq!(saved.kind, WatchKind::Persistent),
            }
        }
    }
}
