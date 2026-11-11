use activsync::logging::{self, Level};
use chrono::{TimeZone, Utc};
use regex::Regex;

#[test]
fn lines_read_like_the_python_apps() {
    logging::configure("warning", "UTC");
    let line = logging::line(Level::Warning, "sync", format_args!("garmin fetch failed: {}", "boom"));
    let shape = Regex::new(r"^\d{4}-\d\d-\d\d \d\d:\d\d:\d\d UTC  WARNING sync     garmin fetch failed: boom$").unwrap();
    assert!(shape.is_match(&line), "{line:?}");
    assert!(logging::enabled(Level::Error) && logging::enabled(Level::Warning));
    assert!(!logging::enabled(Level::Info));

    logging::set_timezone("Europe/Stockholm");
    let summer = Utc.with_ymd_and_hms(2026, 7, 1, 10, 0, 0).unwrap();
    assert_eq!(logging::format_time(summer), "2026-07-01 12:00:00 CEST");
    logging::set_timezone("Mars/Olympus_Mons");
    assert_eq!(logging::format_time(summer), "2026-07-01 12:00:00 CEST", "an unknown zone keeps the current one");

    logging::configure("nonsense", "UTC");
    assert!(logging::enabled(Level::Info) && !logging::enabled(Level::Debug));
}

#[test]
fn routine_successes_stay_out_of_the_access_log() {
    assert!(logging::routine("/health", 200));
    assert!(logging::routine("/assets/app-1234.js", 304));
    assert!(logging::routine("/favicon.ico", 200));
    assert!(!logging::routine("/health", 503), "a failing probe is worth a line");
    assert!(!logging::routine("/assets/missing.css", 404));
    assert!(!logging::routine("/api/v1/app", 200));
}
