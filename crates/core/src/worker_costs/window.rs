use chrono::{DateTime, Duration, NaiveTime, TimeZone, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Window {
    Today,
    Week,
    #[default]
    All,
}

const GAP_STEP_MINUTES: i64 = 15;
const GAP_STEPS: i64 = 16;

pub fn window_start<Tz: TimeZone>(now: &DateTime<Tz>, window: Window) -> Option<DateTime<Utc>> {
    match window {
        Window::All => None,
        Window::Week => now
            .with_timezone(&Utc)
            .checked_sub_signed(Duration::days(7)),
        Window::Today => today_start(now),
    }
}

/// Start of the local day of `now`. A midnight that does not exist moves to the
/// first valid local time after the gap, found in 15-minute steps up to 4 hours.
pub fn today_start<Tz: TimeZone>(now: &DateTime<Tz>) -> Option<DateTime<Utc>> {
    let zone = now.timezone();
    let midnight = now.date_naive().and_time(NaiveTime::MIN);
    (0..=GAP_STEPS)
        .find_map(|step| {
            let local = midnight.checked_add_signed(Duration::minutes(step * GAP_STEP_MINUTES))?;
            zone.from_local_datetime(&local).earliest()
        })
        .map(|start| start.with_timezone(&Utc))
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
