use activsync::auth_guard::AuthConfig;
use activsync::db::Db;
use activsync::{AppState, build_app, schedule};
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn the_browser_tab_and_home_screen_icons_need_no_sign_in() {
    let auth = AuthConfig { disabled: false, ..AuthConfig::sign_in_off() };
    let state = AppState::new(Db::open_in_memory().unwrap(), false, schedule::system_clock(), 0.0, auth);
    let app = build_app(state, None);
    for (path, file) in [
        ("/favicon.ico", &include_bytes!("../../src/activsync/static/favicon.png")[..]),
        ("/apple-touch-icon.png", &include_bytes!("../../src/activsync/static/apple-touch-icon.png")[..]),
    ] {
        let request = Request::builder().uri(path).header(header::HOST, "127.0.0.1:8381").body(Body::empty()).unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(&response.into_body().collect().await.unwrap().to_bytes()[..], file, "{path}");
    }
}
