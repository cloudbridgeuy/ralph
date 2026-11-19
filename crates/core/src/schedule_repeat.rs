use crate::schedule::{format_span, parse_after, TaskKind};
use chrono::{DateTime, Days, Duration, LocalResult, NaiveDate, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

pub const COMMAND_MIN_EVERY_SECS: u64 = 600;
pub const WORKER_MIN_EVERY_SECS: u64 = 3600;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Repeat {
    Every { every_secs: u64 },
    Daily { daily: String },
}

const DAILY_FORMAT: &str = "%H:%M";
const GAP_SEARCH_MINUTES: i64 = 24 * 60;
const DAY_SEARCH: u64 = 3;

fn every_error(detail: &str) -> String {
    format!("every {detail}")
}

fn parse_daily_time(text: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(text.trim(), DAILY_FORMAT).ok()
}

pub fn parse_repeat(raw: &str) -> Result<Repeat, String> {
    let text = raw.trim();
    if text == "daily" || text.starts_with("daily ") {
        let rest = text.trim_start_matches("daily");
        return parse_daily_time(rest)
            .map(|time| Repeat::Daily {
                daily: time.format(DAILY_FORMAT).to_string(),
            })
            .ok_or_else(|| every_error(&format!("'{text}' needs a time like daily 02:00.")));
    }
    parse_after(text)
        .map(|span| Repeat::Every {
            every_secs: span.as_secs(),
        })
        .map_err(|error| {
            every_error(&format!(
                "must be a duration like 6h or daily HH:MM: {error}"
            ))
        })
}

pub fn check_min(repeat: &Repeat, kind: &TaskKind) -> Result<(), String> {
    let minimum = match kind {
        TaskKind::Command { .. } => COMMAND_MIN_EVERY_SECS,
        TaskKind::Worker { .. } => WORKER_MIN_EVERY_SECS,
    };
    match repeat {
        Repeat::Every { every_secs } if *every_secs < minimum => Err(every_error(&format!(
            "must be at least {} for this task.",
            format_span(minimum)
        ))),
        _ => Ok(()),
    }
}

fn next_every(every_secs: u64, anchor: DateTime<Utc>, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    if every_secs == 0 {
        return None;
    }
    if anchor > now {
        return Some(anchor);
    }
    let elapsed = u64::try_from((now - anchor).num_seconds()).ok()?;
    let steps = (elapsed / every_secs).checked_add(1)?;
    let offset = i64::try_from(steps.checked_mul(every_secs)?).ok()?;
    anchor.checked_add_signed(Duration::try_seconds(offset)?)
}

fn local_instant<Tz: TimeZone>(
    zone: &Tz,
    date: NaiveDate,
    time: NaiveTime,
) -> Option<DateTime<Utc>> {
    (0..GAP_SEARCH_MINUTES).find_map(|minutes| {
        let wanted = date
            .and_time(time)
            .checked_add_signed(Duration::try_minutes(minutes)?)?;
        match zone.from_local_datetime(&wanted) {
            LocalResult::Single(moment) => Some(moment.with_timezone(&Utc)),
            LocalResult::Ambiguous(first, second) => Some(first.min(second).with_timezone(&Utc)),
            LocalResult::None => None,
        }
    })
}

fn next_daily<Tz: TimeZone>(
    daily: &str,
    anchor: DateTime<Utc>,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Option<DateTime<Utc>> {
    let time = parse_daily_time(daily)?;
    let today = now.with_timezone(zone).date_naive();
    let anchor_date = (anchor <= now).then(|| anchor.with_timezone(zone).date_naive());
    (0..=DAY_SEARCH)
        .filter_map(|days| today.checked_add_days(Days::new(days)))
        .filter(|date| anchor_date.is_none_or(|fired| *date > fired))
        .filter_map(|date| local_instant(zone, date, time))
        .find(|instant| *instant > now)
}

pub fn next_fire<Tz: TimeZone>(
    repeat: &Repeat,
    anchor: DateTime<Utc>,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Option<DateTime<Utc>> {
    match repeat {
        Repeat::Every { every_secs } => next_every(*every_secs, anchor, now),
        Repeat::Daily { daily } => next_daily(daily, anchor, now, zone),
    }
}

pub fn repeat_label(repeat: &Repeat) -> String {
    match repeat {
        Repeat::Every { every_secs } => format!("every {}", format_span(*every_secs)),
        Repeat::Daily { daily } => format!("daily {daily}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::{Task, TaskStatus};
    use chrono::{
        Duration as ChronoDuration, FixedOffset, LocalResult, NaiveDate, NaiveDateTime, NaiveTime,
    };

    fn utc(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn every(secs: u64) -> Repeat {
        Repeat::Every { every_secs: secs }
    }

    fn daily(text: &str) -> Repeat {
        Repeat::Daily {
            daily: text.to_string(),
        }
    }

    fn command() -> TaskKind {
        TaskKind::Command {
            command: "ls".to_string(),
        }
    }

    fn worker() -> TaskKind {
        TaskKind::Worker {
            brief: "b".to_string(),
        }
    }

    #[derive(Clone)]
    struct Switch {
        at: DateTime<Utc>,
        before: i32,
        after: i32,
    }

    impl Switch {
        fn spring() -> Self {
            Switch {
                at: utc("2026-03-29T01:00:00Z"),
                before: 1,
                after: 2,
            }
        }

        fn fall() -> Self {
            Switch {
                at: utc("2026-10-25T01:00:00Z"),
                before: 2,
                after: 1,
            }
        }

        fn at_utc(&self, moment: &NaiveDateTime) -> FixedOffset {
            let hours = if *moment < self.at.naive_utc() {
                self.before
            } else {
                self.after
            };
            FixedOffset::east_opt(hours * 3600).unwrap()
        }
    }

    impl TimeZone for Switch {
        type Offset = FixedOffset;

        fn from_offset(_: &FixedOffset) -> Self {
            Switch::spring()
        }

        fn offset_from_local_date(&self, local: &NaiveDate) -> LocalResult<FixedOffset> {
            self.offset_from_local_datetime(&local.and_time(NaiveTime::MIN))
        }

        fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> LocalResult<FixedOffset> {
            let mut fits: Vec<FixedOffset> = Vec::new();
            for hours in [self.before, self.after] {
                let offset = FixedOffset::east_opt(hours * 3600).unwrap();
                let moment = *local - ChronoDuration::seconds(offset.local_minus_utc().into());
                if self.at_utc(&moment) == offset && !fits.contains(&offset) {
                    fits.push(offset);
                }
            }
            match fits.as_slice() {
                [one] => LocalResult::Single(*one),
                [a, b] => LocalResult::Ambiguous(*a, *b),
                _ => LocalResult::None,
            }
        }

        fn offset_from_utc_date(&self, moment: &NaiveDate) -> FixedOffset {
            self.at_utc(&moment.and_time(NaiveTime::MIN))
        }

        fn offset_from_utc_datetime(&self, moment: &NaiveDateTime) -> FixedOffset {
            self.at_utc(moment)
        }
    }

    fn plus_one() -> FixedOffset {
        FixedOffset::east_opt(3600).unwrap()
    }

    fn row(repeat: Option<Repeat>) -> Task {
        Task {
            id: "a".to_string(),
            kind: command(),
            follow_up: None,
            at: utc("2026-01-01T10:00:00Z"),
            timeout_secs: None,
            status: TaskStatus::Pending,
            late: false,
            deadline: None,
            repeat,
        }
    }

    #[test]
    fn an_old_row_without_repeat_loads_and_saves_without_the_key() {
        let old = r#"{"id":"a","kind":"command","command":"ls","follow_up":null,"at":"2026-01-01T10:00:00Z","timeout_secs":null,"status":"pending","late":false}"#;
        let task: Task = serde_json::from_str(old).unwrap();
        assert_eq!(task.repeat, None);
        let saved = serde_json::to_string(&task).unwrap();
        assert!(!saved.contains("repeat"), "{saved}");
    }

    #[test]
    fn both_repeat_shapes_round_trip() {
        for (repeat, json) in [
            (every(21600), r#"{"every_secs":21600}"#),
            (daily("02:00"), r#"{"daily":"02:00"}"#),
        ] {
            assert_eq!(serde_json::to_string(&repeat).unwrap(), json);
            assert_eq!(serde_json::from_str::<Repeat>(json).unwrap(), repeat);
            let task = row(Some(repeat));
            let saved = serde_json::to_string(&task).unwrap();
            assert!(saved.contains(json), "{saved}");
            assert_eq!(serde_json::from_str::<Task>(&saved).unwrap(), task);
        }
    }

    #[test]
    fn parse_accepts_durations_and_daily_times() {
        assert_eq!(parse_repeat("6h").unwrap(), every(21600));
        assert_eq!(parse_repeat(" 1h30m ").unwrap(), every(5400));
        assert_eq!(parse_repeat("daily 02:00").unwrap(), daily("02:00"));
        assert_eq!(parse_repeat("  daily 2:05 ").unwrap(), daily("02:05"));
    }

    #[test]
    fn parse_refuses_garbage_with_an_every_message() {
        for bad in [
            "",
            "  ",
            "soon",
            "0s",
            "daily",
            "daily 25:00",
            "Daily 02:00",
            "daily noon",
            "weekly 02:00",
        ] {
            let error = parse_repeat(bad).unwrap_err();
            assert!(error.starts_with("every"), "{bad}: {error}");
        }
    }

    #[test]
    fn minimums_depend_on_the_task_kind() {
        let error = check_min(&every(599), &command()).unwrap_err();
        assert!(error.contains("10m"), "{error}");
        assert!(check_min(&every(600), &command()).is_ok());
        let error = check_min(&every(3599), &worker()).unwrap_err();
        assert!(error.contains("1h"), "{error}");
        assert!(check_min(&every(3600), &worker()).is_ok());
        assert!(check_min(&daily("00:01"), &worker()).is_ok());
    }

    #[test]
    fn every_skips_missed_runs_and_stays_on_the_grid() {
        let anchor = utc("2026-01-01T10:20:00Z");
        let now = utc("2026-01-02T10:30:00Z");
        assert_eq!(
            next_fire(&every(3600), anchor, now, &Utc),
            Some(utc("2026-01-02T11:20:00Z"))
        );
    }

    #[test]
    fn every_is_never_equal_to_now() {
        let anchor = utc("2026-01-01T10:00:00Z");
        let now = utc("2026-01-01T12:00:00Z");
        assert_eq!(
            next_fire(&every(3600), anchor, now, &Utc),
            Some(utc("2026-01-01T13:00:00Z"))
        );
        assert_eq!(
            next_fire(&every(3600), anchor, anchor, &Utc),
            Some(utc("2026-01-01T11:00:00Z"))
        );
    }

    #[test]
    fn every_refuses_zero_and_overflow() {
        let anchor = utc("2026-01-01T10:00:00Z");
        assert_eq!(next_fire(&every(0), anchor, anchor, &Utc), None);
        assert_eq!(next_fire(&every(u64::MAX), anchor, anchor, &Utc), None);
    }

    #[test]
    fn daily_fires_today_when_the_time_is_ahead() {
        let anchor = utc("2026-01-01T08:00:00Z");
        let now = utc("2026-01-02T00:30:00Z");
        assert_eq!(
            next_fire(&daily("02:00"), anchor, now, &plus_one()),
            Some(utc("2026-01-02T01:00:00Z"))
        );
    }

    #[test]
    fn daily_fires_tomorrow_when_the_time_has_passed() {
        let anchor = utc("2025-12-31T01:00:00Z");
        let now = utc("2026-01-02T03:00:00Z");
        assert_eq!(
            next_fire(&daily("02:00"), anchor, now, &plus_one()),
            Some(utc("2026-01-03T01:00:00Z"))
        );
    }

    #[test]
    fn daily_in_a_spring_forward_gap_fires_when_the_gap_ends() {
        let anchor = utc("2026-03-28T01:30:00Z");
        let now = utc("2026-03-28T12:00:00Z");
        assert_eq!(
            next_fire(&daily("02:30"), anchor, now, &Switch::spring()),
            Some(utc("2026-03-29T01:00:00Z"))
        );
    }

    #[test]
    fn daily_in_a_fall_back_hour_fires_once_per_day() {
        let zone = Switch::fall();
        let anchor = utc("2026-10-24T00:30:00Z");
        let now = utc("2026-10-24T12:00:00Z");
        let first = next_fire(&daily("02:30"), anchor, now, &zone).unwrap();
        assert_eq!(first, utc("2026-10-25T00:30:00Z"));
        let second = next_fire(&daily("02:30"), first, first, &zone).unwrap();
        assert_eq!(second, utc("2026-10-26T01:30:00Z"));
    }

    #[test]
    fn daily_with_a_broken_time_has_no_next_fire() {
        let now = utc("2026-01-01T00:00:00Z");
        assert_eq!(next_fire(&daily("noon"), now, now, &Utc), None);
    }

    #[test]
    fn labels_name_the_repeat() {
        assert_eq!(repeat_label(&every(21600)), "every 6h");
        assert_eq!(repeat_label(&daily("02:00")), "daily 02:00");
    }
}
