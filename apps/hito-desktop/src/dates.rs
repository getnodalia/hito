//! Dates as people type and read them: `YYYY-MM-DD` in the computer's time zone. A due date means
//! the end of that day, so a task due today is not overdue until tonight.

use chrono::{Local, NaiveDate, TimeZone};

/// The day a timestamp (milliseconds) falls on here, as `YYYY-MM-DD`.
pub fn date_text(at: i64) -> String {
    Local
        .timestamp_millis_opt(at)
        .single()
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// A short readable day ("18 Nov 2026").
pub fn short(at: i64) -> String {
    Local
        .timestamp_millis_opt(at)
        .single()
        .map(|d| d.format("%-d %b %Y").to_string())
        .unwrap_or_default()
}

/// Day and time, for activity and comments.
pub fn when(at: i64) -> String {
    Local
        .timestamp_millis_opt(at)
        .single()
        .map(|d| d.format("%-d %b %Y, %H:%M").to_string())
        .unwrap_or_default()
}

/// A typed date: `Some(None)` for an empty field (no date), `Some(Some(ms))` for the end of that
/// day here, `None` when it is not a date.
pub fn parse_date(text: &str) -> Option<Option<i64>> {
    let text = text.trim();
    if text.is_empty() {
        return Some(None);
    }
    let day = NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()?;
    let end = day.and_hms_opt(23, 59, 59)?;
    let local = Local.from_local_datetime(&end).earliest()?;
    Some(Some(local.timestamp_millis()))
}

/// Now, in milliseconds.
pub fn now_ms() -> i64 {
    Local::now().timestamp_millis()
}

/// Whole days from now until `at` (negative when past), counted by calendar day here.
pub fn days_until(at: i64) -> i64 {
    let today = Local::now().date_naive();
    Local
        .timestamp_millis_opt(at)
        .single()
        .map(|d| (d.date_naive() - today).num_days())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_dates_round_trip() {
        assert_eq!(parse_date("  "), Some(None));
        assert_eq!(parse_date("mañana"), None);
        assert_eq!(parse_date("2026-02-30"), None);
        let Some(Some(at)) = parse_date("2026-11-18") else {
            panic!("a valid date");
        };
        assert_eq!(date_text(at), "2026-11-18");
    }
}
