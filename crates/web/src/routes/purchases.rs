use std::path::PathBuf;
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
use adm_sfa_core::service::{self, PendingDocument};

use crate::state::AppState;
use crate::templates::{
    AttachResult, HtmlTemplate, PurchaseFormTemplate, PurchaseRow, PurchasesListTemplate,
};

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

/// Re-renders the edit form for handlers that fail (or partially fail)
/// after the point of no redirect-only return — document upload, document
/// removal, and a create-with-documents call whose purchase saved but not
/// every staged document attached (see `create`).
fn purchase_form_response(
    conn: &rusqlite::Connection,
    id: i64,
    error: Option<String>,
    attach_results: Vec<AttachResult>,
) -> Response {
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
        error,
        documents,
        labels,
        attach_results,
        locale,
    })
    .into_response()
}

/// Re-renders the edit form with an error banner and no attach-results —
/// the common case among `purchase_form_response`'s callers.
fn purchase_form_error_response(conn: &rusqlite::Connection, id: i64, error: String) -> Response {
    purchase_form_response(conn, id, Some(error), Vec::new())
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
    // `list` returns newest-first (shared with desktop, which relies on
    // that same order — reversed here, web-presentation-only, matching the
    // precedent set for the EUR Ledger list page).
    let rows = purchases
        .into_iter()
        .rev()
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
    // Populated (unlike before the create-with-documents form gained its own
    // repeatable label pickers): the create page now needs the same
    // allow-list as the edit page's attach form.
    let labels = documents_qry::labels(&conn).unwrap_or_default();
    HtmlTemplate(PurchaseFormTemplate {
        id: None,
        draft: PurchaseDraft::default(),
        error: None,
        documents: Vec::new(),
        is_negotiating: false,
        labels,
        attach_results: Vec::new(),
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
        attach_results: Vec::new(),
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

/// Creates a purchase and, in the same submission, attaches every document
/// staged in the "Documents?" section of the new-purchase form — a single
/// `multipart/form-data` POST rather than the urlencoded `Form<PurchaseForm>`
/// every other purchase-fields-only route uses, since a file can only travel
/// in a multipart body. Unlike `outbound.rs`'s `RawForm` workaround for
/// repeated form keys, `Multipart` streams fields one at a time regardless
/// of name repetition, so repeated `doc_label`/`doc_file` pairs need no
/// special handling here.
async fn create(State(state): State<AppState>, mut multipart: Multipart) -> impl IntoResponse {
    let mut date = String::new();
    let mut currency = String::new();
    let mut cost_str = String::new();
    let mut channel = String::new();
    let mut seller_info = String::new();
    let mut multiple_items: Option<String> = None;
    let mut negotiating: Option<String> = None;

    // Repeated `doc_label`/`doc_file` pairs, paired positionally (not by
    // index) — a `doc_label` sets the label for whichever `doc_file` follows
    // it, matching the row order the browser submits the form fields in.
    // The original filename is kept alongside the generated temp path
    // purely for display: `core::service::AttachmentOutcome::source_name`
    // is derived from whatever path it's handed, which would otherwise
    // surface the meaningless generated temp filename (not what the user
    // picked) in the partial-failure status list below.
    let mut last_label: Option<String> = None;
    let mut pending: Vec<(PathBuf, String, String)> = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        match field.name().unwrap_or("") {
            "date" => date = field.text().await.unwrap_or_default(),
            "currency" => currency = field.text().await.unwrap_or_default(),
            "cost_str" => cost_str = field.text().await.unwrap_or_default(),
            "channel" => channel = field.text().await.unwrap_or_default(),
            "seller_info" => seller_info = field.text().await.unwrap_or_default(),
            "multiple_items" => multiple_items = Some(field.text().await.unwrap_or_default()),
            "negotiating" => negotiating = Some(field.text().await.unwrap_or_default()),
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
                            "adm-sfa-web-upload-new-{}-{unique}.{ext}",
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

    let form = PurchaseForm {
        date,
        currency,
        cost_str,
        channel,
        seller_info,
        multiple_items,
        negotiating,
    };
    let status = if form.negotiating.is_some() {
        PurchaseStatus::Negotiating
    } else {
        PurchaseStatus::Bought
    };
    let draft = draft_from_form(form, status);

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
        service::create_purchase_with_documents(&conn, &state.documents_dir, &draft, &pending_docs);

    // Unconditional cleanup — the batch call above never keeps a temp path
    // around, whether it attached, failed, or was never reached because the
    // purchase itself failed to save.
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
            HtmlTemplate(PurchaseFormTemplate {
                id: None,
                draft,
                error: Some(error),
                documents: Vec::new(),
                is_negotiating: false,
                labels,
                attach_results: Vec::new(),
                locale,
            })
            .into_response()
        }
        Ok(created) if created.attachments.iter().all(|a| a.result.is_ok()) => {
            Redirect::to("/purchases").into_response()
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
            purchase_form_response(&conn, created.id, None, attach_results)
        }
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
                attach_results: Vec::new(),
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
                attach_results: Vec::new(),
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
                attach_results: Vec::new(),
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
    use axum::body::Body;
    use axum::http::{Request, StatusCode};

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

    /// Base purchase-fields-only parts for `create`'s multipart body,
    /// shared by the tests below — callers append `doc_label`/`doc_file`
    /// parts (or override a field) as needed.
    fn base_create_parts<'a>() -> Vec<test_support::MultipartPart<'a>> {
        use test_support::MultipartPart::Text;
        vec![
            Text {
                name: "date",
                value: "2026-01-01",
            },
            Text {
                name: "currency",
                value: "EUR",
            },
            Text {
                name: "cost_str",
                value: "50.00",
            },
            Text {
                name: "channel",
                value: "Kleinanzeigen",
            },
            Text {
                name: "seller_info",
                value: "",
            },
        ]
    }

    #[tokio::test]
    async fn create_with_two_documents_saves_the_purchase_and_both_files_then_redirects_to_the_list(
    ) {
        use test_support::MultipartPart::{File, Text};
        let (state, dir) = test_support::test_app("create-with-two-documents");
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

        let req = test_support::multipart_request_with_parts("/purchases", &cookie, &parts);
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get("location").unwrap(), "/purchases");
        let conn = state.conn();
        let purchases = purchases_qry::list(&conn).unwrap();
        assert_eq!(purchases.len(), 1);
        assert_eq!(
            documents_qry::list_for_record(&conn, "purchase", purchases[0].id)
                .unwrap()
                .len(),
            2
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn create_without_documents_still_redirects_to_the_list() {
        let (state, dir) = test_support::test_app("create-without-documents");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let parts = base_create_parts();
        let req = test_support::multipart_request_with_parts("/purchases", &cookie, &parts);
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get("location").unwrap(), "/purchases");
        let conn = state.conn();
        assert_eq!(purchases_qry::list(&conn).unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn create_with_an_empty_file_row_ignores_it() {
        use test_support::MultipartPart::{File, Text};
        let (state, dir) = test_support::test_app("create-empty-file-row");
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

        let req = test_support::multipart_request_with_parts("/purchases", &cookie, &parts);
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get("location").unwrap(), "/purchases");
        let conn = state.conn();
        let purchases = purchases_qry::list(&conn).unwrap();
        assert_eq!(purchases.len(), 1);
        assert_eq!(
            documents_qry::list_for_record(&conn, "purchase", purchases[0].id)
                .unwrap()
                .len(),
            0
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn create_with_one_unknown_label_keeps_the_purchase_and_reports_the_failure() {
        use test_support::MultipartPart::{File, Text};
        let (state, dir) = test_support::test_app("create-one-unknown-label");
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

        let req = test_support::multipart_request_with_parts("/purchases", &cookie, &parts);
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        assert!(body.contains("Unknown document label"));
        assert!(body.contains("Edit Purchase"));
        // The status list must name the file the user actually picked
        // (`chat.png`/`bad.png`), not the internal generated temp upload
        // path it was written to on the way to `service::attach_documents`.
        assert!(body.contains("chat.png"));
        assert!(body.contains("bad.png"));
        assert!(!body.contains("adm-sfa-web-upload-new-"));
        let conn = state.conn();
        let purchases = purchases_qry::list(&conn).unwrap();
        assert_eq!(purchases.len(), 1);
        assert_eq!(
            documents_qry::list_for_record(&conn, "purchase", purchases[0].id)
                .unwrap()
                .len(),
            1
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn create_with_an_invalid_date_rerenders_the_new_form_and_creates_nothing() {
        use test_support::MultipartPart::{File, Text};
        let (state, dir) = test_support::test_app("create-invalid-date");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let mut parts = base_create_parts();
        // Overrides the "date" part already pushed by base_create_parts —
        // the multipart parser keeps the last-written value for a repeated
        // field name only in the route's *own* accumulation logic (it just
        // overwrites `date` on each "date" field it sees), so appending a
        // second one has the same effect.
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

        let req = test_support::multipart_request_with_parts("/purchases", &cookie, &parts);
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        assert!(body.contains("New Purchase"));
        let conn = state.conn();
        assert_eq!(purchases_qry::list(&conn).unwrap().len(), 0);
        let doc_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM document", [], |row| row.get(0))
            .unwrap();
        assert_eq!(doc_count, 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn create_without_a_session_cookie_redirects_to_login() {
        let (state, dir) = test_support::test_app("create-unauthenticated");
        let app = crate::build_app(state.clone());

        let parts = base_create_parts();
        let req = test_support::multipart_request_with_parts("/purchases", "", &parts);
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get("location").unwrap(), "/login");
        let conn = state.conn();
        assert_eq!(purchases_qry::list(&conn).unwrap().len(), 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Regression coverage for CLAUDE.md's "Sorting" backlog item, mirroring
    /// `eur_ledger.rs`'s `list_shows_the_oldest_entry_first`.
    #[tokio::test]
    async fn list_shows_the_oldest_purchase_first() {
        let (state, dir) = test_support::test_app("purchases-list-oldest-first");
        let conn = state.conn();
        let mut older = a_purchase_draft();
        older.date = "2026-01-01".to_string();
        older.channel = "OlderChannel".to_string();
        purchases_qry::insert(&conn, &older).unwrap();
        let mut newer = a_purchase_draft();
        newer.date = "2026-06-01".to_string();
        newer.channel = "NewerChannel".to_string();
        purchases_qry::insert(&conn, &newer).unwrap();
        drop(conn);

        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("GET")
            .uri("/purchases")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        let older_pos = body.find("OlderChannel").expect("older channel not found");
        let newer_pos = body.find("NewerChannel").expect("newer channel not found");
        assert!(
            older_pos < newer_pos,
            "expected the older purchase to render before the newer one"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
