use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Redirect};
use axum::routing::get;
use axum::Form;
use axum::Router;
use serde::Deserialize;

use adm_sfa_core::db::queries::donors as donors_qry;
use adm_sfa_core::model::donor::DonorDraft;

use crate::state::AppState;
use crate::templates::{DonorFormTemplate, DonorRow, DonorsListTemplate, HtmlTemplate};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/donors", get(list).post(create))
        .route("/donors/new", get(new_form))
        .route("/donors/{id}/edit", get(edit_form).post(update))
}

async fn list(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let donors = donors_qry::list(&conn).unwrap_or_default();
    let rows = donors
        .into_iter()
        .map(|d| DonorRow {
            id: d.id,
            name: d.name,
            contact_info: d.contact_info.unwrap_or_default(),
        })
        .collect();
    HtmlTemplate(DonorsListTemplate {
        donors: rows,
        locale,
    })
}

/// A same-origin, root-relative path — rejects absolute URLs (`https://...`)
/// and protocol-relative ones (`//evil.example`) so a crafted `return_to`
/// can't be used as an open redirect. `create()` is the only place this
/// value is ever used in a `Redirect::to`.
///
/// A bare `starts_with('/') && !starts_with("//")` isn't enough on its own:
/// WHATWG URL parsing (what a real browser applies to a `Location` header)
/// normalizes backslashes to forward slashes and strips ASCII tab/CR/LF
/// *before* resolving a relative reference, so e.g. `/\evil.example` or
/// `/\t/evil.example` both pass that check yet still resolve to an external
/// origin. Reject any backslash or control character outright rather than
/// trying to enumerate every such normalization individually.
fn safe_return_to(s: &str) -> bool {
    s.starts_with('/') && !s.starts_with("//") && !s.chars().any(|c| c == '\\' || c.is_control())
}

#[derive(Deserialize)]
struct NewDonorQuery {
    #[serde(default)]
    return_to: Option<String>,
}

async fn new_form(
    State(state): State<AppState>,
    Query(query): Query<NewDonorQuery>,
) -> impl IntoResponse {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    HtmlTemplate(DonorFormTemplate {
        id: None,
        draft: DonorDraft::default(),
        error: None,
        return_to: query.return_to.filter(|s| safe_return_to(s)),
        locale,
    })
}

async fn edit_form(State(state): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let Some(donor) = donors_qry::get(&conn, id).ok().flatten() else {
        return (axum::http::StatusCode::NOT_FOUND, "donor not found").into_response();
    };
    let draft = DonorDraft {
        name: donor.name,
        contact_info: donor.contact_info.unwrap_or_default(),
        notes: donor.notes.unwrap_or_default(),
    };
    HtmlTemplate(DonorFormTemplate {
        id: Some(id),
        draft,
        error: None,
        return_to: None,
        locale,
    })
    .into_response()
}

#[derive(Deserialize)]
struct DonorForm {
    name: String,
    #[serde(default)]
    contact_info: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    return_to: String,
}

async fn create(State(state): State<AppState>, Form(form): Form<DonorForm>) -> impl IntoResponse {
    let return_to = form.return_to;
    let draft = DonorDraft {
        name: form.name,
        contact_info: form.contact_info,
        notes: form.notes,
    };
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    match donors_qry::insert(&conn, &draft) {
        Ok(id) => {
            if safe_return_to(&return_to) {
                // Assumes return_to carries no #fragment (none of today's
                // callers emit one) — appending a query after a fragment
                // would produce a syntactically-wrong-order URL.
                let sep = if return_to.contains('?') { '&' } else { '?' };
                Redirect::to(&format!("{return_to}{sep}donor_id={id}")).into_response()
            } else {
                Redirect::to(&format!("/donors/{id}/edit")).into_response()
            }
        }
        Err(e) => HtmlTemplate(DonorFormTemplate {
            id: None,
            draft,
            error: Some(e.to_string()),
            return_to: Some(return_to).filter(|s| safe_return_to(s)),
            locale,
        })
        .into_response(),
    }
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<DonorForm>,
) -> impl IntoResponse {
    let draft = DonorDraft {
        name: form.name,
        contact_info: form.contact_info,
        notes: form.notes,
    };
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    match donors_qry::update(&conn, id, &draft) {
        Ok(()) => Redirect::to(&format!("/donors/{id}/edit")).into_response(),
        Err(e) => HtmlTemplate(DonorFormTemplate {
            id: Some(id),
            draft,
            error: Some(e.to_string()),
            return_to: None,
            locale,
        })
        .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};

    /// The "Create new donor" flow: creating a donor with a `return_to`
    /// carried over from the linking page should redirect there (with the
    /// new donor's id appended) instead of always landing on this donor's
    /// own edit page.
    #[tokio::test]
    async fn create_with_a_return_to_redirects_there_with_donor_id_appended() {
        let (state, dir) = test_support::test_app("donors-return-to");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body =
            "name=Alex&contact_info=&notes=&return_to=%2Feur-ledger%2Fnew%3Fdate%3D2026-01-01";
        let req = Request::builder()
            .method("POST")
            .uri("/donors")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let location = res.headers().get("location").unwrap().to_str().unwrap();
        assert_eq!(location, "/eur-ledger/new?date=2026-01-01&donor_id=1");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A `return_to` that isn't a root-relative path (an open-redirect
    /// attempt) is rejected — falls back to the normal edit-page redirect
    /// instead of ever being handed to `Redirect::to`.
    #[tokio::test]
    async fn create_ignores_an_unsafe_return_to() {
        let (state, dir) = test_support::test_app("donors-unsafe-return-to");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body = "name=Alex&contact_info=&notes=&return_to=https%3A%2F%2Fevil.example";
        let req = Request::builder()
            .method("POST")
            .uri("/donors")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let location = res.headers().get("location").unwrap().to_str().unwrap();
        assert_eq!(location, "/donors/1/edit");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Same guard, for a protocol-relative URL (`//evil.example`), which a
    /// naive `starts_with('/')`-only check would have let through.
    #[tokio::test]
    async fn create_ignores_a_protocol_relative_return_to() {
        let (state, dir) = test_support::test_app("donors-protocol-relative-return-to");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body = "name=Alex&contact_info=&notes=&return_to=%2F%2Fevil.example";
        let req = Request::builder()
            .method("POST")
            .uri("/donors")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let location = res.headers().get("location").unwrap().to_str().unwrap();
        assert_eq!(location, "/donors/1/edit");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Regression coverage for a real bypass a reviewer caught: a bare
    /// `starts_with('/') && !starts_with("//")` check lets through values
    /// that WHATWG URL parsing (what a real browser applies to a `Location`
    /// header) still resolves to an external origin, because it normalizes
    /// backslashes to forward slashes and strips tab/CR/LF before parsing.
    #[test]
    fn safe_return_to_rejects_backslash_and_control_character_bypasses() {
        assert!(!super::safe_return_to("/\\evil.example"));
        assert!(!super::safe_return_to("\\/evil.example"));
        assert!(!super::safe_return_to("\\\\evil.example"));
        assert!(!super::safe_return_to("/\t/evil.example"));
        assert!(!super::safe_return_to("/\r\nSet-Cookie: evil"));
    }

    #[test]
    fn safe_return_to_accepts_a_normal_relative_path_with_a_query_string() {
        assert!(super::safe_return_to(
            "/eur-ledger/new?date=2026-01-01&amount_str=10.00"
        ));
    }

    /// Same bypass as the unit tests above, exercised end-to-end through the
    /// actual HTTP route to confirm the fix holds at the boundary an
    /// attacker would actually hit, not just in the helper function.
    #[tokio::test]
    async fn create_ignores_a_backslash_return_to() {
        let (state, dir) = test_support::test_app("donors-backslash-return-to");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body = "name=Alex&contact_info=&notes=&return_to=%2F%5Cevil.example";
        let req = Request::builder()
            .method("POST")
            .uri("/donors")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let location = res.headers().get("location").unwrap().to_str().unwrap();
        assert_eq!(location, "/donors/1/edit");
        std::fs::remove_dir_all(&dir).ok();
    }
}
