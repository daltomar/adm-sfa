use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Form;
use axum::Router;
use serde::Deserialize;

use adm_sfa_core::db::queries::{categories as cat_qry, documents as documents_qry};

use crate::state::AppState;
use crate::templates::{HtmlTemplate, SettingsTemplate};

/// Category and document-label CRUD only — desktop's Settings also has a
/// locale picker, a screenshot-command field, and a manual "backup now"
/// button, none of which are ported here. The locale picker specifically
/// has no web counterpart: `web`'s own UI chrome is translated (see
/// `crate::i18n`), but it follows the single shared `ui_locale` setting
/// desktop's picker writes to — there's nothing per-user to pick here,
/// consistent with "two users, one machine" having one installation-wide
/// language, not per-session preferences. The screenshot command is
/// permanently desktop-only per CLAUDE.md ("a
/// browser cannot invoke the OS screenshot tool on the client machine").
/// Manual backup is skipped deliberately, not overlooked: phase 6 already
/// gives the web deployment its own unattended nightly backup
/// (deploy/adm-sfa-backup.service+.timer) that desktop's manual zip button
/// has no equivalent of, so a duplicate manual trigger here isn't pulling
/// its weight for this pass.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/settings", get(index))
        .route("/settings/categories", post(create_category))
        .route("/settings/categories/{id}", post(rename_category))
        .route("/settings/categories/{id}/delete", post(delete_category))
        .route("/settings/labels", post(create_label))
        .route("/settings/labels/{id}", post(rename_label))
        .route("/settings/labels/{id}/delete", post(delete_label))
}

fn settings_template(conn: &rusqlite::Connection, error: Option<String>) -> SettingsTemplate {
    let categories = cat_qry::list(conn)
        .unwrap_or_default()
        .into_iter()
        .map(|c| (c.id, c.name))
        .collect();
    let labels = documents_qry::list_labels(conn).unwrap_or_default();
    let locale = crate::i18n::resolve_locale(conn);
    SettingsTemplate {
        categories,
        labels,
        error,
        locale,
    }
}

async fn index(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.conn();
    HtmlTemplate(settings_template(&conn, None))
}

#[derive(Deserialize)]
struct NameForm {
    name: String,
}

async fn create_category(State(state): State<AppState>, Form(form): Form<NameForm>) -> Response {
    let conn = state.conn();
    match cat_qry::insert(&conn, &form.name) {
        Ok(_) => Redirect::to("/settings").into_response(),
        Err(e) => HtmlTemplate(settings_template(&conn, Some(e.to_string()))).into_response(),
    }
}

async fn rename_category(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<NameForm>,
) -> Response {
    let conn = state.conn();
    match cat_qry::update(&conn, id, &form.name) {
        Ok(()) => Redirect::to("/settings").into_response(),
        Err(e) => HtmlTemplate(settings_template(&conn, Some(e.to_string()))).into_response(),
    }
}

async fn delete_category(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let conn = state.conn();
    match cat_qry::in_use(&conn, id) {
        Err(e) => HtmlTemplate(settings_template(&conn, Some(e.to_string()))).into_response(),
        Ok(true) => {
            let locale = crate::i18n::resolve_locale(&conn);
            let error =
                rust_i18n::t!("settings.category.error.in_use", locale = &locale).to_string();
            HtmlTemplate(settings_template(&conn, Some(error))).into_response()
        }
        Ok(false) => match cat_qry::delete(&conn, id) {
            Ok(()) => Redirect::to("/settings").into_response(),
            Err(e) => HtmlTemplate(settings_template(&conn, Some(e.to_string()))).into_response(),
        },
    }
}

async fn create_label(State(state): State<AppState>, Form(form): Form<NameForm>) -> Response {
    let conn = state.conn();
    match documents_qry::insert_label(&conn, &form.name) {
        Ok(_) => Redirect::to("/settings").into_response(),
        Err(e) => HtmlTemplate(settings_template(&conn, Some(e.to_string()))).into_response(),
    }
}

async fn rename_label(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<NameForm>,
) -> Response {
    let conn = state.conn();
    match documents_qry::update_label(&conn, id, &form.name) {
        Ok(()) => Redirect::to("/settings").into_response(),
        Err(e) => HtmlTemplate(settings_template(&conn, Some(e.to_string()))).into_response(),
    }
}

async fn delete_label(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let conn = state.conn();
    match documents_qry::delete_label(&conn, id) {
        Ok(()) => Redirect::to("/settings").into_response(),
        Err(e) => HtmlTemplate(settings_template(&conn, Some(e.to_string()))).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support;
    use adm_sfa_core::db::queries::{categories as cat_qry, documents as documents_qry};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};

    #[tokio::test]
    async fn index_lists_categories_and_labels() {
        let (state, dir) = test_support::test_app("settings-index-lists");
        let conn = state.conn();
        cat_qry::insert(&conn, "Decks").unwrap();
        documents_qry::insert_label(&conn, "Invoice").unwrap();
        drop(conn);
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("GET")
            .uri("/settings")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        assert!(body.contains(r#"value="Decks""#));
        assert!(body.contains(r#"value="Invoice""#));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `category` (and `document_label`, further below) come pre-seeded by
    /// `001_initial.sql` (9 default categories, 6 default labels) — every
    /// assertion below has to find its own row by name/id rather than
    /// assume an empty table or a fixed length/index.
    #[tokio::test]
    async fn create_category_redirects_and_inserts() {
        let (state, dir) = test_support::test_app("settings-create-category");
        let before = cat_qry::list(&state.conn()).unwrap().len();
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("POST")
            .uri("/settings/categories")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("name=Skateboard+Wheels"))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get("location").unwrap(), "/settings");
        let categories = cat_qry::list(&state.conn()).unwrap();
        assert_eq!(categories.len(), before + 1);
        assert!(categories.iter().any(|c| c.name == "Skateboard Wheels"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Regression test for the blank-name validation fix documented in
    /// CLAUDE.md (landed at the tail of `phase-5-web-remaining-sections`,
    /// commit `9e109f2`): before that fix, a raw POST with an empty name
    /// silently inserted a blank category — verified live at the time, but
    /// never given an automated test. `core::db::queries::categories::
    /// insert` now rejects a blank/whitespace-only name; this confirms the
    /// route surfaces that as an in-form error rather than a raw 500 or a
    /// silent redirect, and that nothing gets inserted.
    #[tokio::test]
    async fn create_category_with_a_blank_name_rerenders_with_an_error_and_inserts_nothing() {
        let (state, dir) = test_support::test_app("settings-create-category-blank");
        let before = cat_qry::list(&state.conn()).unwrap().len();
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("POST")
            .uri("/settings/categories")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("name=   "))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        assert!(body.contains(r#"<div class="error">"#));
        assert_eq!(cat_qry::list(&state.conn()).unwrap().len(), before);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn rename_category_redirects_and_updates() {
        let (state, dir) = test_support::test_app("settings-rename-category");
        let cat_id = cat_qry::insert(&state.conn(), "Decks").unwrap();
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("POST")
            .uri(format!("/settings/categories/{cat_id}"))
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("name=Skateboard+decks"))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let categories = cat_qry::list(&state.conn()).unwrap();
        let renamed = categories
            .iter()
            .find(|c| c.id == cat_id)
            .expect("renamed category should still exist under the same id");
        assert_eq!(renamed.name, "Skateboard decks");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A category still referenced by an inventory item can't be deleted —
    /// `delete_category` checks `cat_qry::in_use` first and rerenders with
    /// a translated error instead of calling `delete`, mirroring desktop's
    /// own guard.
    #[tokio::test]
    async fn delete_category_in_use_shows_an_error_and_keeps_the_category() {
        use adm_sfa_core::db::queries::{donors as donors_qry, inventory as inv_qry};
        use adm_sfa_core::model::donor::PhysicalDonationDraft;
        use adm_sfa_core::model::inventory::{
            InventoryItemDraft, ItemStatus, Location, SourceType,
        };

        let (state, dir) = test_support::test_app("settings-delete-category-in-use");
        let conn = state.conn();
        let cat_id = cat_qry::insert(&conn, "Decks").unwrap();
        let donation_id = donors_qry::insert_donation(
            &conn,
            &PhysicalDonationDraft {
                donor_id: None,
                date_received: "2026-01-01".to_string(),
                notes: String::new(),
            },
        )
        .unwrap();
        inv_qry::insert(
            &conn,
            &InventoryItemDraft {
                name: "Deck".to_string(),
                category_id: Some(cat_id),
                source_type: SourceType::Donation,
                source_donation_id: Some(donation_id),
                source_purchase_id: None,
                location: Location::Germany,
                status: ItemStatus::Available,
                notes: String::new(),
            },
        )
        .unwrap();
        drop(conn);
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("POST")
            .uri(format!("/settings/categories/{cat_id}/delete"))
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        assert!(body.contains(r#"<div class="error">"#));
        assert!(cat_qry::list(&state.conn())
            .unwrap()
            .iter()
            .any(|c| c.id == cat_id));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn delete_category_not_in_use_redirects_and_removes_it() {
        let (state, dir) = test_support::test_app("settings-delete-category-unused");
        let cat_id = cat_qry::insert(&state.conn(), "Decks").unwrap();
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("POST")
            .uri(format!("/settings/categories/{cat_id}/delete"))
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert!(!cat_qry::list(&state.conn())
            .unwrap()
            .iter()
            .any(|c| c.id == cat_id));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn create_label_redirects_and_inserts() {
        let (state, dir) = test_support::test_app("settings-create-label");
        let before = documents_qry::list_labels(&state.conn()).unwrap().len();
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("POST")
            .uri("/settings/labels")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("name=Invoice"))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let labels = documents_qry::list_labels(&state.conn()).unwrap();
        assert_eq!(labels.len(), before + 1);
        assert!(labels.iter().any(|l| l.1 == "Invoice"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Mirrors `create_category_with_a_blank_name_...` — `document_label`
    /// got the same blank-name fix in the same commit.
    #[tokio::test]
    async fn create_label_with_a_blank_name_rerenders_with_an_error_and_inserts_nothing() {
        let (state, dir) = test_support::test_app("settings-create-label-blank");
        let before = documents_qry::list_labels(&state.conn()).unwrap().len();
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("POST")
            .uri("/settings/labels")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("name=   "))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        assert!(body.contains(r#"<div class="error">"#));
        assert_eq!(
            documents_qry::list_labels(&state.conn()).unwrap().len(),
            before
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn delete_label_redirects_and_removes_it() {
        let (state, dir) = test_support::test_app("settings-delete-label");
        let label_id = documents_qry::insert_label(&state.conn(), "Invoice").unwrap();
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("POST")
            .uri(format!("/settings/labels/{label_id}/delete"))
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert!(!documents_qry::list_labels(&state.conn())
            .unwrap()
            .iter()
            .any(|l| l.0 == label_id));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn index_without_a_session_cookie_redirects_to_login() {
        let (state, dir) = test_support::test_app("settings-index-unauthenticated");
        let app = crate::build_app(state.clone());

        let req = Request::builder()
            .method("GET")
            .uri("/settings")
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get("location").unwrap(), "/login");
        std::fs::remove_dir_all(&dir).ok();
    }
}
