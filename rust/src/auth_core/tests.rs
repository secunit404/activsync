use super::*;

#[derive(Debug, PartialEq)]
struct PasswordCheck {
    verified: bool,
    retry_after: i64,
}

fn check_password(
    conn: &Connection,
    password: &str,
    now: f64,
    device_id: Option<&str>,
    cost: HashCost,
) -> PasswordCheck {
    match reserve_check(conn, now, device_id).unwrap() {
        Ok(reserved) => {
            let verdict = verify_stored(reserved.stored.as_deref(), password, cost);
            PasswordCheck {
                verified: settle_check(conn, device_id, &reserved, &verdict).unwrap(),
                retry_after: 0,
            }
        }
        Err(wait) => PasswordCheck {
            verified: false,
            retry_after: wait,
        },
    }
}

fn verify_password(conn: &Connection, password: &str, cost: HashCost) -> bool {
    let verdict = verify_stored(stored_hash(conn).unwrap().as_deref(), password, cost);
    if verdict.verified {
        store_rehash(conn, &verdict).unwrap();
    }
    verdict.verified
}

#[test]
fn password_policy_bounds_length_without_composition_rules() {
    assert!(password_problem(&"x".repeat(11)).is_some());
    assert!(password_problem(&"x".repeat(12)).is_none());
    assert!(password_problem(&"x".repeat(256)).is_none());
    assert!(password_problem(&"x".repeat(257)).is_some());
}

#[test]
fn verifies_hashes_made_by_the_python_app() {
    let db = crate::db::Db::open_in_memory().unwrap();
    let conn = db.lock();
    let python_standard = "$argon2id$v=19$m=65536,t=3,p=4$OHAqpN2S8c4i1Af7aUvD/w$CojUzSnycSLE8is18+Fo5CpR9JdSF+zfrpeS/q4zM9g";
    set_password_hash(&conn, python_standard, 0.0).unwrap();
    assert!(verify_password(
        &conn,
        "correct horse battery staple",
        HashCost::standard()
    ));
    assert!(!verify_password(
        &conn,
        "wrong horse battery staple",
        HashCost::standard()
    ));
    assert_eq!(
        owner(&conn).unwrap()["password_hash"],
        python_standard,
        "same cost: no rehash"
    );

    let python_cheapest = "$argon2id$v=19$m=8,t=1,p=1$RVVu42Um648$/g/+Hw";
    set_password_hash(&conn, python_cheapest, 0.0).unwrap();
    assert!(verify_password(
        &conn,
        "correct horse battery staple",
        HashCost::cheapest()
    ));
    assert_eq!(owner(&conn).unwrap()["password_hash"], python_cheapest);
    assert!(verify_password(
        &conn,
        "correct horse battery staple",
        HashCost::standard()
    ));
    let rehashed = owner(&conn).unwrap()["password_hash"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        rehashed.starts_with("$argon2id$v=19$m=65536,t=3,p=4$"),
        "{rehashed}"
    );
}

#[test]
fn throttle_backs_off_after_five_failures() {
    let db = crate::db::Db::open_in_memory().unwrap();
    let conn = db.lock();
    set_password_hash(
        &conn,
        &hash_password("correct horse battery staple", HashCost::cheapest()).unwrap(),
        0.0,
    )
    .unwrap();
    for _ in 0..4 {
        assert_eq!(
            check_password(&conn, "nope", 100.0, None, HashCost::cheapest()).retry_after,
            0
        );
    }
    assert_eq!(
        check_password(&conn, "nope", 100.0, None, HashCost::cheapest()).retry_after,
        0
    );
    let blocked = check_password(
        &conn,
        "correct horse battery staple",
        100.5,
        None,
        HashCost::cheapest(),
    );
    assert_eq!(
        blocked,
        PasswordCheck {
            verified: false,
            retry_after: 1
        }
    );
    let ok = check_password(
        &conn,
        "correct horse battery staple",
        102.0,
        None,
        HashCost::cheapest(),
    );
    assert!(ok.verified);
    assert_eq!(
        db::get_config_value(&conn, THROTTLE_KEY).unwrap().unwrap()["failures"],
        0
    );
}

#[test]
fn trusted_device_is_throttled_on_its_own() {
    let db = crate::db::Db::open_in_memory().unwrap();
    let conn = db.lock();
    set_password_hash(
        &conn,
        &hash_password("correct horse battery staple", HashCost::cheapest()).unwrap(),
        0.0,
    )
    .unwrap();
    let device = trust_device(&conn, 0.0, None).unwrap();
    for _ in 0..8 {
        check_password(&conn, "nope", 10.0, None, HashCost::cheapest());
    }
    assert!(check_password(&conn, "nope", 10.0, None, HashCost::cheapest()).retry_after > 0);
    assert!(
        check_password(
            &conn,
            "correct horse battery staple",
            10.0,
            Some(&device),
            HashCost::cheapest()
        )
        .verified
    );
}

#[test]
fn limiter_allows_ten_a_minute_per_client() {
    let limiter = AttemptLimiter::default();
    for _ in 0..10 {
        assert_eq!(limiter.retry_after("1.2.3.4", 0.0), 0);
    }
    assert_eq!(limiter.retry_after("1.2.3.4", 1.0), 59);
    assert_eq!(limiter.retry_after("5.6.7.8", 1.0), 0);
    assert_eq!(limiter.retry_after("1.2.3.4", 61.0), 0);
}

#[test]
fn setup_code_ignores_case_and_dashes() {
    let code = SetupCode::default();
    let issued = code.reissue();
    assert_eq!(issued.len(), 19);
    assert!(code.check(&issued.to_lowercase().replace('-', " ")));
    assert!(!code.check("AAAA-BBBB-CCCC-DDDD"));
    code.discard();
    assert!(!code.check(&issued));
}
