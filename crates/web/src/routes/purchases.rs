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
    let Some(purchase) = purchases_qry::list(conn)
        .unwrap_or_default()
        .into_iter()
        .find(|p| p.id == id)
    else {
        return (axum::http::StatusCode::NOT_FOUND, "purchase not found").into_response();
    };
    let draft = draft_from_purchase(&purchase);
    let labels = documents_qry::labels(conn).unwrap_or_default();
    HtmlTemplate(PurchaseFormTemplate {
        id: Some(id),
        is_negotiating: draft.status == PurchaseStatus::Negotiating,
        draft,
        error: Some(error),
        documents,
        labels,
    })
    .into_response()
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
    let purchases = purchases_qry::list(&conn).unwrap_or_default();
    let rows = purchases
        .into_iter()
        .map(|p| PurchaseRow {
            id: p.id,
            date_display: format::date(&p.date),
            channel: p.channel,
            cost_display: format::amount(p.cost),
            currency_symbol: p.currency.symbol(),
            status_label: match p.status {
                PurchaseStatus::Negotiating => "Negotiating",
                PurchaseStatus::Bought => "Bought",
            },
            multiple_items: p.multiple_items,
        })
        .collect();
    HtmlTemplate(PurchasesListTemplate { purchases: rows })
}

async fn new_form() -> impl IntoResponse {
    HtmlTemplate(PurchaseFormTemplate {
        id: None,
        draft: PurchaseDraft::default(),
        error: None,
        documents: Vec::new(),
        is_negotiating: false,
        labels: Vec::new(),
    })
}

async fn edit_form(State(state): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    let conn = state.conn();
    let Some(purchase) = purchases_qry::list(&conn)
        .unwrap_or_default()
        .into_iter()
        .find(|p| p.id == id)
    else {
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
    match service::create_purchase(&conn, &draft) {
        Ok(id) => Redirect::to(&format!("/purchases/{id}/edit")).into_response(),
        Err(e) => HtmlTemplate(PurchaseFormTemplate {
            id: None,
            draft,
            error: Some(e.to_string()),
            documents: Vec::new(),
            is_negotiating: false,
            labels: Vec::new(),
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
    let current_status = purchases_qry::list(&conn)
        .unwrap_or_default()
        .into_iter()
        .find(|p| p.id == id)
        .map(|p| p.status)
        .unwrap_or(PurchaseStatus::Bought);
    let draft = draft_from_form(form, current_status);

    if !draft.multiple_items {
        if let Ok(Some(n)) = purchases_qry::multiple_items_unset_conflict(&conn, id) {
            let documents =
                documents_qry::list_for_record(&conn, "purchase", id).unwrap_or_default();
            let labels = documents_qry::labels(&conn).unwrap_or_default();
            return HtmlTemplate(PurchaseFormTemplate {
                id: Some(id),
                is_negotiating: draft.status == PurchaseStatus::Negotiating,
                draft,
                error: Some(format!(
                    "Cannot mark as single-item: {n} inventory items are already linked."
                )),
                documents,
                labels,
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
            })
            .into_response()
        }
    }
}

async fn mark_bought(State(state): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    let conn = state.conn();
    let Some(purchase) = purchases_qry::list(&conn)
        .unwrap_or_default()
        .into_iter()
        .find(|p| p.id == id)
    else {
        return (axum::http::StatusCode::NOT_FOUND, "purchase not found").into_response();
    };
    let draft = draft_from_purchase(&purchase);
    match service::mark_purchase_bought(&conn, id, &draft) {
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
        return purchase_form_error_response(
            &conn,
            id,
            "No file was selected, or the upload could not be saved.".to_string(),
        );
    };

    let conn = state.conn();
    let persisted_date = purchases_qry::list(&conn)
        .unwrap_or_default()
        .into_iter()
        .find(|p| p.id == id)
        .map(|p| p.date);
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
