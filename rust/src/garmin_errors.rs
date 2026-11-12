use openidconnect::HttpResponse;
use serde_json::{Value, json};

pub(crate) fn truthy(value: &Value) -> bool {
    crate::db::truthy(Some(value))
}

#[derive(Debug, Clone, PartialEq)]
pub enum GarminError {
    /// The saved session is missing or Garmin no longer accepts it.
    SessionExpired,
    /// Opening the session failed on an outage; the session may be fine and
    /// the next poll tries again.
    Unavailable,
    /// A definitive 404 on an activity call.
    ActivityGone(i64),
    /// A 404 anywhere else.
    NotFound(String),
    /// Garmin definitively refused a FIT upload.
    UploadRejected(String),
    /// Garmin 400-rejected an exercise-sets payload over a (category,
    /// subcategory) pair. The PUT is atomic, so the whole payload failed.
    SubcategoryRejected(String),
    /// Any other answer of 400 or above, worded as the library words it.
    Api(String),
    Network(String),
    Malformed(String),
    Invalid(String),
}

impl std::fmt::Display for GarminError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionExpired => f.write_str("Garmin session expired. Reconnect Garmin to resume syncing."),
            Self::Unavailable => f.write_str("Garmin could not be reached. ActivSync tries again at the next check."),
            Self::ActivityGone(id) => write!(f, "activity {id} not found"),
            Self::NotFound(msg) | Self::UploadRejected(msg) | Self::SubcategoryRejected(msg) | Self::Api(msg) => {
                f.write_str(msg)
            }
            Self::Network(err) => f.write_str(err),
            Self::Malformed(err) => write!(f, "unreadable Garmin response: {err}"),
            Self::Invalid(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for GarminError {}

impl GarminError {
    /// An outage rather than a rejection: a network failure, a 429 or a 5xx.
    /// Only a rejection may pause Garmin until the user reconnects.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Network(_) | Self::Unavailable => true,
            Self::Api(message) => status_of(message).is_some_and(|status| status == 429 || (500..600).contains(&status)),
            _ => false,
        }
    }

    pub(crate) fn is_rate_limited(&self) -> bool {
        matches!(self, Self::Api(message) if message.starts_with("Rate limit exceeded: "))
    }
}

/// The status the library words into its messages as `API Error NNN`.
pub(crate) fn status_of(message: &str) -> Option<u16> {
    let digits = message.split_once("API Error ")?.1.get(..3)?;
    digits.parse().ok()
}

/// Python's `repr()` of decoded JSON, since the library and the Python app
/// interpolate dicts and lists into their messages that way.
pub(crate) fn py_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(b) => if *b { "True" } else { "False" }.into(),
        Value::Number(n) if n.is_f64() => {
            let f = n.as_f64().unwrap_or_default();
            if f.fract() == 0.0 && f.abs() < 1e16 { format!("{f:.1}") } else { f.to_string() }
        }
        Value::Number(n) => n.to_string(),
        Value::String(s) => {
            let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
            let mut out = String::from(quote);
            for c in s.chars() {
                match c {
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if c == quote => {
                        out.push('\\');
                        out.push(c);
                    }
                    c => out.push(c),
                }
            }
            out.push(quote);
            out
        }
        Value::Array(items) => format!("[{}]", items.iter().map(py_repr).collect::<Vec<_>>().join(", ")),
        Value::Object(map) => {
            format!("{{{}}}", map.iter().map(|(k, v)| format!("{}: {}", py_repr(&json!(k)), py_repr(v))).collect::<Vec<_>>().join(", "))
        }
    }
}

pub(crate) fn py_str(value: &Value) -> String {
    value.as_str().map_or_else(|| py_repr(value), str::to_string)
}

/// The library's error text: the status, then whatever message the body
/// offers. The upload and subcategory checks read this text.
pub(crate) fn api_error(response: &HttpResponse) -> GarminError {
    let status = response.status().as_u16();
    let mut message = format!("API Error {status}");
    let detail = serde_json::from_slice::<Value>(response.body()).ok().filter(Value::is_object).and_then(|data| {
        let first = |v: &Value| v.as_array().and_then(|a| a.first()).cloned();
        let nested = || {
            let failures = data.get("detailedImportResult").cloned().unwrap_or(json!({})).get("failures").cloned().unwrap_or(json!([{}]));
            let failure = first(&failures)?;
            first(&failure.get("messages").cloned().unwrap_or(json!([""])))
        };
        [data.get("message").cloned(), data.get("content").cloned()]
            .into_iter()
            .flatten()
            .find(truthy)
            .or_else(nested)
            .filter(truthy)
    });
    if let Some(detail) = detail {
        message.push_str(" - ");
        message.push_str(&py_str(&detail));
    }
    if status == 404 { GarminError::NotFound(message) } else { GarminError::Api(message) }
}
