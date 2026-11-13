use std::collections::HashMap;

use activsync::net::extra_roots;

const ROOT: &str = include_str!("fixtures/test_root.pem");

fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    let map: HashMap<_, _> = vars.iter().copied().collect();
    move |name| map.get(name).map(|v| v.to_string())
}

#[test]
fn requests_variables_are_read_in_the_order_requests_reads_them() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.pem");
    let second = dir.path().join("second.pem");
    std::fs::write(&first, format!("{ROOT}{ROOT}")).unwrap();
    std::fs::write(&second, ROOT).unwrap();
    let (first, second) = (first.to_str().unwrap(), second.to_str().unwrap());
    let vars = ["REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE"];

    assert_eq!(
        extra_roots(
            &vars,
            env(&[("REQUESTS_CA_BUNDLE", first), ("CURL_CA_BUNDLE", second)])
        )
        .len(),
        2
    );
    assert_eq!(
        extra_roots(&vars, env(&[("CURL_CA_BUNDLE", second)])).len(),
        1
    );
    assert_eq!(
        extra_roots(
            &vars,
            env(&[("REQUESTS_CA_BUNDLE", ""), ("CURL_CA_BUNDLE", second)])
        )
        .len(),
        1
    );
    assert!(extra_roots(&vars, env(&[])).is_empty());
}

#[test]
fn ssl_cert_file_wins_over_ssl_cert_dir_and_a_directory_is_read_whole() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("one.pem"), ROOT).unwrap();
    std::fs::write(dir.path().join("two.crt"), ROOT).unwrap();
    std::fs::write(dir.path().join("notes.txt"), "not a certificate").unwrap();
    let file = dir.path().join("one.pem");
    let (file, dir) = (file.to_str().unwrap(), dir.path().to_str().unwrap());
    let vars = ["SSL_CERT_FILE", "SSL_CERT_DIR"];

    assert_eq!(
        extra_roots(
            &vars,
            env(&[("SSL_CERT_FILE", file), ("SSL_CERT_DIR", dir)])
        )
        .len(),
        1
    );
    assert_eq!(extra_roots(&vars, env(&[("SSL_CERT_DIR", dir)])).len(), 2);
    assert!(extra_roots(&vars, env(&[("SSL_CERT_FILE", "/nonexistent/bundle.pem")])).is_empty());
}
