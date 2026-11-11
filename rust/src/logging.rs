//! Readable log lines for `docker logs`, as the Python app's
//! `logging_setup.py` writes them: a timestamp in the app's display timezone,
//! the level, the part of the app speaking, and the message. The level comes
//! from `ACTIVSYNC_LOG_LEVEL`; the timezone follows the in-app setting live.

use std::io::Write;
use std::net::SocketAddr;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU8, Ordering};

use axum::extract::{ConnectInfo, Request};
use axum::middleware::Next;
use axum::response::Response;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;

const DEFAULT_TZ: Tz = chrono_tz::Europe::Stockholm;
const TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S %Z";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Level {
    Debug = 10,
    Info = 20,
    Warning = 30,
    Error = 40,
}

impl Level {
    /// Python's level names; anything else reads as INFO.
    pub fn parse(name: &str) -> Self {
        match name.trim().to_uppercase().as_str() {
            "DEBUG" => Self::Debug,
            "WARNING" | "WARN" => Self::Warning,
            "ERROR" | "CRITICAL" => Self::Error,
            _ => Self::Info,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warning => "WARNING",
            Self::Error => "ERROR",
        }
    }
}

static LEVEL: AtomicU8 = AtomicU8::new(Level::Info as u8);
static TIMEZONE: RwLock<Tz> = RwLock::new(DEFAULT_TZ);

pub fn configure(level: &str, timezone: &str) {
    LEVEL.store(Level::parse(level) as u8, Ordering::Relaxed);
    set_timezone(timezone);
}

/// Follows the display timezone setting; an unknown zone keeps the current one.
pub fn set_timezone(name: &str) {
    if let Ok(tz) = name.parse::<Tz>() {
        *TIMEZONE.write().unwrap_or_else(|p| p.into_inner()) = tz;
    }
}

fn timezone() -> Tz {
    *TIMEZONE.read().unwrap_or_else(|p| p.into_inner())
}

pub fn enabled(level: Level) -> bool {
    level as u8 >= LEVEL.load(Ordering::Relaxed)
}

/// An instant as log lines show their own time, for times inside a message:
/// a raw UTC time next to a local-time prefix reads as the wrong time.
pub fn format_time(at: DateTime<Utc>) -> String {
    at.with_timezone(&timezone()).format(TIME_FORMAT).to_string()
}

pub fn line(level: Level, component: &str, message: std::fmt::Arguments<'_>) -> String {
    format!("{}  {:<7} {:<8} {message}", format_time(Utc::now()), level.name(), component)
}

pub fn write(level: Level, component: &str, message: std::fmt::Arguments<'_>) {
    if enabled(level) {
        let _ = writeln!(std::io::stdout().lock(), "{}", line(level, component, message));
    }
}

#[macro_export]
macro_rules! log_debug {
    ($component:expr, $($arg:tt)*) => { $crate::logging::write($crate::logging::Level::Debug, $component, format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! log_info {
    ($component:expr, $($arg:tt)*) => { $crate::logging::write($crate::logging::Level::Info, $component, format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! log_warn {
    ($component:expr, $($arg:tt)*) => { $crate::logging::write($crate::logging::Level::Warning, $component, format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! log_error {
    ($component:expr, $($arg:tt)*) => { $crate::logging::write($crate::logging::Level::Error, $component, format_args!($($arg)*)) };
}

/// The health probe fires every 30 s and assets come a dozen per page load;
/// a successful one says nothing the page's own line didn't.
pub fn routine(path: &str, status: u16) -> bool {
    status < 400 && (path == "/health" || path == "/favicon.ico" || path.starts_with("/assets/"))
}

/// One line per request, as uvicorn's access log writes it. Routine
/// successes are only kept at DEBUG.
pub async fn access_log(request: Request, next: Next) -> Response {
    let peer = request.extensions().get::<ConnectInfo<SocketAddr>>().map_or_else(|| "-".to_string(), |c| c.0.to_string());
    let method = request.method().clone();
    let target = request.uri().path_and_query().map_or_else(|| request.uri().path().to_string(), |pq| pq.to_string());
    let version = request.version();
    let path = request.uri().path().to_string();
    let response = next.run(request).await;
    let status = response.status().as_u16();
    if enabled(Level::Debug) || !routine(&path, status) {
        write(Level::Info, "access", format_args!("{peer} - \"{method} {target} {version:?}\" {status}"));
    }
    response
}
