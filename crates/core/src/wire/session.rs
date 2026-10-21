use chrono::{DateTime, Utc};

pub fn session_id(now: DateTime<Utc>, suffix: u32) -> String {
    format!("{}-{suffix:08x}", now.format("%Y%m%dT%H%M%SZ"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn sortable_with_padded_suffix() {
        let early = Utc.with_ymd_and_hms(2026, 10, 21, 16, 24, 29).unwrap();
        let late = Utc.with_ymd_and_hms(2026, 10, 21, 16, 24, 30).unwrap();
        assert_eq!(session_id(early, 0x1a2b3c4d), "20261021T162429Z-1a2b3c4d");
        assert_eq!(session_id(early, 1), "20261021T162429Z-00000001");
        assert!(session_id(early, u32::MAX) < session_id(late, 0));
    }
}
