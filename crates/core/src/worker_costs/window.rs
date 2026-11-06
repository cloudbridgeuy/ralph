use chrono::{DateTime, Duration, FixedOffset, NaiveTime, Offset, TimeZone, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Window {
    Today,
    Week,
    #[default]
    All,
}

pub fn window_start(
    now: DateTime<Utc>,
    offset: FixedOffset,
    window: Window,
) -> Option<DateTime<Utc>> {
    match window {
        Window::All => None,
        Window::Week => now.checked_sub_signed(Duration::days(7)),
        Window::Today => now
            .with_timezone(&offset)
            .date_naive()
            .and_time(NaiveTime::MIN)
            .and_utc()
            .checked_sub_signed(Duration::seconds(i64::from(offset.local_minus_utc()))),
    }
}

pub fn midnight_offset<Tz: TimeZone>(now: &DateTime<Tz>) -> FixedOffset {
    let current = now.offset().fix();
    now.timezone()
        .from_local_datetime(&now.date_naive().and_time(NaiveTime::MIN))
        .earliest()
        .map_or(current, |midnight| midnight.offset().fix())
}

pub(crate) fn instant(at: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

pub fn in_window(at: &str, since: Option<DateTime<Utc>>) -> bool {
    since.is_none_or(|since| instant(at).is_some_and(|at| at >= since))
}

#[cfg(test)]
mod tests;
