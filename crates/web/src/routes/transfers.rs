use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::{Multipart, Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Form;
use axum::Router;
use serde::Deserialize;

use adm_sfa_core::db::queries::{documents as documents_qry, transfers as qry};
use adm_sfa_core::docs_fs;
use adm_sfa_core::format;
use adm_sfa_core::model::transfer::{AnnualTransfer, TransferDraft};
use adm_sfa_core::service;

use crate::state::AppState;
use crate::templates::{HtmlTemplate, TransferFormTemplate, TransferRow, TransfersListTemplate};

/// Distinguishes concurrent uploads landing on the same temp path — same
/// pattern (and same reason) as `purchases.rs`'s `UPLOAD_COUNTER`.
static UPLOAD_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/transfers", get(list).post(create))
        .route("/transfers/new", get(new_form))
        .route("/transfers/{id}/edit", get(edit_form))
        .route("/transfers/{id}", post(update))
        .route("/transfers/{id}/documents", post(attach_document))
        .route(
            "/transfers/{id}/documents/{doc_id}/delete",
            post(remove_document),
        )
}

fn draft_from_transfer(t: &AnnualTransfer) -> TransferDraft {
    TransferDraft {
        date: t.date.clone(),
        eur_amount_sent_str: t.eur_amount_sent.to_string(),
        exchange_rate_str: t.exchange_rate.to_string(),
        notes: t.notes.clone().unwrap_or_default(),
    }
}

/// Full "BRL amount received: R$ 123.45" sentence, pre-formatted here (not
/// in the template) since word order around an interpolated amount can
/// differ per language — same reasoning as `EurLedgerListTemplate::
/// balance_label`.
fn brl_preview(draft: &TransferDraft, locale: &str) -> Option<String> {
    let eur = adm_sfa_core::money::parse_amount_input(draft.eur_amount_sent_str.trim())?;
    let rate = adm_sfa_core::money::parse_amount_input(draft.exchange_rate_str.trim())?;
    // `checked_mul` (not `*`), matching `core::db::queries::transfers`'s own
    // guard against `Decimal`'s panicking multiplication — a huge but
    // individually-parseable amount/rate pair must not crash the request
    // just to render a preview label. Silently omitting the preview here
    // (like the existing "unparseable" case) is fine since `insert`/`update`
    // is the actual authority and will reject the same input with a real
    // error banner instead of a preview.
    let amount = eur.checked_mul(rate).map(format::amount)?;
    Some(
        rust_i18n::t!(
            "transfers.field.brl_received",
            locale = locale,
            amount = &amount
        )
        .to_string(),
    )
}

#[allow(clippy::too_many_arguments)]
fn form_template(
    id: Option<i64>,
    draft: TransferDraft,
    error: Option<String>,
    documents: Vec<adm_sfa_core::model::document::Document>,
    labels: Vec<String>,
    locale: String,
) -> TransferFormTemplate {
    let brl_preview = brl_preview(&draft, &locale);
    TransferFormTemplate {
        id,
        date: draft.date,
        eur_amount_sent_str: draft.eur_amount_sent_str,
        exchange_rate_str: draft.exchange_rate_str,
        notes: draft.notes,
        brl_preview,
        error,
        documents,
        labels,
        locale,
    }
}

/// Re-renders the edit form with an error banner — same pattern as
/// `purchases.rs::purchase_form_error_response`.
fn transfer_form_error_response(conn: &rusqlite::Connection, id: i64, error: String) -> Response {
    let documents = documents_qry::list_for_record(conn, "transfer", id).unwrap_or_default();
    let Some(transfer) = qry::get(conn, id).ok().flatten() else {
        return (axum::http::StatusCode::NOT_FOUND, "transfer not found").into_response();
    };
    let draft = draft_from_transfer(&transfer);
    let labels = documents_qry::labels(conn).unwrap_or_default();
    let locale = crate::i18n::resolve_locale(conn);
    HtmlTemplate(form_template(
        Some(id),
        draft,
        Some(error),
        documents,
        labels,
        locale,
    ))
    .into_response()
}

async fn list(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let transfers = qry::list(&conn).unwrap_or_default();
    let rows = transfers
        .into_iter()
        .map(|t| TransferRow {
            id: t.id,
            date_display: format::date(&t.date),
            eur_display: format::amount(t.eur_amount_sent),
            brl_display: format::amount(t.brl_amount_received),
            rate_display: format::number(t.exchange_rate, 4),
        })
        .collect();
    HtmlTemplate(TransfersListTemplate {
        transfers: rows,
        locale,
    })
}

async fn new_form(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let draft = TransferDraft {
        date: chrono::Local::now().format("%Y-%m-%d").to_string(),
        ..TransferDraft::default()
    };
    HtmlTemplate(form_template(
        None,
        draft,
        None,
        Vec::new(),
        Vec::new(),
        locale,
    ))
}

async fn edit_form(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let Some(transfer) = qry::get(&conn, id).ok().flatten() else {
        return (axum::http::StatusCode::NOT_FOUND, "transfer not found").into_response();
    };
    let documents = documents_qry::list_for_record(&conn, "transfer", id).unwrap_or_default();
    let labels = documents_qry::labels(&conn).unwrap_or_default();
    let draft = draft_from_transfer(&transfer);
    HtmlTemplate(form_template(
        Some(id),
        draft,
        None,
        documents,
        labels,
        locale,
    ))
    .into_response()
}

#[derive(Deserialize)]
struct TransferForm {
    date: String,
    eur_amount_sent_str: String,
    exchange_rate_str: String,
    #[serde(default)]
    notes: String,
}

fn draft_from_form(form: TransferForm) -> TransferDraft {
    TransferDraft {
        date: form.date,
        eur_amount_sent_str: form.eur_amount_sent_str,
        exchange_rate_str: form.exchange_rate_str,
        notes: form.notes,
    }
}

async fn create(State(state): State<AppState>, Form(form): Form<TransferForm>) -> Response {
    let draft = draft_from_form(form);
    let conn = state.conn();
    match qry::insert(&conn, &draft) {
        Ok(id) => Redirect::to(&format!("/transfers/{id}/edit")).into_response(),
        Err(e) => {
            let locale = crate::i18n::resolve_locale(&conn);
            HtmlTemplate(form_template(
                None,
                draft,
                Some(e.to_string()),
                Vec::new(),
                Vec::new(),
                locale,
            ))
            .into_response()
        }
    }
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<TransferForm>,
) -> Response {
    let draft = draft_from_form(form);
    let conn = state.conn();
    match qry::update(&conn, id, &draft) {
        Ok(()) => Redirect::to(&format!("/transfers/{id}/edit")).into_response(),
        Err(e) => {
            let documents =
                documents_qry::list_for_record(&conn, "transfer", id).unwrap_or_default();
            let labels = documents_qry::labels(&conn).unwrap_or_default();
            let locale = crate::i18n::resolve_locale(&conn);
            HtmlTemplate(form_template(
                Some(id),
                draft,
                Some(e.to_string()),
                documents,
                labels,
                locale,
            ))
            .into_response()
        }
    }
}

async fn attach_document(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    mut multipart: Multipart,
) -> Response {
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
                            "adm-sfa-web-upload-transfer-{id}-{}-{unique}.{ext}",
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
        return transfer_form_error_response(&conn, id, error);
    };

    let conn = state.conn();
    let persisted_date = qry::get(&conn, id).ok().flatten().map(|t| t.date);
    let existing: Vec<String> = documents_qry::list_for_record(&conn, "transfer", id)
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
        ("transfer", id),
        &label,
        &existing,
    );
    let _ = std::fs::remove_file(&tmp_path);

    match result {
        Ok(_) => Redirect::to(&format!("/transfers/{id}/edit")).into_response(),
        Err(e) => transfer_form_error_response(&conn, id, e),
    }
}

async fn remove_document(
    State(state): State<AppState>,
    Path((id, doc_id)): Path<(i64, i64)>,
) -> Response {
    let conn = state.conn();
    let Some(doc) = documents_qry::list_for_record(&conn, "transfer", id)
        .unwrap_or_default()
        .into_iter()
        .find(|d| d.id == doc_id)
    else {
        return Redirect::to(&format!("/transfers/{id}/edit")).into_response();
    };
    match docs_fs::remove_document(&conn, &state.documents_dir, doc.id, &doc.filename) {
        Ok(()) => Redirect::to(&format!("/transfers/{id}/edit")).into_response(),
        Err(e) => transfer_form_error_response(&conn, id, e),
    }
}
