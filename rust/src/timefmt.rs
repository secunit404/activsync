use chrono::{DateTime, Datelike, Duration, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;

pub const GARMIN_TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

pub fn is_valid_timezone(name: &str) -> bool {
    name.parse::<Tz>().is_ok()
}

pub fn parse_tz(name: &str) -> Tz {
    name.parse().unwrap_or(chrono_tz::Europe::Stockholm)
}

pub fn parse_garmin(start_time: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(start_time, GARMIN_TIME_FORMAT)
        .ok()
        .map(|naive| naive.and_utc())
}

/// Python's `timeutil.parse_timestamp`: ISO-8601 with or without `Z`, a bare
/// date, or Garmin's space-separated form; a naive value reads as UTC.
pub fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    let cleaned = value.trim().replace('Z', "+00:00");
    if cleaned.is_empty() {
        return None;
    }
    if let Ok(aware) = DateTime::parse_from_rfc3339(&cleaned) {
        return Some(aware.with_timezone(&Utc));
    }
    for format in [
        "%Y-%m-%dT%H:%M:%S%.f%:z",
        "%Y-%m-%d %H:%M:%S%.f%:z",
        "%Y-%m-%dT%H:%M%:z",
    ] {
        if let Ok(aware) = DateTime::parse_from_str(&cleaned, format) {
            return Some(aware.with_timezone(&Utc));
        }
    }
    for format in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(&cleaned, format) {
            return Some(naive.and_utc());
        }
    }
    chrono::NaiveDate::parse_from_str(&cleaned, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|naive| naive.and_utc())
}

pub fn to_local(start_time: &str, tz: Tz) -> Option<DateTime<Tz>> {
    parse_garmin(start_time).map(|utc| utc.with_timezone(&tz))
}

/// "9 Jul"
pub fn format_local_date(start_time: &str, tz: Tz) -> String {
    to_local(start_time, tz)
        .map(|local| format!("{} {}", local.day(), local.format("%b")))
        .unwrap_or_default()
}

/// "July 2026"
pub fn format_local_month_year(start_time: &str, tz: Tz) -> String {
    to_local(start_time, tz)
        .map(|local| local.format("%B %Y").to_string())
        .unwrap_or_default()
}

/// "11:00"
pub fn format_local_clock(start_time: &str, tz: Tz) -> String {
    to_local(start_time, tz)
        .map(|local| local.format("%H:%M").to_string())
        .unwrap_or_default()
}

/// Monday 00:00 to the next Monday 00:00 in `tz`, as wall-clock days the way
/// Python's aware-datetime arithmetic counts them across a DST change.
pub fn current_week(now: DateTime<Utc>, tz: Tz) -> (DateTime<Tz>, DateTime<Tz>) {
    let local = now.with_timezone(&tz);
    let monday =
        local.date_naive() - Duration::days(i64::from(local.weekday().num_days_from_monday()));
    let start_naive = monday.and_hms_opt(0, 0, 0).expect("midnight exists");
    let resolve = |naive: NaiveDateTime| {
        tz.from_local_datetime(&naive)
            .earliest()
            .unwrap_or_else(|| tz.from_utc_datetime(&naive))
    };
    (
        resolve(start_naive),
        resolve(start_naive + Duration::days(7)),
    )
}
