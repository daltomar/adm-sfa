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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use crate::test_support::{self, TEST_PASSWORD};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use axum_extra::extract::cookie::Key;

    #[test]
    fn password_matches_accepts_the_correct_password() {
        let (state, dir) = test_support::test_app("password-matches-ok");
        assert!(password_matches(&state, TEST_PASSWORD));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn password_matches_rejects_a_wrong_password() {
        let (state, dir) = test_support::test_app("password-matches-wrong");
        assert!(!password_matches(&state, "not the password"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn password_matches_rejects_a_password_of_different_length() {
        // The length check has to run before `ct_eq` (which panics/misbehaves
        // on mismatched lengths in some constant-time comparison libraries) —
        // this exercises that branch specifically, not just "any wrong value".
        let (state, dir) = test_support::test_app("password-matches-length");
        assert!(!password_matches(&state, "short"));
        assert!(!password_matches(
            &state,
            &format!("{TEST_PASSWORD}-with-extra-suffix")
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn set_session_cookie_has_the_expected_security_attributes() {
        let jar = SignedCookieJar::new(Key::generate());
        let jar = set_session_cookie(jar);
        let cookie = jar
            .get(SESSION_COOKIE)
            .expect("set_session_cookie should have added the session cookie");

        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(cookie.same_site(), Some(SameSite::Strict));
        assert_eq!(cookie.path(), Some("/"));
        assert_eq!(cookie.max_age(), Some(SESSION_MAX_AGE));
    }

    #[test]
    fn clear_session_cookie_removes_a_previously_set_cookie() {
        let jar = SignedCookieJar::new(Key::generate());
        let jar = set_session_cookie(jar);
        assert!(jar.get(SESSION_COOKIE).is_some());

        let jar = clear_session_cookie(jar);
        assert!(jar.get(SESSION_COOKIE).is_none());
    }

    fn protected_router(state: AppState) -> Router {
        Router::new()
            .route("/protected", get(|| async { "ok" }))
            .route_layer(axum::middleware::from_fn_with_state(
                state.clone(),
                require_auth,
            ))
            .with_state(state)
    }

    #[tokio::test]
    async fn require_auth_redirects_to_login_when_no_cookie_is_present() {
        let (state, dir) = test_support::test_app("require-auth-no-cookie");
        let app = protected_router(state);
        let req = Request::builder()
            .uri("/protected")
            .body(Body::empty())
            .unwrap();

        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get("location").unwrap(), "/login");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn require_auth_redirects_when_the_cookie_is_unsigned_garbage() {
        let (state, dir) = test_support::test_app("require-auth-garbage-cookie");
        let app = protected_router(state);
        let req = Request::builder()
            .uri("/protected")
            .header("cookie", "adm_sfa_session=not-a-valid-signature")
            .body(Body::empty())
            .unwrap();

        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get("location").unwrap(), "/login");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn require_auth_passes_through_with_a_validly_signed_cookie() {
        let (state, dir) = test_support::test_app("require-auth-valid-cookie");
        let key = state.cookie_key.clone();
        let app = protected_router(state);

        // Round-trip through a real `Set-Cookie` response header rather than
        // reading the jar's plaintext value directly — `SignedCookieJar::get`
        // returns the *verified* value, not the signed wire format a real
        // `Cookie:` request header needs, so this is the only way to get a
        // signature `require_auth` will actually accept.
        let jar = SignedCookieJar::new(key);
        let jar = set_session_cookie(jar);
        let set_cookie_response = (jar, StatusCode::OK).into_response();
        let set_cookie = set_cookie_response
            .headers()
            .get("set-cookie")
            .expect("set_session_cookie should have produced a Set-Cookie header")
            .to_str()
            .unwrap();
        let cookie_header = set_cookie
            .split(';')
            .next()
            .expect("Set-Cookie header was empty")
            .to_string();

        let req = Request::builder()
            .uri("/protected")
            .header("cookie", cookie_header)
            .body(Body::empty())
            .unwrap();

        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(test_support::body_text(res).await, "ok");
        std::fs::remove_dir_all(&dir).ok();
    }
}
