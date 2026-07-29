use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::{Multipart, Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Form;
use axum::Router;
use serde::Deserialize;

use adm_sfa_core::db::queries::{documents as documents_qry, purchases as purchases_qry};
use adm_sfa_core::docs_fs;
use adm_sfa_core::format;
use adm_sfa_core::model::purchase::{Currency, Purchase, PurchaseDraft, PurchaseStatus};
use adm_sfa_core::service;

use crate::state::AppState;
use crate::templates::{HtmlTemplate, PurchaseFormTemplate, PurchaseRow, PurchasesListTemplate};

/// Distinguishes concurrent uploads that would otherwise land on the same
/// temp path (same purchase id, same extension, same process) — see
/// `attach_document`.
static UPLOAD_COUNTER: AtomicU64 = AtomicU64::new(0);

fn draft_from_purchase(p: &Purchase) -> PurchaseDraft {
    PurchaseDraft {
        date: p.date.clone(),
        currency: p.currency,
        cost_str: p.cost.to_string(),
        channel: p.channel.clone(),
        seller_info: p.seller_info.clone().unwrap_or_default(),
        multiple_items: p.multiple_items,
        status: p.status,
    }
}

/// Re-renders the edit form with an error banner, for handlers (upload,
/// document removal) that fail after the point of no redirect-only return.
fn purchase_form_error_response(conn: &rusqlite::Connection, id: i64, error: String) -> Response {
    let documents = documents_qry::list_for_record(conn, "purchase", id).unwrap_or_default();
    let Some(purchase) = purchases_qry::get(conn, id).ok().flatten() else {
        return (axum::http::StatusCode::NOT_FOUND, "purchase not found").into_response();
    };
    let draft = draft_from_purchase(&purchase);
    let labels = documents_qry::labels(conn).unwrap_or_default();
    let locale = crate::i18n::resolve_locale(conn);
    HtmlTemplate(PurchaseFormTemplate {
        id: Some(id),
        is_negotiating: draft.status == PurchaseStatus::Negotiating,
        draft,
        error: Some(error),
        documents,
        labels,
        locale,
    })
    .into_response()
}

fn status_label(status: PurchaseStatus, locale: &str) -> String {
    let key = match status {
        PurchaseStatus::Negotiating => "status.purchase.negotiating",
        PurchaseStatus::Bought => "status.purchase.bought",
    };
    rust_i18n::t!(key, locale = locale).to_string()
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/purchases", get(list).post(create))
        .route("/purchases/new", get(new_form))
        .route("/purchases/{id}/edit", get(edit_form))
        .route("/purchases/{id}", post(update))
        .route("/purchases/{id}/mark-bought", post(mark_bought))
        .route("/purchases/{id}/drop", post(drop_negotiating))
        .route("/purchases/{id}/documents", post(attach_document))
        .route(
            "/purchases/{id}/documents/{doc_id}/delete",
            post(remove_document),
        )
}

async fn list(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let purchases = purchases_qry::list(&conn).unwrap_or_default();
    let rows = purchases
        .into_iter()
        .map(|p| PurchaseRow {
            id: p.id,
            date_display: format::date(&p.date),
            channel: p.channel,
            cost_display: format::amount(p.cost),
            currency_symbol: p.currency.symbol(),
            status_label: status_label(p.status, &locale),
            multiple_items: p.multiple_items,
        })
        .collect();
    HtmlTemplate(PurchasesListTemplate {
        purchases: rows,
        locale,
    })
}

async fn new_form(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    HtmlTemplate(PurchaseFormTemplate {
        id: None,
        draft: PurchaseDraft::default(),
        error: None,
        documents: Vec::new(),
        is_negotiating: false,
        labels: Vec::new(),
        locale,
    })
}

async fn edit_form(State(state): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let Some(purchase) = purchases_qry::get(&conn, id).ok().flatten() else {
        return (axum::http::StatusCode::NOT_FOUND, "purchase not found").into_response();
    };
    let documents = documents_qry::list_for_record(&conn, "purchase", id).unwrap_or_default();
    let labels = documents_qry::labels(&conn).unwrap_or_default();
    let draft = draft_from_purchase(&purchase);
    HtmlTemplate(PurchaseFormTemplate {
        id: Some(id),
        is_negotiating: draft.status == PurchaseStatus::Negotiating,
        draft,
        error: None,
        documents,
        labels,
        locale,
    })
    .into_response()
}

#[derive(Deserialize)]
struct PurchaseForm {
    date: String,
    currency: String,
    cost_str: String,
    channel: String,
    #[serde(default)]
    seller_info: String,
    /// HTML checkboxes only appear in the submitted body when checked, so
    /// these are `Some(_)` (any value) when on, absent (deserializes as
    /// `None`) when off — deliberately not `bool`, which `serde_urlencoded`
    /// doesn't reliably map from an absent field.
    multiple_items: Option<String>,
    /// Create-form only — matches desktop's "Start as negotiating"
    /// checkbox. Not present on the edit form at all: status changes after
    /// creation go through the dedicated "Mark as bought" action, never a
    /// resubmitted form field.
    #[serde(default)]
    negotiating: Option<String>,
}

fn draft_from_form(form: PurchaseForm, status: PurchaseStatus) -> PurchaseDraft {
    PurchaseDraft {
        date: form.date,
        currency: Currency::from_str(&form.currency).unwrap_or(Currency::Eur),
        cost_str: form.cost_str,
        channel: form.channel,
        seller_info: form.seller_info,
        multiple_items: form.multiple_items.is_some(),
        status,
    }
}

async fn create(
    State(state): State<AppState>,
    Form(form): Form<PurchaseForm>,
) -> impl IntoResponse {
    let status = if form.negotiating.is_some() {
        PurchaseStatus::Negotiating
    } else {
        PurchaseStatus::Bought
    };
    let draft = draft_from_form(form, status);
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    match service::create_purchase(&conn, &draft) {
        Ok(id) => Redirect::to(&format!("/purchases/{id}/edit")).into_response(),
        Err(e) => HtmlTemplate(PurchaseFormTemplate {
            id: None,
            draft,
            error: Some(e.to_string()),
            documents: Vec::new(),
            is_negotiating: false,
            labels: Vec::new(),
            locale,
        })
        .into_response(),
    }
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<PurchaseForm>,
) -> impl IntoResponse {
    let conn = state.conn();
    // Preserve the persisted status — editing the other fields must not
    // silently change negotiating/bought; that's `mark_bought`'s job, and
    // `purchases_qry::update` itself refuses to let a bought purchase
    // revert regardless of what's submitted here.
    let current_status = purchases_qry::get(&conn, id)
        .ok()
        .flatten()
        .map(|p| p.status)
        .unwrap_or(PurchaseStatus::Bought);
    let draft = draft_from_form(form, current_status);
    let locale = crate::i18n::resolve_locale(&conn);

    if !draft.multiple_items {
        if let Ok(Some(n)) = purchases_qry::multiple_items_unset_conflict(&conn, id) {
            let documents =
                documents_qry::list_for_record(&conn, "purchase", id).unwrap_or_default();
            let labels = documents_qry::labels(&conn).unwrap_or_default();
            let error = rust_i18n::t!(
                "purchases.error.cannot_mark_single_other",
                locale = &locale,
                n = n
            )
            .to_string();
            return HtmlTemplate(PurchaseFormTemplate {
                id: Some(id),
                is_negotiating: draft.status == PurchaseStatus::Negotiating,
                draft,
                error: Some(error),
                documents,
                labels,
                locale,
            })
            .into_response();
        }
    }

    match purchases_qry::update(&conn, id, &draft) {
        Ok(()) => Redirect::to(&format!("/purchases/{id}/edit")).into_response(),
        Err(e) => {
            let documents =
                documents_qry::list_for_record(&conn, "purchase", id).unwrap_or_default();
            let labels = documents_qry::labels(&conn).unwrap_or_default();
            HtmlTemplate(PurchaseFormTemplate {
                id: Some(id),
                is_negotiating: draft.status == PurchaseStatus::Negotiating,
                draft,
                error: Some(e.to_string()),
                documents,
                labels,
                locale,
            })
            .into_response()
        }
    }
}

async fn mark_bought(State(state): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    let conn = state.conn();
    let Some(purchase) = purchases_qry::get(&conn, id).ok().flatten() else {
        return (axum::http::StatusCode::NOT_FOUND, "purchase not found").into_response();
    };
    let draft = draft_from_purchase(&purchase);
    match service::mark_purchase_bought(&conn, id, &draft) {
        Ok(()) => Redirect::to(&format!("/purchases/{id}/edit")).into_response(),
        Err(e) => {
            let documents =
                documents_qry::list_for_record(&conn, "purchase", id).unwrap_or_default();
            let labels = documents_qry::labels(&conn).unwrap_or_default();
            let locale = crate::i18n::resolve_locale(&conn);
            HtmlTemplate(PurchaseFormTemplate {
                id: Some(id),
                is_negotiating: draft.status == PurchaseStatus::Negotiating,
                draft,
                error: Some(e.to_string()),
                documents,
                labels,
                locale,
            })
            .into_response()
        }
    }
}

async fn drop_negotiating(State(state): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    let conn = state.conn();
    match service::drop_negotiating_purchase(&conn, &state.documents_dir, id) {
        Ok(()) => Redirect::to("/purchases").into_response(),
        Err(e) => (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
    }
}

async fn attach_document(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let mut label = "other".to_string();
    let mut tmp_path: Option<std::path::PathBuf> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        match field.name().unwrap_or("") {
            "label" => {
                if let Ok(text) = field.text().await {
                    if !text.trim().is_empty() {
                        label = text;
                    }
                }
            }
            "file" => {
                let original_name = field.file_name().unwrap_or("upload.bin").to_string();
                if let Ok(bytes) = field.bytes().await {
                    if !bytes.is_empty() {
                        let ext = std::path::Path::new(&original_name)
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("bin");
                        let unique = UPLOAD_COUNTER.fetch_add(1, Ordering::Relaxed);
                        let path = std::env::temp_dir().join(format!(
                            "adm-sfa-web-upload-{id}-{}-{unique}.{ext}",
                            std::process::id()
                        ));
                        if std::fs::write(&path, &bytes).is_ok() {
                            tmp_path = Some(path);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let Some(tmp_path) = tmp_path else {
        let conn = state.conn();
        let locale = crate::i18n::resolve_locale(&conn);
        let error = rust_i18n::t!("web.doc.error.no_file", locale = &locale).to_string();
        return purchase_form_error_response(&conn, id, error);
    };

    let conn = state.conn();
    let persisted_date = purchases_qry::get(&conn, id).ok().flatten().map(|p| p.date);
    let existing: Vec<String> = documents_qry::list_for_record(&conn, "purchase", id)
        .unwrap_or_default()
        .into_iter()
        .map(|d| d.filename)
        .collect();

    let result = service::attach_document(
        &conn,
        &state.documents_dir,
        &tmp_path,
        "",
        persisted_date.as_deref(),
        ("purchase", id),
        &label,
        &existing,
    );
    let _ = std::fs::remove_file(&tmp_path);

    match result {
        Ok(_) => Redirect::to(&format!("/purchases/{id}/edit")).into_response(),
        Err(e) => purchase_form_error_response(&conn, id, e),
    }
}

async fn remove_document(
    State(state): State<AppState>,
    Path((id, doc_id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let conn = state.conn();
    let Some(doc) = documents_qry::list_for_record(&conn, "purchase", id)
        .unwrap_or_default()
        .into_iter()
        .find(|d| d.id == doc_id)
    else {
        return Redirect::to(&format!("/purchases/{id}/edit")).into_response();
    };
    match docs_fs::remove_document(&conn, &state.documents_dir, doc.id, &doc.filename) {
        Ok(()) => Redirect::to(&format!("/purchases/{id}/edit")).into_response(),
        Err(e) => purchase_form_error_response(&conn, id, e),
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support;
    use adm_sfa_core::db::queries::{documents as documents_qry, purchases as purchases_qry};
    use adm_sfa_core::model::purchase::{Currency, PurchaseDraft, PurchaseStatus};
    use axum::http::StatusCode;

    fn a_purchase_draft() -> PurchaseDraft {
        PurchaseDraft {
            date: "2026-01-01".to_string(),
            currency: Currency::Eur,
            cost_str: "50.00".to_string(),
            channel: "Kleinanzeigen".to_string(),
            seller_info: String::new(),
            multiple_items: false,
            status: PurchaseStatus::Bought,
        }
    }

    /// Regression coverage for the document-label allow-list fix
    /// (CLAUDE.md backlog): the multipart upload's `label` field used to be
    /// arbitrary free text with no check against `document_label` at all.
    #[tokio::test]
    async fn attach_document_rejects_an_unknown_label_and_rerenders_the_form() {
        let (state, dir) = test_support::test_app("attach-doc-unknown-label");
        let purchase_id = purchases_qry::insert(&state.conn(), &a_purchase_draft()).unwrap();
        let app = crate::build_app(state.clone());

        let cookie = test_support::login(&app).await;
        let req = test_support::multipart_request(
            &format!("/purchases/{purchase_id}/documents"),
            &cookie,
            "not-a-real-label",
            b"fake image bytes",
        );
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        assert!(body.contains("Unknown document label"));
        assert!(
            documents_qry::list_for_record(&state.conn(), "purchase", purchase_id)
                .unwrap()
                .is_empty()
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn attach_document_with_a_known_label_creates_a_document_and_redirects() {
        let (state, dir) = test_support::test_app("attach-doc-known-label");
        let purchase_id = purchases_qry::insert(&state.conn(), &a_purchase_draft()).unwrap();
        let app = crate::build_app(state.clone());

        let cookie = test_support::login(&app).await;
        let req = test_support::multipart_request(
            &format!("/purchases/{purchase_id}/documents"),
            &cookie,
            "chat",
            b"fake image bytes",
        );
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let docs = documents_qry::list_for_record(&state.conn(), "purchase", purchase_id).unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].label, "chat");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn attach_document_without_a_session_cookie_redirects_to_login() {
        let (state, dir) = test_support::test_app("attach-doc-unauthenticated");
        let purchase_id = purchases_qry::insert(&state.conn(), &a_purchase_draft()).unwrap();
        let app = crate::build_app(state.clone());

        let req = test_support::multipart_request(
            &format!("/purchases/{purchase_id}/documents"),
            "", // no cookie at all
            "chat",
            b"fake image bytes",
        );
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get("location").unwrap(), "/login");
        assert!(
            documents_qry::list_for_record(&state.conn(), "purchase", purchase_id)
                .unwrap()
                .is_empty()
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn attach_document_with_no_file_rerenders_the_form_with_an_error() {
        let (state, dir) = test_support::test_app("attach-doc-no-file");
        let purchase_id = purchases_qry::insert(&state.conn(), &a_purchase_draft()).unwrap();
        let app = crate::build_app(state.clone());

        let cookie = test_support::login(&app).await;
        // Empty file bytes mirrors what a browser sends for an untouched
        // `<input type="file">`: the `file` field is present with an empty
        // filename/body, not absent — `attach_document`'s `!bytes.is_empty()`
        // check is what actually distinguishes "no file" from a real upload.
        let req = test_support::multipart_request(
            &format!("/purchases/{purchase_id}/documents"),
            &cookie,
            "chat",
            b"",
        );
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        assert!(body.contains("No file was selected"));
        assert!(
            documents_qry::list_for_record(&state.conn(), "purchase", purchase_id)
                .unwrap()
                .is_empty()
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Regression test locking in what CLAUDE.md's backlog only verified by
    /// hand: a path-traversal-flavored label is rejected by the allow-list,
    /// not merely "harmless by accident of filename construction" as it was
    /// before that fix landed.
    #[tokio::test]
    async fn attach_document_rejects_a_path_traversal_flavored_label() {
        let (state, dir) = test_support::test_app("attach-doc-path-traversal-label");
        let purchase_id = purchases_qry::insert(&state.conn(), &a_purchase_draft()).unwrap();
        let app = crate::build_app(state.clone());

        let cookie = test_support::login(&app).await;
        let req = test_support::multipart_request(
            &format!("/purchases/{purchase_id}/documents"),
            &cookie,
            "../../../../tmp/pwned",
            b"fake image bytes",
        );
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        assert!(body.contains("Unknown document label"));
        assert!(
            documents_qry::list_for_record(&state.conn(), "purchase", purchase_id)
                .unwrap()
                .is_empty()
        );
        assert!(
            !std::path::Path::new("/tmp/pwned").exists(),
            "the traversal-flavored label must not have escaped the documents dir"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
