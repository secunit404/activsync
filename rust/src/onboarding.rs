use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::config;
use crate::db::{self, DbResult};
use crate::mock;
use crate::view::Connections;

/// The first-run step still to do, or `None` once onboarded. The claim
/// ("account") step comes first even on an onboarded install: an upgrade that
/// introduces sign-in must be claimed before anything else is reachable.
pub fn setup_step(conn: &Connection, connections: &Connections, auth_disabled: bool) -> DbResult<Option<&'static str>> {
    if !auth_disabled && !crate::auth_core::owner_claimed(conn)? {
        return Ok(Some("account"));
    }
    if db::truthy(db::get_config_value(conn, "initial_sync_done")?.as_ref()) {
        return Ok(None);
    }
    if !connections.garmin.connected {
        return Ok(Some("garmin"));
    }
    if connections.broken.contains(&"strava") {
        return Ok(Some("strava"));
    }
    if !db::truthy(db::get_config_value(conn, "setup_hevy_done")?.as_ref()) {
        return Ok(Some("hevy"));
    }
    Ok(Some("syncing"))
}

/// Store Garmin's activity types, optionally holding every type for review.
pub fn store_garmin_categories(conn: &Connection, development: bool, hold_all: bool, now: DateTime<Utc>) -> DbResult<()> {
    let types = mock::garmin_activity_types();
    db::set_config_value(conn, "garmin_activity_types", &Value::Array(types.clone()))?;
    db::set_config_value(conn, "garmin_activity_types_fetched_at", &json!(db::iso(now)))?;
    if hold_all {
        let mut keys: Vec<&str> = types.iter().filter_map(|t| t["type_key"].as_str()).collect();
        keys.sort_unstable();
        config::edit(conn, development, |cfg| cfg.set("held_activity_types", json!(keys)))?;
    }
    Ok(())
}

pub fn finalize_garmin_connect(conn: &Connection, development: bool, now: DateTime<Utc>) -> DbResult<()> {
    db::set_config_value(conn, "garmin_credentials_verified", &json!(true))?;
    db::set_config_value(conn, "garmin_last_sync_error", &Value::Null)?;
    let first_run = !db::truthy(db::get_config_value(conn, "initial_sync_done")?.as_ref());
    store_garmin_categories(conn, development, first_run, now)
}

/// Saving different Strava app credentials invalidates the old tokens.
pub fn persist_strava_credentials(conn: &Connection, client_id: &str, client_secret: &str) -> DbResult<()> {
    let existing = db::get_config_value(conn, "strava_credentials")?.unwrap_or(Value::Null);
    let changed = existing.get("client_id").and_then(Value::as_str) != Some(client_id)
        || existing.get("client_secret").and_then(Value::as_str) != Some(client_secret);
    db::set_config_value(
        conn,
        "strava_credentials",
        &json!({"client_id": client_id, "client_secret": client_secret}),
    )?;
    if changed {
        db::set_config_value(conn, "strava_tokens", &Value::Null)?;
    }
    Ok(())
}

/// What a successful Garmin fetch records. The mock Garmin account holds
/// exactly what the local table already has, so nothing else changes.
pub fn record_garmin_sync(conn: &Connection, now: DateTime<Utc>) -> DbResult<()> {
    let at = json!(db::iso(now));
    db::set_config_value(conn, "garmin_last_sync_at", &at)?;
    db::set_config_value(conn, "garmin_last_sync_ok_at", &at)?;
    db::set_config_value(conn, "garmin_last_sync_ok", &json!(true))?;
    db::set_config_value(conn, "garmin_last_sync_error", &Value::Null)?;
    db::set_config_value(conn, "garmin_credentials_verified", &json!(true))
}

/// Reconnect catch-up. Against the mock accounts nothing new turns up, so the
/// dashboard summary is cleared, as the Python app does when it finds nothing.
pub fn run_catch_up(conn: &Connection, now: DateTime<Utc>) -> DbResult<()> {
    record_garmin_sync(conn, now)?;
    db::set_config_value(conn, "catch_up_report", &Value::Null)
}

/// Un-hold rows whose type was just switched to autosync. Backlog holds stay:
/// a new autosync category says nothing about publishing an outage backlog.
pub fn reconcile_held_activities(conn: &Connection, held_types: &[String]) -> DbResult<usize> {
    let mut flipped = 0;
    for row in db::list_activities(conn, false)? {
        if row.publish_status != "held" || row.hold_reason.as_deref() == Some("backlog") {
            continue;
        }
        if !held_types.contains(&row.activity_type) {
            db::set_publish_status(conn, row.garmin_activity_id, "pending", None)?;
            flipped += 1;
        }
    }
    Ok(flipped)
}
