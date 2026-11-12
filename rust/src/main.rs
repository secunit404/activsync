use std::future::IntoFuture;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use activsync::auth_guard::AuthConfig;
use activsync::garmin_login::SignIn;
use activsync::hevy_sync::{RealHevyLeg, RealMatchApplier};
use activsync::poller::{self, Clients, Poller, RealClients};
use activsync::{AppState, auth_core, config, log_error, log_info, log_warn, logging, timefmt, build_app, db::{self, Db}, default_web_dir, net, schedule, seed, update_check};
use openidconnect::http;

const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
const DEV_PASSWORD: &str = "activsync-dev-password";

/// `name`, else `legacy_name` (what it was called before the rename to
/// ActivSync; empty for a name that never had one), skipping empty values.
fn env_value(name: &str, legacy_name: &str) -> Option<String> {
    [name, legacy_name].into_iter().filter(|n| !n.is_empty()).find_map(|n| std::env::var(n).ok().filter(|v| !v.is_empty()))
}

fn env_enabled(name: &str, legacy_name: &str) -> bool {
    env_value(name, legacy_name).is_some_and(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes"))
}

/// The in-app display timezone, else `TZ`, else Stockholm, as the Python app
/// picks it for its logs.
fn log_timezone(db: &Db, development: bool) -> String {
    let stored = config::load_config(&db.lock(), development).ok().map(|cfg| cfg.display_timezone());
    [stored, std::env::var("TZ").ok()]
        .into_iter()
        .flatten()
        .find(|tz| timefmt::is_valid_timezone(tz))
        .unwrap_or_else(|| "Europe/Stockholm".into())
}

fn fail(message: &str) -> ! {
    eprintln!("ActivSync (Rust) cannot start: {message}");
    std::process::exit(1);
}

/// `activsync health-check`: whether the server on `PORT` answers, for the
/// container's HEALTHCHECK, which has no curl or Python to ask with.
async fn health_check(port: u16) -> ! {
    let url = format!("http://127.0.0.1:{port}/health");
    let healthy = match net::request(http::Method::GET, &url, &[], Vec::new()) {
        Ok(request) => {
            let answer = tokio::time::timeout(Duration::from_secs(3), net::service_http()(request)).await;
            matches!(answer, Ok(Ok(response)) if response.status().as_u16() == 200)
        }
        Err(_) => false,
    };
    std::process::exit(if healthy { 0 } else { 1 });
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("PORT")
        .ok()
        .map(|p| p.parse().unwrap_or_else(|_| fail("PORT must be a number")))
        .unwrap_or(8383);
    if std::env::args().nth(1).as_deref() == Some("health-check") {
        health_check(port).await;
    }
    let mock = env_enabled("ACTIVSYNC_DEV_MOCK_DATA", "G2S_DEV_MOCK_DATA");
    let manual_only = env_enabled("ACTIVSYNC_MANUAL_ONLY", "");
    let host: IpAddr = std::env::var("ACTIVSYNC_HOST")
        .ok()
        .map(|h| h.parse().unwrap_or_else(|_| fail("ACTIVSYNC_HOST must be an IP address")))
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    let auth = AuthConfig::from_env().unwrap_or_else(|e| fail(&e));
    let db_path = db::resolve_path(env_value("ACTIVSYNC_DB_PATH", "G2S_DB_PATH"), mock, Path::exists);
    let token_dir = env_value("ACTIVSYNC_GARMIN_TOKEN_DIR", "G2S_GARMIN_TOKEN_DIR").unwrap_or_else(|| "/config/.garminconnect".into());
    let web_dir = std::env::var("ACTIVSYNC_WEB_DIR").map(PathBuf::from).unwrap_or_else(|_| default_web_dir());

    let db = Db::open(&db_path).unwrap_or_else(|e| fail(&format!("could not open {}: {e}", db_path.display())));
    let development = ["ACTIVSYNC_DEV_MOCK_DATA", "ACTIVSYNC_MANUAL_ONLY"].iter().any(|name| env_enabled(name, ""));
    logging::configure(&env_value("ACTIVSYNC_LOG_LEVEL", "").unwrap_or_else(|| "INFO".into()), &log_timezone(&db, development));
    if auth.disabled && host != IpAddr::V4(Ipv4Addr::LOCALHOST) && host != IpAddr::V6(Ipv6Addr::LOCALHOST) {
        log_error!(
            "auth",
            "Authentication is disabled (ACTIVSYNC_AUTH_DISABLED) and ActivSync listens on {host}. Anyone who can reach it has full access."
        );
    }
    if let Some(proxy) = auth.proxy.as_ref().filter(|proxy| !proxy.trusted_gateways.is_empty()) {
        let gateways: Vec<String> = proxy.trusted_gateways.iter().map(ToString::to_string).collect();
        log_warn!(
            "auth",
            "ACTIVSYNC_AUTH_PROXY_TRUST_GATEWAY is set, so {} is trusted from the default gateway ({}). Docker relays every connection to the published port from that address; port 8381 must not be reachable except from the Docker host itself (publish it as 127.0.0.1:8381:8381).",
            proxy.header,
            gateways.join(", ")
        );
    }

    if mock {
        seed_mock(&db, &auth, host);
    }

    let clock = schedule::system_clock();
    let state = AppState::new(db.clone(), mock, clock.clone(), schedule::UNDO_WINDOW_SECONDS, auth).with_development(development);
    let (state, poller) = if mock { (state, None) } else { real_accounts(state, manual_only, token_dir) };
    let scheduler = state.scheduler.clone().spawn();
    let match_scheduler = state.match_scheduler.clone().spawn();
    let update_checker = if mock || manual_only { None } else { update_check::spawn(net::service_http(), state.update.clone()) };
    let app = build_app(state, Some(&web_dir));

    let addr = SocketAddr::new(host, port);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| fail(&format!("could not listen on {addr}: {e}")));
    let mode = match (mock, manual_only) {
        (true, _) => "mock data",
        (false, true) => "real accounts, Garmin and Strava polling off",
        (false, false) => "real accounts",
    };
    log_info!("main", "ActivSync (Rust, {mode}) listening on http://{addr} using {}", db_path.display());
    let (signalled_tx, signalled_rx) = tokio::sync::oneshot::channel::<()>();
    let serve = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            let _ = signalled_tx.send(());
        })
        .into_future();
    tokio::select! {
        result = serve => result.unwrap_or_else(|e| fail(&format!("server stopped: {e}"))),
        _ = async {
            if signalled_rx.await.is_ok() {
                tokio::time::sleep(SHUTDOWN_GRACE).await;
            } else {
                std::future::pending::<()>().await;
            }
        } => {}
    }
    scheduler.abort();
    match_scheduler.abort();
    for task in [poller, update_checker].into_iter().flatten() {
        task.abort();
    }
}

/// Resolves on SIGINT or SIGTERM, the signal `docker stop` sends.
async fn shutdown_signal() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = interrupt => {}
        _ = terminate => {}
    }
}

/// Sample data for the mock accounts, and the dev password on a server only
/// this machine can reach.
fn seed_mock(db: &Db, auth: &AuthConfig, host: IpAddr) {
    let onboarded = env_enabled("ACTIVSYNC_DEV_E2E_ONBOARDED", "");
    match seed::seed(&mut db.lock(), chrono::Utc::now(), onboarded) {
        Ok(true) => {}
        Ok(false) => fail("that database holds real activities; mock mode will not touch it"),
        Err(e) => fail(&format!("could not seed sample data: {e}")),
    }
    let conn = db.lock();
    let now = chrono::Utc::now().timestamp() as f64;
    let _ = auth_core::prune_expired_sessions(&conn, now);
    let claimed = auth_core::owner_claimed(&conn).unwrap_or(true);
    // The dev password is public, so it is only seeded for a server no one
    // else can reach; on a network address the setup-code claim applies.
    if env_enabled("ACTIVSYNC_DEV_SEED_OWNER", "") && !claimed && host.is_loopback() {
        let hash = auth_core::hash_password(DEV_PASSWORD, auth.cost).unwrap_or_else(|e| fail(&e));
        auth_core::set_password_hash(&conn, &hash, now).unwrap_or_else(|e| fail(&e.to_string()));
        log_info!("main", "Dev sign-in password: {DEV_PASSWORD}");
    }
}

/// Garmin, Strava and Hevy for real, and the background poller.
/// `ACTIVSYNC_MANUAL_ONLY` keeps Garmin and Strava polling off, so only Hevy
/// is polled and matches wait for review.
fn real_accounts(state: AppState, manual_only: bool, token_dir: String) -> (AppState, Option<tokio::task::JoinHandle<()>>) {
    let clock = schedule::system_clock();
    let clients: Arc<dyn Clients> = Arc::new(RealClients {
        db: state.db.clone(),
        clock: clock.clone(),
        garmin_http: net::garmin_http(),
        strava_http: net::service_http(),
        garmin_token_dir: token_dir.clone(),
    });
    let hevy_http = net::service_http();
    let sign_in = SignIn { http: net::garmin_sign_in_http(), clock: clock.clone(), token_dir, pause: Duration::from_secs(1) };
    let applier = RealMatchApplier {
        db: state.db.clone(),
        events: state.events.clone(),
        clients: clients.clone(),
        hevy_http: hevy_http.clone(),
        development: state.development,
    };
    let options = poller::Options {
        garmin_polling: !manual_only,
        strava_polling: !manual_only,
        development: state.development,
        ..Default::default()
    };
    let hevy = RealHevyLeg { http: hevy_http.clone(), development: state.development };
    let poller = Poller::new(state.db.clone(), state.events.clone(), clients.clone(), Some(Arc::new(hevy)), options).spawn(clock);
    let state = state.with_match_applier(Arc::new(applier)).with_clients(clients).with_hevy_http(hevy_http).with_garmin_sign_in(sign_in);
    (state, Some(poller))
}
