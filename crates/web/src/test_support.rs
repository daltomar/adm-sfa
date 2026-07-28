//! Shared test helpers — building a real `AppState` (temp data dir, a
//! fully-migrated DB via `adm_sfa_core::db::open_db`, same as `main`'s own
//! startup) and driving requests through the real `build_app` router via
//! `tower::ServiceExt::oneshot`, so route tests exercise the exact same
//! middleware stack and handler wiring the running server does.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use axum_extra::extract::cookie::Key;
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::state::AppState;

pub const TEST_PASSWORD: &str = "test-password";

/// Builds a fresh temp data dir + fully-migrated DB (same `open_db` path
/// `main` itself uses) and the `AppState` on top of it. Returns the temp
/// dir alongside so callers clean it up at the end of the test with
/// `std::fs::remove_dir_all(&data_dir).ok();` — matching the manual-cleanup
/// convention every other temp-dir test in this workspace already uses
/// (`core`'s `docs_fs.rs`/`service.rs` tests), rather than introducing a
/// `Drop` impl, which can't coexist with moving `state` back out by value.
/// Each test gets a distinct path via `tag` plus an atomic counter, since
/// tests can run in parallel.
static TEST_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn test_app(tag: &str) -> (AppState, PathBuf) {
    let unique = TEST_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let data_dir = std::env::temp_dir().join(format!(
        "adm-sfa-web-test-{tag}-{}-{unique}",
        std::process::id()
    ));
    adm_sfa_core::config::ensure_dirs(&data_dir);
    let db = adm_sfa_core::db::open_db(&data_dir).expect("failed to open test db");
    let state = AppState {
        db: Arc::new(Mutex::new(db)),
        documents_dir: data_dir.join("documents"),
        password: TEST_PASSWORD.to_string(),
        cookie_key: Key::generate(),
    };
    (state, data_dir)
}

/// Sends a single request through the given router (built by
/// `crate::build_app`) and returns the response, mirroring what a real
/// client would see — status, headers, body all intact.
pub async fn send(app: axum::Router, req: Request<Body>) -> Response<Body> {
    app.oneshot(req).await.expect("router is infallible")
}

pub async fn body_text(res: Response<Body>) -> String {
    let bytes = res
        .into_body()
        .collect()
        .await
        .expect("failed to read body")
        .to_bytes();
    String::from_utf8(bytes.to_vec()).expect("body was not valid utf-8")
}

/// Logs in with `TEST_PASSWORD` and returns the raw `Cookie` header value
/// (`name=value`) to attach to subsequent requests — exercises the real
/// `/login` handler rather than constructing a session cookie by hand, so
/// tests that build on this also incidentally cover a correct login.
pub async fn login(app: &axum::Router) -> String {
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("password={TEST_PASSWORD}")))
        .unwrap();
    let res = send(app.clone(), req).await;
    assert_eq!(
        res.status(),
        StatusCode::SEE_OTHER,
        "login with the correct test password should redirect"
    );
    let set_cookie = res
        .headers()
        .get("set-cookie")
        .expect("login response had no Set-Cookie header")
        .to_str()
        .unwrap();
    // "adm_sfa_session=<value>; HttpOnly; SameSite=Strict; ..." — the
    // `Cookie:` header on a later request only ever wants the first segment.
    set_cookie
        .split(';')
        .next()
        .expect("Set-Cookie header was empty")
        .to_string()
}

/// Builds a `multipart/form-data` body by hand (no client library — this is
/// a dev-only dependency-avoidance choice, not a claim it's the ergonomic
/// way to do this): one text field (`label`) and one file field (`file`),
/// matching what a browser's `<form enctype="multipart/form-data">` sends.
pub fn multipart_request(uri: &str, cookie: &str, label: &str, file_bytes: &[u8]) -> Request<Body> {
    const BOUNDARY: &str = "adm-sfa-test-boundary";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"label\"\r\n\r\n{label}\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(
        format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.jpg\"\r\n\
             Content-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(file_bytes);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());

    let mut builder = Request::builder().method("POST").uri(uri).header(
        "content-type",
        format!("multipart/form-data; boundary={BOUNDARY}"),
    );
    if !cookie.is_empty() {
        builder = builder.header("cookie", cookie);
    }
    builder.body(Body::from(body)).unwrap()
}
