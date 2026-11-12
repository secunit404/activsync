use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::db::{self, Db};
use crate::hevy_client::HevyClient;
use crate::hevy_db;
use crate::poller::HevyLegError;
use crate::timefmt::parse_timestamp;

pub const CURSOR_KEY: &str = "hevy_events_cursor";
const CURSOR_LAG_MINUTES: i64 = 5;

/// A Hevy deletion for one of these is surfaced for review, never applied:
/// the workout has landed on Garmin or is on its way.
const SYNCED_STATUSES: [&str; 7] = [
    "merged",
    "described",
    "replaced",
    "uploaded_passive",
    "linked_existing",
    "syncing",
    "needs_review",
];

fn text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// (type, workout id, timestamp): the dedupe key.
fn event_key(event: &Value) -> (&'static str, String, String) {
    if event["type"] == "deleted" {
        return (
            "deleted",
            text(event.get("id")),
            text(event.get("deleted_at")),
        );
    }
    let workout = &event["workout"];
    (
        "updated",
        text(workout.get("id")),
        text(workout.get("updated_at")),
    )
}

fn apply_updated(conn: &Connection, event: &Value) -> rusqlite::Result<()> {
    let workout = &event["workout"];
    let id = text(workout.get("id"));
    let updated_at = text(workout.get("updated_at"));
    let before = hevy_db::get_workout(conn, &id)?;
    let title = text(workout.get("title"));
    hevy_db::upsert_workout(
        conn,
        &id,
        &title,
        &text(workout.get("start_time")),
        &text(workout.get("end_time")),
        &updated_at,
        workout,
    )?;
    let after = hevy_db::get_workout(conn, &id)?;
    if before.is_none() && after.is_some() {
        crate::log_info!(
            "hevy_sync",
            "hevy workout {id} ({title}) received, ended {}",
            text(workout.get("end_time"))
        );
    }
    // A Hevy edit can remove the exercise that needed a mapping. The equality
    // wakes only the current revision and keeps a replay after a crash safe.
    if before.is_some_and(|b| b.status == "needs_mapping")
        && after.is_some_and(|a| a.source_updated_at.as_deref() == Some(&updated_at))
    {
        hevy_db::wake_workout(conn, &id)?;
    }
    Ok(())
}

fn apply_deleted(conn: &Connection, event: &Value) -> rusqlite::Result<()> {
    let Some(row) = hevy_db::get_workout(conn, &text(event.get("id")))? else {
        return Ok(());
    };
    if SYNCED_STATUSES.contains(&row.status.as_str()) {
        hevy_db::set_workout_status(conn, &row.hevy_id, "needs_review", Some("deleted in Hevy"))
    } else {
        hevy_db::delete_workout(conn, &row.hevy_id)
    }
}

/// Reads Hevy's events feed and applies the new ones; returns how many. A
/// missing cursor starts at `now`: history is never replayed here, backfill
/// is its own action.
pub(super) async fn ingest_events(
    db: &Db,
    hevy: &HevyClient,
    now: DateTime<Utc>,
) -> Result<usize, HevyLegError> {
    let cursor = db::get_config_value(&db.lock(), CURSOR_KEY)?.filter(|c| db::truthy(Some(c)));
    let Some(cursor) = cursor else {
        db::set_config_value(&db.lock(), CURSOR_KEY, &json!(db::iso(now)))?;
        return Ok(0);
    };
    let cursor = text(Some(&cursor));
    let since = parse_timestamp(&cursor).map_or_else(
        || cursor.clone(),
        |at| db::iso(at - Duration::minutes(CURSOR_LAG_MINUTES)),
    );
    let mut events = hevy.iter_events_since(&since).await?;
    // Oldest first, a deletion last at equal timestamps: an old update must
    // never resurrect a newer deletion, whatever order the page came in.
    events.sort_by_cached_key(|event| {
        let (kind, _, ts) = event_key(event);
        (ts, kind == "deleted")
    });

    let conn = db.lock();
    let mut processed = cursor.clone();
    let mut fresh = 0;
    let mut outcome = Ok(());
    for event in &events {
        let (kind, workout_id, ts) = event_key(event);
        let seen = match hevy_db::event_seen(&conn, kind, &workout_id, &ts) {
            Ok(seen) => seen,
            Err(err) => {
                outcome = Err(err);
                break;
            }
        };
        if !seen {
            // The effect before the dedupe record: a crash between them
            // re-runs the idempotent effect rather than losing it.
            let applied = if kind == "updated" {
                apply_updated(&conn, event)
            } else {
                apply_deleted(&conn, event)
            }
            .and_then(|()| hevy_db::record_event_seen(&conn, kind, &workout_id, &ts));
            if let Err(err) = applied {
                outcome = Err(err);
                break;
            }
            fresh += 1;
        }
        // A seen event still moves the cursor, or it would lag until an
        // unrelated newer event arrived.
        if ts > processed {
            processed = ts;
        }
    }
    if processed != cursor {
        db::set_config_value(&conn, CURSOR_KEY, &json!(processed))?;
    }
    outcome?;
    Ok(fresh)
}
