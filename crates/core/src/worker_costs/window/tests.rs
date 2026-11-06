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

#[test]
fn no_start_includes_every_row_even_with_an_unreadable_time() {
    assert!(in_window("garbage", None));
    assert!(in_window("2026-11-06T00:00:00Z", None));
}

#[test]
fn a_start_includes_rows_at_or_after_it() {
    let start = Some(utc(2026, 11, 6, 0, 0));
    assert!(in_window("2026-11-06T00:00:00Z", start));
    assert!(in_window("2026-11-06T02:00:00+02:00", start));
    assert!(in_window("2027-01-01T00:00:00Z", start));
    assert!(!in_window("2026-11-05T23:59:59Z", start));
}

#[test]
fn a_start_excludes_an_unreadable_time() {
    assert!(!in_window("garbage", Some(utc(2026, 11, 6, 0, 0))));
}

#[derive(Debug, Clone, Copy)]
struct ZoneOffset {
    fixed: FixedOffset,
    switch: DateTime<Utc>,
}

impl std::fmt::Display for ZoneOffset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.fixed.fmt(f)
    }
}

impl chrono::Offset for ZoneOffset {
    fn fix(&self) -> FixedOffset {
        self.fixed
    }
}

#[derive(Debug, Clone, Copy)]
struct SpringForward {
    switch: DateTime<Utc>,
}

impl SpringForward {
    fn before(&self) -> ZoneOffset {
        ZoneOffset {
            fixed: east(0),
            switch: self.switch,
        }
    }

    fn after(&self) -> ZoneOffset {
        ZoneOffset {
            fixed: east(1),
            switch: self.switch,
        }
    }
}

impl TimeZone for SpringForward {
    type Offset = ZoneOffset;

    fn from_offset(offset: &ZoneOffset) -> Self {
        SpringForward {
            switch: offset.switch,
        }
    }

    fn offset_from_local_date(&self, local: &chrono::NaiveDate) -> chrono::LocalResult<ZoneOffset> {
        self.offset_from_local_datetime(&local.and_time(NaiveTime::MIN))
    }

    fn offset_from_local_datetime(
        &self,
        local: &chrono::NaiveDateTime,
    ) -> chrono::LocalResult<ZoneOffset> {
        let gap_start = self.switch.naive_utc();
        if *local < gap_start {
            chrono::LocalResult::Single(self.before())
        } else if *local >= gap_start + Duration::hours(1) {
            chrono::LocalResult::Single(self.after())
        } else {
            chrono::LocalResult::None
        }
    }

    fn offset_from_utc_date(&self, utc: &chrono::NaiveDate) -> ZoneOffset {
        self.offset_from_utc_datetime(&utc.and_time(NaiveTime::MIN))
    }

    fn offset_from_utc_datetime(&self, utc: &chrono::NaiveDateTime) -> ZoneOffset {
        if *utc >= self.switch.naive_utc() {
            self.after()
        } else {
            self.before()
        }
    }
}

#[test]
fn the_midnight_offset_is_the_one_in_force_at_local_midnight() {
    let zone = SpringForward {
        switch: utc(2026, 3, 8, 2, 0),
    };
    let now = utc(2026, 3, 8, 10, 0).with_timezone(&zone);
    assert_eq!(now.offset().fixed, east(1));
    assert_eq!(midnight_offset(&now), east(0));
}

#[test]
fn a_midnight_that_does_not_exist_falls_back_to_the_current_offset() {
    let zone = SpringForward {
        switch: utc(2026, 3, 8, 0, 0),
    };
    let now = utc(2026, 3, 8, 10, 0).with_timezone(&zone);
    assert_eq!(midnight_offset(&now), east(1));
}

#[test]
fn a_fixed_zone_keeps_its_offset() {
    let now = utc(2026, 11, 6, 12, 0).with_timezone(&east(-5));
    assert_eq!(midnight_offset(&now), east(-5));
}
