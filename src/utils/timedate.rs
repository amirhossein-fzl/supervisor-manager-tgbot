use chrono::{DateTime, Utc};

pub fn unix_to_datetime(timestamp: i64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp(timestamp, 0)
}

pub fn diff_for_humans(timestamp1: i64, timestamp2: i64) -> Option<String> {
    let dt1 = unix_to_datetime(timestamp1)?;
    let dt2 = unix_to_datetime(timestamp2)?;

    let diff = dt2.signed_duration_since(dt1);

    let (unit, value) = match diff.num_seconds().abs() {
        0..=59 => ("second", diff.num_seconds().abs()),
        60..=3599 => ("minute", diff.num_minutes().abs()),
        3600..=86399 => ("hour", diff.num_hours().abs()),
        86400..=2591999 => ("day", diff.num_days().abs()),
        2592000..=31535999 => ("month", (diff.num_days().abs() / 30)),
        _ => ("year", (diff.num_days().abs() / 365)),
    };

    let plural = if value == 1 { "" } else { "s" };
    let hours = diff.num_hours();
    let minutes = diff.num_minutes() % 60;
    let seconds = diff.num_seconds() % 60;

    Some(format!(
        "{} {}{} ago ({:02}:{:02}:{:02})",
        value, unit, plural, hours, minutes, seconds
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_to_datetime_converts_valid_timestamps() {
        let dt = unix_to_datetime(1_700_000_000).expect("timestamp is in range");
        assert_eq!(dt.timestamp(), 1_700_000_000);
    }

    #[test]
    fn unix_to_datetime_rejects_out_of_range_timestamps() {
        assert!(unix_to_datetime(i64::MAX).is_none());
        assert!(unix_to_datetime(i64::MIN).is_none());
    }

    #[test]
    fn diff_for_humans_formats_small_differences() {
        assert_eq!(
            diff_for_humans(1_700_000_000, 1_700_000_030).as_deref(),
            Some("30 seconds ago (00:00:30)")
        );
    }

    #[test]
    fn diff_for_humans_formats_hours() {
        let result = diff_for_humans(1_700_000_000, 1_700_003_600).expect("valid timestamps");
        assert!(result.starts_with("1 hour ago"));
    }

    #[test]
    fn diff_for_humans_rejects_out_of_range_timestamps() {
        assert!(diff_for_humans(i64::MAX, 0).is_none());
        assert!(diff_for_humans(0, i64::MAX).is_none());
    }
}
