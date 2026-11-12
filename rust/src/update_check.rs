//! Checks GitHub Releases for a newer ActivSync. Pages only read the last
//! answer; a failed check keeps the last-known one, so GitHub being down
//! never shows on a page.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use openidconnect::http;
use serde_json::Value;

use crate::net::{self, Http};
use crate::version;

pub const REPO_URL: &str = "https://github.com/secunit404/activsync";
const RELEASES_API: &str = "https://api.github.com/repos/secunit404/activsync/releases/latest";
const CHECK_INTERVAL: Duration = Duration::from_secs(86400);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq)]
pub struct UpdateStatus {
    pub latest: String,
    pub available: bool,
}

/// The last answer GitHub gave, shared by every request.
#[derive(Clone, Default)]
pub struct UpdateCache(Arc<Mutex<Option<UpdateStatus>>>);

impl UpdateCache {
    pub fn get(&self) -> Option<UpdateStatus> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn set(&self, status: UpdateStatus) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(status);
    }
}

/// A tag like `v1.2.3` as `[1, 2, 3]`, stopping at the first part that does
/// not start with a digit; nothing usable gives an empty list.
fn parse_version(text: &str) -> Vec<u64> {
    text.trim()
        .trim_start_matches(['v', 'V'])
        .split('.')
        .map_while(|chunk| {
            let digits = chunk
                .find(|c: char| !c.is_ascii_digit())
                .map_or(chunk, |end| &chunk[..end]);
            digits.parse().ok()
        })
        .collect()
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    let (latest, current) = (parse_version(latest), parse_version(current));
    !latest.is_empty() && !current.is_empty() && latest > current
}

async fn latest_tag(http: &Http) -> Result<String, String> {
    let headers = [
        ("User-Agent", "ActivSync-update-check"),
        ("Accept", "application/vnd.github+json"),
    ];
    let request =
        net::request(http::Method::GET, RELEASES_API, &headers, Vec::new()).map_err(|e| e.0)?;
    let response = tokio::time::timeout(REQUEST_TIMEOUT, http(request))
        .await
        .map_err(|_| "timed out".to_string())?
        .map_err(|e| e.0)?;
    if !response.status().is_success() {
        return Err(format!("GitHub answered {}", response.status().as_u16()));
    }
    let data: Value = serde_json::from_slice(response.body()).map_err(|e| e.to_string())?;
    data.get("tag_name")
        .and_then(Value::as_str)
        .map(|tag| tag.trim().to_string())
        .ok_or_else(|| "no tag_name".into())
}

/// Asks GitHub once and keeps the answer; a failure keeps the last one.
pub async fn refresh(http: &Http, cache: &UpdateCache, current: &str) {
    match latest_tag(http).await {
        Ok(latest) => cache.set(UpdateStatus {
            available: is_newer(&latest, current),
            latest,
        }),
        Err(err) => crate::log_debug!("update_check", "update check failed: {err}"),
    }
}

/// Checks now and then daily. A dev build names its branch rather than a
/// version, so there is nothing to compare and it never checks.
pub fn spawn(http: Http, cache: UpdateCache) -> Option<tokio::task::JoinHandle<()>> {
    if version::display_version() != *version::VERSION {
        return None;
    }
    Some(tokio::spawn(async move {
        loop {
            refresh(&http, &cache, &version::VERSION).await;
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    }))
}
