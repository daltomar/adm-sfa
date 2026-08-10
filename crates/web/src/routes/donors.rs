use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Redirect};
use axum::routing::get;
use axum::Form;
use axum::Router;
use axum_extra::extract::SignedCookieJar;
use serde::Deserialize;

use adm_sfa_core::db::queries::donors as donors_qry;
use adm_sfa_core::model::donor::DonorDraft;

use crate::flash::{self, FlashKind};
use crate::routes::safe_return_to;
use crate::state::AppState;
use crate::templates::{DonorFormTemplate, DonorRow, DonorsListTemplate, HtmlTemplate};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/donors", get(list).post(create))
        .route("/donors/new", get(new_form))
        .route("/donors/{id}/edit", get(edit_form).post(update))
}

async fn list(State(state): State<AppState>, jar: SignedCookieJar) -> impl IntoResponse {
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
    let (jar, kind) = flash::take_flash(jar);
    let flash = flash::flash_for_template(kind, &locale);
    (
        jar,
        HtmlTemplate(DonorsListTemplate {
            donors: rows,
            flash,
            locale,
        }),
    )
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
        flash: None,
        locale,
    })
}

async fn edit_form(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    jar: SignedCookieJar,
) -> impl IntoResponse {
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
    let (jar, kind) = flash::take_flash(jar);
    let flash = flash::flash_for_template(kind, &locale);
    (
        jar,
        HtmlTemplate(DonorFormTemplate {
            id: Some(id),
            draft,
            error: None,
            return_to: None,
            flash,
            locale,
        }),
    )
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

async fn create(
    State(state): State<AppState>,
    jar: SignedCookieJar,
    Form(form): Form<DonorForm>,
) -> impl IntoResponse {
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
                // Lands back on the linking page's own in-progress form
                // (e.g. `/eur-ledger/new`), not a page this feature wired a
                // `flash` field into — no flash is set here, since an
                // unconsumed flash cookie would otherwise linger and
                // incorrectly surface on whatever flash-aware page the user
                // navigates to next. This inline "+ New donor" round trip is
                // an explicitly out-of-scope secondary action (see
                // CLAUDE.md); only the plain "Donors page → + Add donor"
                // flow below counts as this feature's donor "save event".
                //
                // Assumes return_to carries no #fragment (none of today's
                // callers emit one) — appending a query after a fragment
                // would produce a syntactically-wrong-order URL.
                let sep = if return_to.contains('?') { '&' } else { '?' };
                Redirect::to(&format!("{return_to}{sep}donor_id={id}")).into_response()
            } else {
                // No caller-supplied return path (the normal "Donors page →
                // + Add donor" flow) or an unsafe one (rejected above,
                // falls back here too) — either way lands on the Donors
                // list, matching desktop's own Save-while-adding behavior
                // (`ui/views/donors.rs`'s Save handler sets `Mode::List`,
                // never a per-donor edit view).
                let jar = flash::set_flash(jar, FlashKind::Success);
                (jar, Redirect::to("/donors")).into_response()
            }
        }
        Err(e) => HtmlTemplate(DonorFormTemplate {
            id: None,
            draft,
            error: Some(e.to_string()),
            return_to: Some(return_to).filter(|s| safe_return_to(s)),
            flash: None,
            locale,
        })
        .into_response(),
    }
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    jar: SignedCookieJar,
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
        Ok(()) => {
            let jar = flash::set_flash(jar, FlashKind::Success);
            (jar, Redirect::to(&format!("/donors/{id}/edit"))).into_response()
        }
        Err(e) => HtmlTemplate(DonorFormTemplate {
            id: Some(id),
            draft,
            error: Some(e.to_string()),
            return_to: None,
            flash: None,
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
    /// attempt) is rejected — falls back to the same "no return_to"
    /// destination (the Donors list) instead of ever being handed to
    /// `Redirect::to`.
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
        assert_eq!(location, "/donors");
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
        assert_eq!(location, "/donors");
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
        assert_eq!(location, "/donors");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The normal "Donors page → + Add donor" flow (no `return_to` at all)
    /// lands on the Donors list, not this donor's own edit page — matching
    /// desktop's Save-while-adding behavior (`ui/views/donors.rs`, which
    /// returns to `Mode::List` on success, never a per-donor edit view).
    #[tokio::test]
    async fn create_without_a_return_to_redirects_to_the_donors_list() {
        let (state, dir) = test_support::test_app("donors-no-return-to");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body = "name=Alex&contact_info=&notes=";
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
        assert_eq!(location, "/donors");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The normal "Donors page → + Add donor" flow's redirect to `/donors`
    /// sets a flash cookie, and the list page shows the translated
    /// "Saved successfully." banner when it reads that cookie back — a
    /// second load of the same page (simulating a manual refresh) doesn't
    /// re-show it, since `take_flash` clears the cookie on read.
    #[tokio::test]
    async fn create_redirects_and_sets_a_success_flash() {
        let (state, dir) = test_support::test_app("donors-create-flash");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body = "name=Alex&contact_info=&notes=";
        let req = Request::builder()
            .method("POST")
            .uri("/donors")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app.clone(), req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let flash_cookie = res
            .headers()
            .get("set-cookie")
            .expect("create should have set a flash cookie")
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();

        let combined_cookie = format!("{cookie}; {flash_cookie}");
        let req = Request::builder()
            .method("GET")
            .uri("/donors")
            .header("cookie", &combined_cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app.clone(), req).await;
        assert_eq!(res.status(), StatusCode::OK);
        let body_text = test_support::body_text(res).await;
        assert!(body_text.contains("Saved successfully."));

        // A second load without resending the (now-cleared) flash cookie —
        // simulating a manual refresh — must not show the banner again.
        let req = Request::builder()
            .method("GET")
            .uri("/donors")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;
        assert_eq!(res.status(), StatusCode::OK);
        let body_text = test_support::body_text(res).await;
        assert!(!body_text.contains("Saved successfully."));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A validation failure never redirects (the form re-renders inline
    /// with the existing `.error` div), so it must never set a flash cookie
    /// either — otherwise an unrelated later navigation could surface a
    /// stray "Saved successfully." banner for a save that never happened.
    #[tokio::test]
    async fn create_with_invalid_data_shows_no_flash_cookie() {
        let (state, dir) = test_support::test_app("donors-create-invalid-no-flash");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body = "name=+++&contact_info=&notes=";
        let req = Request::builder()
            .method("POST")
            .uri("/donors")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        assert!(
            res.headers().get("set-cookie").is_none(),
            "a validation failure must not set a flash cookie"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
