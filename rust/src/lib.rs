pub mod auth_core;
pub mod auth_guard;
pub mod auth_routes;
pub mod config;
pub mod db;
pub mod events;
pub mod exercise_tables;
pub mod fit_builder;
pub mod fit_encode;
pub mod garmin_client;
mod garmin_errors;
pub mod garmin_login;
mod garmin_retry;
pub mod garmin_routes;
pub mod hevy_apply;
pub mod hevy_backfill;
pub mod hevy_client;
pub mod hevy_db;
pub mod hevy_description;
pub mod hevy_match;
pub mod hevy_profile;
pub mod hevy_queue;
pub mod hevy_seed;
pub mod hevy_settings;
pub mod hevy_sync;
pub mod hevy_tools;
pub mod hr_sources;
pub mod logging;
pub mod mock;
pub mod net;
pub mod oidc_config;
pub mod oidc_flow;
pub mod oidc_logo;
pub mod oidc_routes;
pub mod onboarding;
pub mod poller;
pub mod proxy_headers;
pub mod publish_routes;
pub mod routes;
pub mod schedule;
pub mod seed;
pub mod settings_routes;
pub mod strava_client;
pub mod strava_routes;
pub mod strava_strength;
pub mod sync;
pub mod sync_routes;
pub mod timefmt;
pub mod update_check;
pub mod version;
pub mod view;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::http::{HeaderValue, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use tower_http::services::ServeDir;

use crate::auth_core::{AttemptLimiter, SetupCode};
use crate::auth_guard::AuthConfig;
use crate::db::Db;
use crate::events::EventBus;
use crate::hevy_match::HevyMatchScheduler;
use crate::schedule::PublishScheduler;

/// Garmin credentials held in memory while Garmin waits for a verification
/// code; they are only saved once the code is accepted.
#[derive(Clone, Debug)]
pub struct GarminCredentials {
    pub email: String,
    pub password: String,
}

#[derive(Clone, Debug)]
pub struct PendingMfa {
    pub credentials: GarminCredentials,
    pub login: garmin_login::PendingLogin,
}

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub events: EventBus,
    pub scheduler: PublishScheduler,
    pub match_scheduler: HevyMatchScheduler,
    pub mock_mode: bool,
    /// Mock data or `ACTIVSYNC_MANUAL_ONLY`: new installs default to reviewing
    /// Hevy matches by hand.
    pub development: bool,
    pub auth: AuthConfig,
    pub setup_code: SetupCode,
    pub limiter: AttemptLimiter,
    pub oidc_http: oidc_flow::OidcHttp,
    pub oidc_sessions: oidc_flow::OidcSessions,
    pending_mfa: Arc<Mutex<Option<PendingMfa>>>,
    /// The real Garmin and Strava; `None` serves the mock accounts.
    pub clients: Option<Arc<dyn poller::Clients>>,
    /// How the real Hevy API is reached; `None` serves the mock account.
    pub hevy_http: Option<net::Http>,
    /// Signs in to the real Garmin; `None` serves the mock account.
    pub garmin_sign_in: Option<garmin_login::SignIn>,
    pub update: update_check::UpdateCache,
}

impl AppState {
    /// Issues (and logs) a setup code when sign-in is on and nobody has
    /// claimed the install yet.
    pub fn new(
        db: Db,
        mock_mode: bool,
        clock: schedule::Clock,
        window_seconds: f64,
        auth: AuthConfig,
    ) -> Self {
        let events = EventBus::new();
        let scheduler =
            PublishScheduler::new(db.clone(), events.clone(), clock.clone(), window_seconds);
        let match_scheduler =
            HevyMatchScheduler::new(db.clone(), events.clone(), clock, window_seconds);
        let setup_code = SetupCode::default();
        if let Err(err) = setup_code.ensure(&db.lock(), auth.disabled) {
            crate::log_error!("server", "could not read the owner record: {err}");
        }
        Self {
            db,
            events,
            scheduler,
            match_scheduler,
            mock_mode,
            development: mock_mode,
            auth,
            setup_code,
            limiter: AttemptLimiter::default(),
            oidc_http: net::provider_http(),
            oidc_sessions: oidc_flow::OidcSessions::default(),
            pending_mfa: Arc::default(),
            clients: None,
            hevy_http: None,
            garmin_sign_in: None,
            update: update_check::UpdateCache::default(),
        }
    }

    pub fn with_garmin_sign_in(self, sign_in: garmin_login::SignIn) -> Self {
        Self {
            garmin_sign_in: Some(sign_in),
            ..self
        }
    }

    pub fn with_match_applier(self, applier: Arc<dyn hevy_match::MatchApplier>) -> Self {
        let match_scheduler = self.match_scheduler.clone().with_applier(applier);
        Self {
            match_scheduler,
            ..self
        }
    }

    pub fn with_development(self, development: bool) -> Self {
        Self {
            development,
            ..self
        }
    }

    /// Makes both undo schedulers read the undo delay setting at schedule time.
    pub fn with_configured_undo_window(self) -> Self {
        Self {
            scheduler: self.scheduler.with_configured_window(),
            match_scheduler: self.match_scheduler.with_configured_window(),
            ..self
        }
    }

    pub fn with_hevy_http(self, hevy_http: net::Http) -> Self {
        Self {
            hevy_http: Some(hevy_http),
            ..self
        }
    }

    /// The real Hevy client for `api_key`, or `None` for the mock account.
    pub fn real_hevy(
        &self,
        api_key: &str,
    ) -> Option<Result<hevy_client::HevyClient, &'static str>> {
        self.hevy_http
            .as_ref()
            .map(|http| hevy_client::HevyClient::new(http.clone(), api_key))
    }

    pub fn with_clients(self, clients: Arc<dyn poller::Clients>) -> Self {
        let scheduler = self.scheduler.clone().with_clients(clients.clone());
        Self {
            clients: Some(clients),
            scheduler,
            ..self
        }
    }

    /// For tests: a stand-in identity provider.
    pub fn with_oidc_http(self, oidc_http: oidc_flow::OidcHttp) -> Self {
        Self { oidc_http, ..self }
    }

    pub fn pending_mfa(&self) -> Option<PendingMfa> {
        self.pending_mfa
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn set_pending_mfa(&self, value: Option<PendingMfa>) {
        *self.pending_mfa.lock().unwrap_or_else(|p| p.into_inner()) = value;
    }
}

fn settings_router() -> Router<AppState> {
    use garmin_routes::{cancel_mfa, complete_mfa, reconnect_garmin, setup_garmin};
    use settings_routes::*;
    use strava_routes::strava_authorize;
    use sync_routes::{initial_sync, manual_sync, refresh_activity_types};
    Router::new()
        .route("/settings", get(settings_state))
        .route("/settings/garmin/reconnect", post(reconnect_garmin))
        .route("/settings/strava-credentials", put(save_strava_credentials))
        .route("/settings/strava/authorize", post(strava_authorize))
        .route("/settings/strava", delete(disconnect_strava))
        .route("/settings/preferences", put(save_preferences))
        .route("/settings/activity-types", put(save_activity_types))
        .route(
            "/settings/activity-types/refresh",
            post(refresh_activity_types),
        )
        .route("/settings/sync/{service}", post(manual_sync))
        .route("/settings/hevy/credentials", post(save_hevy_credentials))
        .route(
            "/settings/hevy",
            put(save_hevy_settings).delete(disconnect_hevy),
        )
        .route("/settings/hevy/device-options", get(hevy_device_options))
        .route("/settings/hevy/tools", get(hevy_tools::tools_state))
        .route(
            "/settings/hevy/backfill/preview",
            post(hevy_tools::preview_backfill),
        )
        .route(
            "/settings/hevy/backfill/run",
            post(hevy_tools::run_backfill),
        )
        .route(
            "/settings/hevy/mappings/{template_id}",
            put(hevy_tools::save_mapping).delete(hevy_tools::delete_mapping),
        )
        .route(
            "/settings/hevy/mappings/{template_id}/strava",
            put(hevy_tools::save_strava_mapping).delete(hevy_tools::delete_strava_mapping),
        )
        .route("/setup/garmin", post(setup_garmin))
        .route("/setup/hevy", post(setup_hevy))
        .route("/setup/hevy/skip", post(skip_hevy))
        .route("/setup/initial-sync", post(initial_sync))
        .route("/garmin/mfa", post(complete_mfa).delete(cancel_mfa))
        .route("/dev/onboarding-state", post(set_dev_onboarding_state))
}

fn auth_router() -> Router<AppState> {
    use auth_routes::*;
    Router::new()
        .route("/auth/status", get(auth_status))
        .route("/auth/claim", post(claim))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/sessions", get(list_sessions).delete(revoke_others))
        .route("/auth/sessions/{public_id}", delete(revoke_one))
        .route("/auth/password", put(change_password))
        .route("/auth/settings", get(auth_settings))
        .route("/dev/unclaim", post(dev_unclaim))
        .route(
            "/auth/oidc",
            get(oidc_routes::oidc_state)
                .put(oidc_routes::save_oidc)
                .delete(oidc_routes::delete_oidc),
        )
        .route("/auth/oidc/link", post(oidc_routes::start_link))
        .route(
            "/auth/oidc/logo",
            get(oidc_routes::logo)
                .put(oidc_routes::upload_logo)
                .delete(oidc_routes::delete_logo),
        )
}

fn hevy_router() -> Router<AppState> {
    use hevy_queue::*;
    Router::new()
        .route("/hevy/queue", get(queue_state))
        .route("/hevy/{hevy_id}", get(workout))
        .route("/hevy/{hevy_id}/match", post(choose_match))
        .route("/hevy/{hevy_id}/match/cancel", post(cancel_match))
        .route("/hevy/{hevy_id}/match/status", get(match_status))
        .route("/hevy/{hevy_id}/retry", post(retry))
        .route("/hevy/{hevy_id}/skip", post(skip))
        .route("/hevy/{hevy_id}/unskip", post(unskip))
        .route("/hevy/{hevy_id}/resync-fresh", post(resync_fresh))
}

fn api_router() -> Router<AppState> {
    use publish_routes::*;
    use routes::*;
    Router::new()
        .merge(settings_router())
        .merge(auth_router())
        .merge(hevy_router())
        .route("/app", get(app_state))
        .route("/catch-up-report", delete(dismiss_catch_up_report))
        .route("/activities", get(activities_page))
        .route("/activities/publish", post(publish_activities))
        .route(
            "/activities/publish/schedule",
            post(schedule_publish_activities),
        )
        .route(
            "/activities/publish/cancel",
            post(cancel_publish_activities),
        )
        .route("/activities/publish/batches/{batch_id}", get(publish_batch))
        .route("/activities/{id}", put(edit_activity))
        .route("/activities/{id}/exclude", post(exclude_activity))
        .route("/activities/{id}/restore", post(restore_activity))
        .route("/activities/{id}/publish", post(publish_activity))
        .route(
            "/activities/{id}/publish/schedule",
            post(schedule_publish_activity),
        )
        .route(
            "/activities/{id}/publish/cancel",
            post(cancel_publish_activity),
        )
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
}

const FAVICON: &[u8] = include_bytes!("../../src/activsync/static/favicon.png");
const APPLE_TOUCH_ICON: &[u8] = include_bytes!("../../src/activsync/static/apple-touch-icon.png");

/// With a built frontend in `web_dir`, every path outside the reserved
/// prefixes falls back to its `index.html`.
pub fn build_app(mut state: AppState, web_dir: Option<&Path>) -> Router {
    let index = web_dir.map(|d| d.join("index.html"));
    state.auth.csp = Arc::new(auth_guard::content_security_policy(index.as_deref()));
    let mut app = Router::new()
        .nest("/api/v1", api_router())
        .route("/api/events", get(routes::sse_events))
        .route("/api/{*rest}", get(routes::not_found))
        .route("/health", get(routes::health))
        .route("/favicon.ico", get(|| async { routes::icon(FAVICON) }))
        .route(
            "/apple-touch-icon.png",
            get(|| async { routes::icon(APPLE_TOUCH_ICON) }),
        )
        .route("/strava/callback", get(strava_routes::strava_callback))
        .route("/auth/oidc/login", get(oidc_routes::oidc_login))
        .route("/auth/oidc/callback", get(oidc_routes::oidc_callback));
    if let Some(dir) = web_dir {
        let assets =
            ServeDir::new(dir.join("assets")).fallback(Router::new().fallback(routes::not_found));
        app = app.nest_service(
            "/assets",
            Router::new()
                .fallback_service(assets)
                .layer(axum::middleware::map_response(text_charset)),
        );
    }
    let index = index.filter(|index| index.is_file()).map(Arc::new);
    app = app.fallback(get(move |uri: Uri| spa(index.clone(), uri)));
    app.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        auth_guard::guard,
    ))
    .layer(axum::middleware::from_fn(logging::access_log))
    .with_state(state)
}

async fn spa(index: Option<Arc<PathBuf>>, uri: Uri) -> Response {
    let first_segment = uri
        .path()
        .strip_prefix('/')
        .unwrap_or(uri.path())
        .split('/')
        .next()
        .unwrap_or_default();
    let body = match &index {
        Some(index) if !auth_guard::SPA_RESERVED_PREFIXES.contains(&first_segment) => {
            tokio::fs::read(index.as_path()).await.ok()
        }
        _ => None,
    };
    match body {
        Some(body) => (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            body,
        )
            .into_response(),
        None => routes::not_found().await.into_response(),
    }
}

/// Starlette adds the charset to every `text/*` type it serves.
async fn text_charset(mut response: Response) -> Response {
    let text = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.starts_with("text/") && !value.contains("charset"))
        .map(|value| format!("{value}; charset=utf-8"));
    if let Some(value) = text.and_then(|value| HeaderValue::from_str(&value).ok()) {
        response.headers_mut().insert(header::CONTENT_TYPE, value);
    }
    response
}
