use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use activsync::auth_guard::AuthConfig;
use activsync::{AppState, auth_core, build_app, db::Db, default_web_dir, schedule, seed};

const DEV_PASSWORD: &str = "activsync-dev-password";

fn env_enabled(name: &str) -> bool {
    std::env::var(name)
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

fn fail(message: &str) -> ! {
    eprintln!("ActivSync (Rust) cannot start: {message}");
    std::process::exit(1);
}

#[tokio::main]
async fn main() {
    // The real Garmin and Strava clients are not ported yet, so this server
    // must never touch a real account.
    if !env_enabled("ACTIVSYNC_DEV_MOCK_DATA") {
        fail("only mock mode is ported so far; set ACTIVSYNC_DEV_MOCK_DATA=1");
    }
    let host: IpAddr = std::env::var("ACTIVSYNC_HOST")
        .ok()
        .map(|h| h.parse().unwrap_or_else(|_| fail("ACTIVSYNC_HOST must be an IP address")))
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    let auth = AuthConfig::from_env().unwrap_or_else(|e| fail(&e));
    if auth.disabled && !host.is_loopback() {
        fail("sign-in is off (ACTIVSYNC_AUTH_DISABLED), so the server only listens on 127.0.0.1");
    }
    let port: u16 = std::env::var("PORT")
        .ok()
        .map(|p| p.parse().unwrap_or_else(|_| fail("PORT must be a number")))
        .unwrap_or(8383);
    let db_path = std::env::var("ACTIVSYNC_DB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/activsync-rust-dev.db"));
    let web_dir = std::env::var("ACTIVSYNC_WEB_DIR").map(PathBuf::from).unwrap_or_else(|_| default_web_dir());

    let db = Db::open(&db_path).unwrap_or_else(|e| fail(&format!("could not open {}: {e}", db_path.display())));
    let onboarded = !env_enabled("ACTIVSYNC_DEV_FRESH");
    match seed::seed(&mut db.lock(), chrono::Utc::now(), onboarded) {
        Ok(true) => {}
        Ok(false) => fail("that database holds real activities; mock mode will not touch it"),
        Err(e) => fail(&format!("could not seed sample data: {e}")),
    }
    {
        let conn = db.lock();
        let now = chrono::Utc::now().timestamp() as f64;
        let _ = auth_core::prune_expired_sessions(&conn, now);
        let claimed = auth_core::owner_claimed(&conn).unwrap_or(true);
        // The dev password is public, so it is only seeded for a server no one
        // else can reach; on a network address the setup-code claim applies.
        if onboarded && !auth.disabled && !claimed && host.is_loopback() {
            let hash = auth_core::hash_password(DEV_PASSWORD, auth.cost).unwrap_or_else(|e| fail(&e));
            auth_core::set_password_hash(&conn, &hash, now).unwrap_or_else(|e| fail(&e.to_string()));
            println!("Dev sign-in password: {DEV_PASSWORD}");
        }
    }

    let state = AppState::new(db, true, schedule::system_clock(), schedule::UNDO_WINDOW_SECONDS, auth);
    let scheduler = state.scheduler.clone().spawn();
    let app = build_app(state, Some(&web_dir));

    let addr = SocketAddr::new(host, port);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| fail(&format!("could not listen on {addr}: {e}")));
    println!("ActivSync (Rust, mock data) listening on http://{addr} using {}", db_path.display());
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .unwrap_or_else(|e| fail(&format!("server stopped: {e}")));
    scheduler.abort();
}
