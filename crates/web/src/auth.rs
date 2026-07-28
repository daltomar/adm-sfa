use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::{Cookie, SameSite};
use axum_extra::extract::SignedCookieJar;
use subtle::ConstantTimeEq;

use crate::state::AppState;

const SESSION_COOKIE: &str = "adm_sfa_session";

/// A client-enforced expiry only — the browser stops sending the cookie
/// once its `Max-Age` elapses, but `require_auth` never checks an issued-at
/// timestamp of its own, so a captured raw cookie value replayed directly
/// (not through a compliant browser) stays valid until the process
/// restarts and regenerates `cookie_key`. 8 hours covers a working session
/// on this LAN-only, occasional-use tool without requiring a re-login
/// mid-task; it's not a guarantee against a stolen cookie outliving it.
const SESSION_MAX_AGE: time::Duration = time::Duration::hours(8);

/// Constant-time comparison — a plain `==` on the submitted password would
/// leak timing information about how many leading bytes matched. The
/// stakes are low for a LAN-only single-shared-password app, but this is
/// free to get right.
pub fn password_matches(state: &AppState, submitted: &str) -> bool {
    let expected = state.password.as_bytes();
    let submitted = submitted.as_bytes();
    expected.len() == submitted.len() && bool::from(expected.ct_eq(submitted))
}

/// Sets the signed session cookie after a successful login. The cookie
/// value itself carries no information beyond "this was set by us" — the
/// signature is what a client can't forge; there's no per-user identity to
/// encode (CLAUDE.md: single shared password, not per-user accounts).
pub fn set_session_cookie(jar: SignedCookieJar) -> SignedCookieJar {
    let cookie = Cookie::build((SESSION_COOKIE, "authenticated"))
        .http_only(true)
        .same_site(SameSite::Strict)
        .path("/")
        .max_age(SESSION_MAX_AGE)
        .build();
    jar.add(cookie)
}

pub fn clear_session_cookie(jar: SignedCookieJar) -> SignedCookieJar {
    jar.remove(Cookie::from(SESSION_COOKIE))
}

/// Middleware guarding every route except `/login`: redirects to the login
/// page unless a validly-signed session cookie is present. `SignedCookieJar`
/// verifies the signature itself (via `AppState`'s `cookie_key`, resolved
/// through `FromRef` since `S = AppState` is already fixed by the
/// `from_fn_with_state` call site) — a tampered or unsigned cookie value
/// simply won't be found by `jar.get`.
pub async fn require_auth(jar: SignedCookieJar, request: Request, next: Next) -> Response {
    if jar.get(SESSION_COOKIE).is_some() {
        next.run(request).await
    } else {
        Redirect::to("/login").into_response()
    }
}
