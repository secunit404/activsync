use activsync::net::{self, Browser};
use openidconnect::http::Method;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Answers one request and returns its head, lowercased.
async fn one_request(listener: TcpListener) -> String {
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut seen = Vec::new();
    let mut chunk = [0u8; 4096];
    while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
        let read = socket.read(&mut chunk).await.unwrap();
        assert!(read > 0, "the request ended before its headers did");
        seen.extend_from_slice(&chunk[..read]);
    }
    socket.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await.unwrap();
    String::from_utf8_lossy(&seen).to_lowercase()
}

async fn user_agents_sent_as(browser: Browser, own: Option<&str>) -> Vec<String> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(one_request(listener));
    let headers: Vec<(&str, &str)> = own.map(|ua| ("User-Agent", ua)).into_iter().collect();
    let mut request = net::request(Method::GET, &url, &headers, Vec::new()).unwrap();
    request.extensions_mut().insert(browser);
    let response = net::garmin_sign_in_http()(request).await.unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let head = server.await.unwrap();
    head.lines().filter_map(|line| line.strip_prefix("user-agent:")).map(|value| value.trim().to_string()).collect()
}

#[tokio::test]
async fn a_request_without_a_user_agent_gets_its_browsers_own() {
    let expected = [
        (Browser::SafariIos, "iphone", "safari/"),
        (Browser::Safari, "macintosh", "safari/"),
        (Browser::Chrome120, "chrome/120", "safari/"),
        (Browser::Edge101, "edg/101", "chrome/101"),
        (Browser::Chrome, "chrome/131", "safari/"),
    ];
    for (browser, first, second) in expected {
        let sent = user_agents_sent_as(browser, None).await;
        assert_eq!(sent.len(), 1, "{browser:?} sent {sent:?}");
        assert!(sent[0].contains(first) && sent[0].contains(second), "{browser:?} sent {sent:?}");
    }
    for safari in [Browser::SafariIos, Browser::Safari] {
        let sent = user_agents_sent_as(safari, None).await;
        assert!(!sent[0].contains("chrome"), "{safari:?} sent {sent:?}");
    }
}

#[tokio::test]
async fn a_requests_own_user_agent_replaces_the_browsers() {
    let sent = user_agents_sent_as(Browser::SafariIos, Some("the route's own")).await;
    assert_eq!(sent, ["the route's own"]);
}
