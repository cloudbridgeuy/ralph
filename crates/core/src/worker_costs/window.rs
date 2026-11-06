use chrono::{DateTime, Duration, FixedOffset, NaiveTime, Utc};

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

#[cfg(test)]
mod tests;
