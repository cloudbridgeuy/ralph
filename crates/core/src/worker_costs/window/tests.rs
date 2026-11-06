use super::*;
use chrono::{FixedOffset, TimeZone};

fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
}

fn east(hours: i32) -> FixedOffset {
    FixedOffset::east_opt(hours * 3600).unwrap()
}

#[test]
fn all_time_has_no_start() {
    assert_eq!(
        window_start(utc(2026, 11, 6, 12, 0), east(0), Window::All),
        None
    );
}

#[test]
fn seven_days_is_168_hours_before_now() {
    assert_eq!(
        window_start(utc(2026, 11, 6, 12, 30), east(-5), Window::Week),
        Some(utc(2026, 10, 30, 12, 30))
    );
}

#[test]
fn today_starts_at_utc_midnight_for_a_zero_offset() {
    assert_eq!(
        window_start(utc(2026, 11, 6, 23, 59), east(0), Window::Today),
        Some(utc(2026, 11, 6, 0, 0))
    );
}

#[test]
fn today_starts_at_local_midnight_west_of_utc() {
    assert_eq!(
        window_start(utc(2026, 11, 6, 3, 0), east(-5), Window::Today),
        Some(utc(2026, 11, 5, 5, 0))
    );
}

#[test]
fn today_starts_at_local_midnight_east_of_utc() {
    assert_eq!(
        window_start(utc(2026, 11, 6, 20, 0), east(3), Window::Today),
        Some(utc(2026, 11, 5, 21, 0))
    );
}

#[test]
fn today_handles_a_half_hour_offset() {
    let offset = FixedOffset::east_opt(5 * 3600 + 1800).unwrap();
    assert_eq!(
        window_start(utc(2026, 11, 6, 1, 0), offset, Window::Today),
        Some(utc(2026, 11, 5, 18, 30))
    );
}
