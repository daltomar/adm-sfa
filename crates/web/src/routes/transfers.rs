use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::{Multipart, Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Form;
use axum::Router;
use serde::Deserialize;

use adm_sfa_core::db::queries::{documents as documents_qry, transfers as qry};
use adm_sfa_core::docs_fs;
use adm_sfa_core::format;
use adm_sfa_core::model::transfer::{AnnualTransfer, TransferDraft};
use adm_sfa_core::service::{self, PendingDocument};

use crate::state::AppState;
use crate::templates::{
    AttachResult, HtmlTemplate, TransferFormTemplate, TransferRow, TransfersListTemplate,
};

/// Distinguishes concurrent uploads landing on the same temp path — same
/// pattern (and same reason) as `purchases.rs`'s `UPLOAD_COUNTER`.
static UPLOAD_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/transfers", get(list).post(create))
        .route("/transfers/new", get(new_form))
        .route("/transfers/preview", get(preview))
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
    let amount = eur
        .checked_mul(rate)
        .map(|amt| format::amount_in(amt, locale))?;
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
    attach_results: Vec<AttachResult>,
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
        attach_results,
        locale,
    }
}

/// Re-renders the edit form for handlers that fail (or partially fail)
/// after the point of no redirect-only return — document upload, document
/// removal, and a create-with-documents call whose transfer saved but not
/// every staged document attached (see `create`). Mirrors `purchases.rs`'s
/// `purchase_form_response`.
fn transfer_form_response(
    conn: &rusqlite::Connection,
    id: i64,
    error: Option<String>,
    attach_results: Vec<AttachResult>,
) -> Response {
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
        error,
        documents,
        labels,
        attach_results,
        locale,
    ))
    .into_response()
}

/// Re-renders the edit form with an error banner and no attach-results —
/// the common case among `transfer_form_response`'s callers.
fn transfer_form_error_response(conn: &rusqlite::Connection, id: i64, error: String) -> Response {
    transfer_form_response(conn, id, Some(error), Vec::new())
}

async fn list(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let transfers = qry::list(&conn).unwrap_or_default();
    // `qry::list` returns newest-first (shared with desktop, which relies on
    // that same order — reversed here, web-presentation-only, matching the
    // precedent set for the EUR Ledger list page).
    let rows = transfers
        .iter()
        .rev()
        .map(|t| TransferRow {
            id: t.id,
            date_display: format::date(&t.date),
            eur_display: format::amount_in(t.eur_amount_sent, &locale),
            brl_display: format::amount_in(t.brl_amount_received, &locale),
            rate_display: format::number_in(t.exchange_rate, 4, &locale),
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
    // Populated (unlike before the create-with-documents form gained its own
    // repeatable label pickers): the create page now needs the same
    // allow-list as the edit page's attach form.
    let labels = documents_qry::labels(&conn).unwrap_or_default();
    let draft = TransferDraft {
        date: chrono::Local::now().format("%Y-%m-%d").to_string(),
        ..TransferDraft::default()
    };
    HtmlTemplate(form_template(
        None,
        draft,
        None,
        Vec::new(),
        labels,
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
        Vec::new(),
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

/// Query params for the live "BRL received" preview (`GET
/// /transfers/preview`) — the form's own JS fires this on every keystroke
/// in the EUR amount / exchange rate fields, since replicating
/// `core::format`'s locale-aware decimal formatting (T6-adjacent: German-
/// format thousands/decimal separators differ per locale) and the
/// translated `%{amount}` sentence in client-side JS would mean a second,
/// divergence-prone copy of logic that already lives in `brl_preview`.
/// Round-tripping through the server on every input event keeps this a
/// single source of truth instead.
#[derive(Deserialize)]
struct PreviewQuery {
    #[serde(default)]
    eur_amount_sent_str: String,
    #[serde(default)]
    exchange_rate_str: String,
}

/// Returns the plain-text preview line (or an empty body if the amount/rate
/// don't currently parse) for the form's live-update JS to drop into the
/// page. Not a template render — just the same string `form_template`
/// would have embedded, on its own.
async fn preview(
    State(state): State<AppState>,
    Query(q): Query<PreviewQuery>,
) -> impl IntoResponse {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let draft = TransferDraft {
        date: String::new(),
        eur_amount_sent_str: q.eur_amount_sent_str,
        exchange_rate_str: q.exchange_rate_str,
        notes: String::new(),
    };
    brl_preview(&draft, &locale).unwrap_or_default()
}

/// Creates a transfer and, in the same submission, attaches every document
/// staged in the "Documents?" section of the new-transfer form — a single
/// `multipart/form-data` POST rather than the urlencoded `Form<TransferForm>`
/// `update` still uses, since a file can only travel in a multipart body.
/// Mirrors `purchases.rs::create` field-for-field (repeated `doc_label`/
/// `doc_file` pairs paired positionally, not by index).
async fn create(State(state): State<AppState>, mut multipart: Multipart) -> Response {
    let mut date = String::new();
    let mut eur_amount_sent_str = String::new();
    let mut exchange_rate_str = String::new();
    let mut notes = String::new();

    let mut last_label: Option<String> = None;
    let mut pending: Vec<(PathBuf, String, String)> = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        match field.name().unwrap_or("") {
            "date" => date = field.text().await.unwrap_or_default(),
            "eur_amount_sent_str" => eur_amount_sent_str = field.text().await.unwrap_or_default(),
            "exchange_rate_str" => exchange_rate_str = field.text().await.unwrap_or_default(),
            "notes" => notes = field.text().await.unwrap_or_default(),
            "doc_label" => {
                last_label = field.text().await.ok();
            }
            "doc_file" => {
                // `take()` unconditionally, even for a row whose file ends up
                // empty — otherwise an unused row's label could leak onto
                // the next row's file.
                let label = last_label
                    .take()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| "other".to_string());
                let original_name = field.file_name().unwrap_or("upload.bin").to_string();
                if let Ok(bytes) = field.bytes().await {
                    // A "Documents? Yes" section with an unused extra row is
                    // legitimate — an empty file here is silently skipped,
                    // not an error, unlike the edit page's dedicated attach
                    // form where a submission implies real upload intent.
                    if !bytes.is_empty() {
                        let ext = std::path::Path::new(&original_name)
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("bin");
                        let unique = UPLOAD_COUNTER.fetch_add(1, Ordering::Relaxed);
                        let path = std::env::temp_dir().join(format!(
                            "adm-sfa-web-upload-transfer-new-{}-{unique}.{ext}",
                            std::process::id()
                        ));
                        if std::fs::write(&path, &bytes).is_ok() {
                            pending.push((path, label, original_name));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let draft = TransferDraft {
        date,
        eur_amount_sent_str,
        exchange_rate_str,
        notes,
    };

    let pending_docs: Vec<PendingDocument> = pending
        .iter()
        .map(|(path, label, _)| PendingDocument {
            path: path.as_path(),
            label: label.as_str(),
        })
        .collect();

    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let result =
        service::create_transfer_with_documents(&conn, &state.documents_dir, &draft, &pending_docs);

    // Unconditional cleanup — the batch call above never keeps a temp path
    // around, whether it attached, failed, or was never reached because the
    // transfer itself failed to save.
    for (path, _, _) in &pending {
        let _ = std::fs::remove_file(path);
    }

    match result {
        Err(e) => {
            let labels = documents_qry::labels(&conn).unwrap_or_default();
            // A browser cannot repopulate a file input for security reasons,
            // so any staged files are unavoidably lost on this path — tell
            // the user rather than silently dropping them.
            let mut error = e.to_string();
            if !pending.is_empty() {
                let notice = rust_i18n::t!("web.doc.notice.reselect_after_error", locale = &locale)
                    .to_string();
                error = format!("{error} {notice}");
            }
            HtmlTemplate(form_template(
                None,
                draft,
                Some(error),
                Vec::new(),
                labels,
                Vec::new(),
                locale,
            ))
            .into_response()
        }
        Ok(created) if created.attachments.iter().all(|a| a.result.is_ok()) => {
            Redirect::to("/transfers").into_response()
        }
        Ok(created) => {
            // Zipped by index rather than using `a.source_name` — `pending`
            // and `created.attachments` are guaranteed the same length and
            // order (see `attach_documents`'s doc comment), and `pending`
            // carries the original filename the user picked, not the
            // generated temp path `source_name` would otherwise show.
            let attach_results: Vec<AttachResult> = created
                .attachments
                .iter()
                .zip(pending.iter())
                .map(|(a, (_, _, original_name))| match &a.result {
                    Ok(_) => AttachResult {
                        ok: true,
                        message: rust_i18n::t!(
                            "common.doc.status.attached",
                            locale = &locale,
                            name = original_name,
                            label = &a.label
                        )
                        .to_string(),
                    },
                    Err(err) => AttachResult {
                        ok: false,
                        message: rust_i18n::t!(
                            "common.doc.status.failed",
                            locale = &locale,
                            name = original_name,
                            error = err
                        )
                        .to_string(),
                    },
                })
                .collect();
            transfer_form_response(&conn, created.id, None, attach_results)
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
                Vec::new(),
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

#[cfg(test)]
mod tests {
    use crate::test_support;
    use adm_sfa_core::db::queries::{documents as documents_qry, transfers as qry};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};

    /// Base transfer-fields-only parts for `create`'s multipart body,
    /// shared by the tests below — mirrors `purchases.rs`'s
    /// `base_create_parts`.
    fn base_create_parts<'a>() -> Vec<test_support::MultipartPart<'a>> {
        use test_support::MultipartPart::Text;
        vec![
            Text {
                name: "date",
                value: "2026-01-01",
            },
            Text {
                name: "eur_amount_sent_str",
                value: "1000.00",
            },
            Text {
                name: "exchange_rate_str",
                value: "5.5",
            },
            Text {
                name: "notes",
                value: "",
            },
        ]
    }

    #[tokio::test]
    async fn create_with_two_documents_saves_the_transfer_and_both_files_then_redirects_to_the_list(
    ) {
        use test_support::MultipartPart::{File, Text};
        let (state, dir) = test_support::test_app("transfer-create-with-two-documents");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let mut parts = base_create_parts();
        parts.push(Text {
            name: "doc_label",
            value: "chat",
        });
        parts.push(File {
            name: "doc_file",
            filename: "chat.png",
            bytes: b"fake chat bytes",
        });
        parts.push(Text {
            name: "doc_label",
            value: "receipt",
        });
        parts.push(File {
            name: "doc_file",
            filename: "receipt.pdf",
            bytes: b"fake receipt bytes",
        });

        let req = test_support::multipart_request_with_parts("/transfers", &cookie, &parts);
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get("location").unwrap(), "/transfers");
        let conn = state.conn();
        let transfers = qry::list(&conn).unwrap();
        assert_eq!(transfers.len(), 1);
        assert_eq!(
            documents_qry::list_for_record(&conn, "transfer", transfers[0].id)
                .unwrap()
                .len(),
            2
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn create_without_documents_still_redirects_to_the_list() {
        let (state, dir) = test_support::test_app("transfer-create-without-documents");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let parts = base_create_parts();
        let req = test_support::multipart_request_with_parts("/transfers", &cookie, &parts);
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get("location").unwrap(), "/transfers");
        let conn = state.conn();
        assert_eq!(qry::list(&conn).unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn create_with_an_empty_file_row_ignores_it() {
        use test_support::MultipartPart::{File, Text};
        let (state, dir) = test_support::test_app("transfer-create-empty-file-row");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let mut parts = base_create_parts();
        parts.push(Text {
            name: "doc_label",
            value: "chat",
        });
        // An untouched `<input type="file">` row: present, but empty.
        parts.push(File {
            name: "doc_file",
            filename: "",
            bytes: b"",
        });

        let req = test_support::multipart_request_with_parts("/transfers", &cookie, &parts);
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get("location").unwrap(), "/transfers");
        let conn = state.conn();
        let transfers = qry::list(&conn).unwrap();
        assert_eq!(transfers.len(), 1);
        assert_eq!(
            documents_qry::list_for_record(&conn, "transfer", transfers[0].id)
                .unwrap()
                .len(),
            0
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn create_with_one_unknown_label_keeps_the_transfer_and_reports_the_failure() {
        use test_support::MultipartPart::{File, Text};
        let (state, dir) = test_support::test_app("transfer-create-one-unknown-label");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let mut parts = base_create_parts();
        parts.push(Text {
            name: "doc_label",
            value: "chat",
        });
        parts.push(File {
            name: "doc_file",
            filename: "chat.png",
            bytes: b"fake chat bytes",
        });
        parts.push(Text {
            name: "doc_label",
            value: "not-a-real-label",
        });
        parts.push(File {
            name: "doc_file",
            filename: "bad.png",
            bytes: b"fake bad bytes",
        });

        let req = test_support::multipart_request_with_parts("/transfers", &cookie, &parts);
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        assert!(body.contains("Unknown document label"));
        assert!(body.contains("Edit Transfer"));
        // The status list must name the file the user actually picked
        // (`chat.png`/`bad.png`), not the internal generated temp upload
        // path it was written to on the way to `service::attach_documents`.
        assert!(body.contains("chat.png"));
        assert!(body.contains("bad.png"));
        assert!(!body.contains("adm-sfa-web-upload-transfer-new-"));
        let conn = state.conn();
        let transfers = qry::list(&conn).unwrap();
        assert_eq!(transfers.len(), 1);
        assert_eq!(
            documents_qry::list_for_record(&conn, "transfer", transfers[0].id)
                .unwrap()
                .len(),
            1
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn create_with_an_invalid_date_rerenders_the_new_form_and_creates_nothing() {
        use test_support::MultipartPart::{File, Text};
        let (state, dir) = test_support::test_app("transfer-create-invalid-date");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let mut parts = base_create_parts();
        // Overrides the "date" part already pushed by base_create_parts —
        // the route's own accumulation logic just overwrites `date` on each
        // "date" field it sees, same as `purchases.rs`'s equivalent test.
        parts.push(Text {
            name: "date",
            value: "not-a-date",
        });
        parts.push(Text {
            name: "doc_label",
            value: "chat",
        });
        parts.push(File {
            name: "doc_file",
            filename: "chat.png",
            bytes: b"fake chat bytes",
        });

        let req = test_support::multipart_request_with_parts("/transfers", &cookie, &parts);
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        assert!(body.contains("New Transfer"));
        let conn = state.conn();
        assert_eq!(qry::list(&conn).unwrap().len(), 0);
        let doc_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM document", [], |row| row.get(0))
            .unwrap();
        assert_eq!(doc_count, 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn create_without_a_session_cookie_redirects_to_login() {
        let (state, dir) = test_support::test_app("transfer-create-unauthenticated");
        let app = crate::build_app(state.clone());

        let parts = base_create_parts();
        let req = test_support::multipart_request_with_parts("/transfers", "", &parts);
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get("location").unwrap(), "/login");
        let conn = state.conn();
        assert_eq!(qry::list(&conn).unwrap().len(), 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn preview_returns_the_translated_brl_amount_when_both_fields_parse() {
        let (state, dir) = test_support::test_app("transfer-preview-valid");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("GET")
            .uri("/transfers/preview?eur_amount_sent_str=1000&exchange_rate_str=5.5")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        assert!(body.contains("5,500.00") || body.contains("5.500,00"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn preview_returns_an_empty_body_when_the_amount_is_unparseable() {
        let (state, dir) = test_support::test_app("transfer-preview-invalid");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("GET")
            .uri("/transfers/preview?eur_amount_sent_str=not-a-number&exchange_rate_str=5.5")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        assert!(body.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Regression test: `brl_preview()` used to call `format::amount()` (the
    /// *ambient* locale, which `web` never sets) instead of
    /// `format::amount_in(amt, locale)` — mirrors `brl_ledger.rs`'s
    /// identical regression test. With `ui_locale` set to German, the
    /// previewed BRL amount must render with a comma decimal separator
    /// (`5.500,00`), not the English default (`5,500.00`).
    #[tokio::test]
    async fn preview_formats_the_brl_amount_using_the_resolved_ui_locale() {
        let (state, dir) = test_support::test_app("transfer-preview-locale-amount-format");
        adm_sfa_core::db::queries::settings::set(&state.conn(), "ui_locale", "de").unwrap();
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("GET")
            .uri("/transfers/preview?eur_amount_sent_str=1000&exchange_rate_str=5.5")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        assert!(
            body.contains("5.500,00"),
            "expected the German-formatted amount in the body: {body}"
        );
        assert!(
            !body.contains("5,500.00"),
            "amount rendered in English format despite ui_locale=de: {body}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn preview_without_a_session_cookie_redirects_to_login() {
        let (state, dir) = test_support::test_app("transfer-preview-unauthenticated");
        let app = crate::build_app(state.clone());

        let req = Request::builder()
            .method("GET")
            .uri("/transfers/preview?eur_amount_sent_str=1000&exchange_rate_str=5.5")
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get("location").unwrap(), "/login");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Regression coverage for CLAUDE.md's "Sorting" backlog item, mirroring
    /// `eur_ledger.rs`'s `list_shows_the_oldest_entry_first`: the list page
    /// shows the oldest transfer first, reversing `qry::list`'s newest-first
    /// SQL order in the web presentation layer only.
    #[tokio::test]
    async fn list_shows_the_oldest_transfer_first() {
        let (state, dir) = test_support::test_app("transfer-list-oldest-first");
        let conn = state.conn();
        qry::insert(
            &conn,
            &adm_sfa_core::model::transfer::TransferDraft {
                date: "2026-01-01".to_string(),
                eur_amount_sent_str: "111.11".to_string(),
                exchange_rate_str: "5.0".to_string(),
                notes: String::new(),
            },
        )
        .unwrap();
        qry::insert(
            &conn,
            &adm_sfa_core::model::transfer::TransferDraft {
                date: "2026-06-01".to_string(),
                eur_amount_sent_str: "222.22".to_string(),
                exchange_rate_str: "5.0".to_string(),
                notes: String::new(),
            },
        )
        .unwrap();
        drop(conn);
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("GET")
            .uri("/transfers")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        // `t.eur_display` (list.html) is the only column that distinguishes
        // the two rows — notes aren't rendered in the Transfers list table.
        let older_pos = body.find("111.11").expect("older amount not found");
        let newer_pos = body.find("222.22").expect("newer amount not found");
        assert!(
            older_pos < newer_pos,
            "expected the older transfer to render before the newer one"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Regression test: `list()` used to call `format::amount()`/
    /// `format::number()` (the *ambient* locale, which `web` never sets)
    /// for `eur_display`/`brl_display`/`rate_display` — mirrors
    /// `brl_ledger.rs`'s identical regression test. With `ui_locale` set to
    /// German, all three must render with a comma decimal separator, not
    /// the English default.
    #[tokio::test]
    async fn list_formats_amounts_and_rate_using_the_resolved_ui_locale() {
        let (state, dir) = test_support::test_app("transfer-list-locale-amount-format");
        let conn = state.conn();
        adm_sfa_core::db::queries::settings::set(&conn, "ui_locale", "de").unwrap();
        qry::insert(
            &conn,
            &adm_sfa_core::model::transfer::TransferDraft {
                date: "2026-01-01".to_string(),
                eur_amount_sent_str: "1000.00".to_string(),
                exchange_rate_str: "5.5".to_string(),
                notes: String::new(),
            },
        )
        .unwrap();
        drop(conn);
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("GET")
            .uri("/transfers")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        assert!(
            body.contains("1.000,00"),
            "expected the German-formatted EUR amount in the body: {body}"
        );
        assert!(
            body.contains("5.500,00"),
            "expected the German-formatted BRL amount in the body: {body}"
        );
        assert!(
            body.contains("5,5000"),
            "expected the German-formatted exchange rate in the body: {body}"
        );
        assert!(
            !body.contains("1,000.00") && !body.contains("5,500.00"),
            "an amount rendered in English format despite ui_locale=de: {body}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
