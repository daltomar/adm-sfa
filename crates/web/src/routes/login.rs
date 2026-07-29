use axum::extract::State;
use axum::response::{IntoResponse, Redirect};
use axum::routing::{get, post};
use axum::{Form, Router};
use axum_extra::extract::SignedCookieJar;
use serde::Deserialize;

use crate::auth::{clear_session_cookie, password_matches, set_session_cookie};
use crate::state::AppState;
use crate::templates::{HtmlTemplate, LoginTemplate};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/login", get(show).post(submit))
        .route("/logout", post(logout))
}

async fn show(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    HtmlTemplate(LoginTemplate {
        error: None,
        locale,
    })
}

#[derive(Deserialize)]
struct LoginForm {
    password: String,
}

async fn submit(
    State(state): State<AppState>,
    jar: SignedCookieJar,
    Form(form): Form<LoginForm>,
) -> impl IntoResponse {
    if password_matches(&state, &form.password) {
        let jar = set_session_cookie(jar);
        (jar, Redirect::to("/")).into_response()
    } else {
        let conn = state.conn();
        let locale = crate::i18n::resolve_locale(&conn);
        let error =
            Some(rust_i18n::t!("login.error.incorrect_password", locale = &locale).to_string());
        HtmlTemplate(LoginTemplate { error, locale }).into_response()
    }
}

async fn logout(jar: SignedCookieJar) -> impl IntoResponse {
    let jar = clear_session_cookie(jar);
    (jar, Redirect::to("/login"))
}

#[cfg(test)]
mod tests {
    use crate::test_support::{self, TEST_PASSWORD};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};

    #[tokio::test]
    async fn correct_password_redirects_and_sets_a_session_cookie() {
        let (state, dir) = test_support::test_app("login-correct-password");
        let app = crate::build_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/login")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(format!("password={TEST_PASSWORD}")))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert!(res.headers().get("set-cookie").is_some());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn wrong_password_rerenders_the_login_form_with_an_error() {
        let (state, dir) = test_support::test_app("login-wrong-password");
        let app = crate::build_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/login")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("password=definitely-wrong"))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        assert!(res.headers().get("set-cookie").is_none());
        let body = test_support::body_text(res).await;
        assert!(body.contains("Incorrect password"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_valid_login_actually_unlocks_a_protected_page() {
        // Exercises the full stack end to end: log in for real, then confirm
        // the resulting cookie is what `require_auth` accepts on a page
        // outside `login.rs`'s own router — not just that `/login` itself
        // returned the right status.
        let (state, dir) = test_support::test_app("login-then-protected");
        let app = crate::build_app(state);

        let cookie = test_support::login(&app).await;
        let req = Request::builder()
            .uri("/purchases")
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn get_logout_is_method_not_allowed() {
        // Regression test: /logout used to be a plain GET, trivially
        // triggerable cross-site (e.g. an <img> tag) to force-end a session.
        let (state, dir) = test_support::test_app("logout-get-not-allowed");
        let app = crate::build_app(state);

        let req = Request::builder()
            .method("GET")
            .uri("/logout")
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn post_logout_clears_the_session_so_protected_pages_redirect_again() {
        let (state, dir) = test_support::test_app("logout-post-clears-session");
        let app = crate::build_app(state);

        let cookie = test_support::login(&app).await;
        let logout_req = Request::builder()
            .method("POST")
            .uri("/logout")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let logout_res = test_support::send(app.clone(), logout_req).await;
        assert_eq!(logout_res.status(), StatusCode::SEE_OTHER);
        let cleared_cookie = logout_res
            .headers()
            .get("set-cookie")
            .expect("logout should send a Set-Cookie header clearing the session")
            .to_str()
            .unwrap()
            .to_string();

        // Use the *cleared* cookie (what a real browser would now hold), not
        // the original — this proves the clearing cookie actually overwrites
        // the session rather than merely checking the old cookie still works.
        let req = Request::builder()
            .uri("/purchases")
            .header("cookie", cleared_cookie.split(';').next().unwrap())
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get("location").unwrap(), "/login");
        std::fs::remove_dir_all(&dir).ok();
    }
}
