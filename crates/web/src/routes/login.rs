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

async fn show() -> impl IntoResponse {
    HtmlTemplate(LoginTemplate { error: None })
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
        HtmlTemplate(LoginTemplate {
            error: Some("Incorrect password.".to_string()),
        })
        .into_response()
    }
}

async fn logout(jar: SignedCookieJar) -> impl IntoResponse {
    let jar = clear_session_cookie(jar);
    (jar, Redirect::to("/login"))
}
