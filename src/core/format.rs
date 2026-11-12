//! Text formats that the CLI and the TUI both use: dates, sizes, and durations.

use chrono::{DateTime, FixedOffset, Local, Utc};

use super::settings::DateFormat;

/// Format a date for a list column.
pub fn format_date(d: DateTime<FixedOffset>, f: DateFormat, now: DateTime<Utc>) -> String {
    match f {
        DateFormat::Iso => d.format("%Y-%m-%d").to_string(),
        DateFormat::Local => d.with_timezone(&Local).format("%b %d %H:%M").to_string(),
        DateFormat::Relative => {
            let secs = (now - d.with_timezone(&Utc)).num_seconds().max(0);
            match secs {
                0..60 => "now".into(),
                60..3600 => format!("{}m", secs / 60),
                3600..86400 => format!("{}h", secs / 3600),
                86400..2_592_000 => format!("{}d", secs / 86400),
                2_592_000..31_536_000 => format!("{}mo", secs / 2_592_000),
                _ => format!("{}y", secs / 31_536_000),
            }
        }
    }
}

/// Format a date for a detail view: the date and time, and the relative age.
pub fn format_when(d: DateTime<FixedOffset>, f: DateFormat, now: DateTime<Utc>) -> String {
    match f {
        DateFormat::Relative => format!("{} ({})", d.format("%Y-%m-%d %H:%M"), format_date(d, DateFormat::Relative, now)),
        f => format_date(d, f, now),
    }
}

pub fn human_size(b: u64) -> String {
    match b {
        0..1024 => format!("{b} B"),
        1024..1_048_576 => format!("{:.0} KB", b as f64 / 1024.0),
        _ => format!("{:.1} MB", b as f64 / 1_048_576.0),
    }
}

/// Seconds as Jira time: 8 hours in a day (server setting).
pub fn human_duration(secs: u64) -> String {
    let (d, h, m) = (secs / 28_800, (secs % 28_800) / 3600, (secs % 3600) / 60);
    let mut parts = vec![];
    if d > 0 {
        parts.push(format!("{d}d"));
    }
    if h > 0 {
        parts.push(format!("{h}h"));
    }
    if m > 0 || parts.is_empty() {
        parts.push(format!("{m}m"));
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(s).unwrap()
    }

    #[test]
    fn relative_dates() {
        let now = d("2026-10-19T12:00:00Z").with_timezone(&Utc);
        assert_eq!(format_date(d("2026-10-19T11:59:30Z"), DateFormat::Relative, now), "now");
        assert_eq!(format_date(d("2026-10-19T09:00:00Z"), DateFormat::Relative, now), "3h");
        assert_eq!(format_date(d("2026-10-16T12:00:00Z"), DateFormat::Relative, now), "3d");
        assert_eq!(format_date(d("2026-10-16T12:00:00-07:00"), DateFormat::Iso, now), "2026-10-16");
    }

    #[test]
    fn durations_use_8_hour_days() {
        assert_eq!(human_duration(5400), "1h 30m");
        assert_eq!(human_duration(28_800 + 3600), "1d 1h");
        assert_eq!(human_duration(0), "0m");
    }
}
