use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::{Multipart, Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Form;
use axum::Router;
use serde::Deserialize;

use adm_sfa_core::db::queries::{
    categories as cat_qry, documents as documents_qry, donors as donors_qry, inventory as qry,
    purchases as purchases_qry,
};
use adm_sfa_core::docs_fs;
use adm_sfa_core::format;
use adm_sfa_core::model::category::Category;
use adm_sfa_core::model::donor::PhysicalDonationDraft;
use adm_sfa_core::model::inventory::{
    InventoryItemDraft, InventoryItemRow, ItemStatus, Location, SourceType,
};
use adm_sfa_core::model::purchase::{Purchase, PurchaseStatus};
use adm_sfa_core::service::{self, PendingDocument};

use crate::routes::safe_return_to;
use crate::state::AppState;
use crate::templates::{
    AttachResult, CategoryOption, DonationOption, DonationRow, DonationsTemplate, HtmlTemplate,
    InventoryFormTemplate, InventoryListTemplate, InventoryRow, PurchaseOption,
};

/// Distinguishes concurrent uploads landing on the same temp path — same
/// pattern as `purchases.rs`'s `UPLOAD_COUNTER`.
static UPLOAD_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/inventory", get(list).post(create))
        .route("/inventory/new", get(new_form))
        .route("/inventory/{id}/edit", get(edit_form))
        .route("/inventory/{id}", post(update))
        .route("/inventory/{id}/documents", post(attach_document))
        .route(
            "/inventory/{id}/documents/{doc_id}/delete",
            post(remove_document),
        )
        .route("/inventory/donations", get(donations).post(create_donation))
}

/// Same keys `core::model::inventory::Location::label()` uses, but with an
/// explicit locale — see `eur_ledger.rs`'s `type_label` doc comment.
fn location_label(l: Location, locale: &str) -> String {
    let key = match l {
        Location::Germany => "status.location.germany",
        Location::Brazil => "status.location.brazil",
    };
    rust_i18n::t!(key, locale = locale).to_string()
}

/// Same keys `core::model::inventory::ItemStatus::label()` uses, but with
/// an explicit locale — see `eur_ledger.rs`'s `type_label` doc comment.
fn status_label(s: ItemStatus, locale: &str) -> String {
    let key = match s {
        ItemStatus::Available => "status.item.available",
        ItemStatus::Reserved => "status.item.reserved",
        ItemStatus::Donated => "status.item.donated",
    };
    rust_i18n::t!(key, locale = locale).to_string()
}

fn purchase_label(p: &Purchase, locale: &str) -> String {
    let multi = if p.multiple_items { " (multi)" } else { "" };
    format!(
        "{} \u{2014} {} \u{2014} {} {}{multi}",
        format::date(&p.date),
        p.channel,
        p.currency.symbol(),
        format::amount_in(p.cost, locale)
    )
}

/// A single-item purchase (`multiple_items = false`) already linked to a
/// *different* inventory item than `edit_id` — mirrors desktop's
/// `InventoryView::purchase_source_blocked`, reading the same two
/// already-fetched lists rather than issuing one query per candidate
/// purchase. `inventory::purchase_source_conflict` in core is what's
/// actually authoritative at insert/update time; this is grey-out only.
fn purchase_source_blocked(items: &[InventoryItemRow], p: &Purchase, edit_id: Option<i64>) -> bool {
    if p.multiple_items {
        return false;
    }
    items
        .iter()
        .any(|item| edit_id != Some(item.id) && item.source_purchase_id == Some(p.id))
}

fn category_options(categories: &[Category], selected: Option<i64>) -> Vec<CategoryOption> {
    categories
        .iter()
        .map(|c| CategoryOption {
            id: c.id,
            name: c.name.clone(),
            selected: selected == Some(c.id),
        })
        .collect()
}

fn purchase_options(
    purchases: &[Purchase],
    items: &[InventoryItemRow],
    edit_id: Option<i64>,
    selected: Option<i64>,
    locale: &str,
) -> Vec<PurchaseOption> {
    purchases
        .iter()
        .filter(|p| p.status == PurchaseStatus::Bought)
        .map(|p| {
            let blocked = purchase_source_blocked(items, p, edit_id);
            let base_label = purchase_label(p, locale);
            // The blocked/"(in use)" suffix has to be baked into the
            // `<option>` text here rather than in the template: a disabled
            // `<option>` still needs to explain *why* it's unpickable, and
            // the template only gets `label` as one opaque string.
            let label = if blocked {
                rust_i18n::t!(
                    "web.inventory.purchase_combo.in_use",
                    locale = locale,
                    label = &base_label
                )
                .to_string()
            } else {
                base_label
            };
            PurchaseOption {
                id: p.id,
                label,
                selected: selected == Some(p.id),
                blocked,
            }
        })
        .collect()
}

fn donation_label(d: &adm_sfa_core::model::donor::PhysicalDonation, locale: &str) -> String {
    match &d.donor_name {
        Some(name) => format!("{} \u{2014} {name}", format::date(&d.date_received)),
        None => {
            let anonymous = rust_i18n::t!("common.anonymous", locale = locale);
            format!("{} \u{2014} {anonymous}", format::date(&d.date_received))
        }
    }
}

fn donation_options(
    donations: &[adm_sfa_core::model::donor::PhysicalDonation],
    selected: Option<i64>,
    locale: &str,
) -> Vec<DonationOption> {
    donations
        .iter()
        .map(|d| DonationOption {
            id: d.id,
            label: donation_label(d, locale),
            selected: selected == Some(d.id),
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn form_template(
    conn: &rusqlite::Connection,
    id: Option<i64>,
    draft: &InventoryItemDraft,
    error: Option<String>,
    documents: Vec<adm_sfa_core::model::document::Document>,
    locale: String,
    // Overrides which Source radio renders `checked`, independent of
    // `draft.source_type` (a required enum with no "unset" variant) — the
    // mandatory/unselected-by-default behaviour on the New Item form needs
    // to represent "no source chosen yet", which the enum itself can't.
    // `Some("")` renders neither radio checked (fresh New Item form, or a
    // rejected submission with a missing/invalid source); `None` uses
    // `draft.source_type` as before (Edit form's persisted value, or a
    // validation error unrelated to the source type, which should keep
    // whatever was actually submitted checked). Mirrors `eur_ledger.rs`'s
    // `donation_checked`/`self_funding_checked`, just as one overridable
    // string instead of two bools since this template already compares
    // `source_type` directly rather than using per-radio bools.
    source_type_override: Option<&str>,
    // See `PurchaseFormTemplate::attach_results`. Empty on every render
    // path except a create-with-documents submission where the item
    // saved but at least one staged document failed to attach.
    attach_results: Vec<AttachResult>,
) -> InventoryFormTemplate {
    let categories = cat_qry::list(conn).unwrap_or_default();
    let donations = donors_qry::list_donations(conn).unwrap_or_default();
    let purchases = purchases_qry::list(conn).unwrap_or_default();
    let items = qry::list(conn).unwrap_or_default();
    let labels = documents_qry::labels(conn).unwrap_or_default();
    // The *persisted* status, not `draft.status` — a rejected attempt to
    // submit `status` away from `donated` must not visually unlock the form
    // before the next page load just because the rejected draft said
    // otherwise (`core::db::queries::inventory::update` is what actually
    // enforces the lock; this only has to match it, not re-derive it from
    // untrusted input).
    let locked = id
        .and_then(|id| qry::get(conn, id).ok().flatten())
        .is_some_and(|item| item.status == ItemStatus::Donated);

    let source_type = source_type_override
        .map(|s| s.to_string())
        .unwrap_or_else(|| draft.source_type.as_str().to_string());

    InventoryFormTemplate {
        id,
        name: draft.name.clone(),
        location: draft.location.as_str().to_string(),
        status: draft.status.as_str().to_string(),
        source_type,
        category_id: draft.category_id,
        source_donation_id: draft.source_donation_id,
        source_purchase_id: draft.source_purchase_id,
        categories: category_options(&categories, draft.category_id),
        donations: donation_options(&donations, draft.source_donation_id, &locale),
        purchases: purchase_options(&purchases, &items, id, draft.source_purchase_id, &locale),
        notes: draft.notes.clone(),
        error,
        documents,
        labels,
        locked,
        attach_results,
        locale,
    }
}

/// Re-fetches the persisted item and re-renders its edit form — same
/// pattern as `purchases.rs::purchase_form_response`. Used for a document
/// upload/removal error (no in-flight draft of its own — a document-only
/// action, not a full item edit), and for a create-with-documents
/// submission whose item saved but not every staged document attached (see
/// `create`).
fn item_form_response(
    conn: &rusqlite::Connection,
    id: i64,
    error: Option<String>,
    attach_results: Vec<AttachResult>,
) -> Response {
    let Some(item) = qry::get(conn, id).ok().flatten() else {
        return (axum::http::StatusCode::NOT_FOUND, "item not found").into_response();
    };
    let documents = documents_qry::list_for_record(conn, "item", id).unwrap_or_default();
    let draft = InventoryItemDraft {
        name: item.name,
        category_id: Some(item.category_id),
        source_type: item.source_type,
        source_donation_id: item.source_donation_id,
        source_purchase_id: item.source_purchase_id,
        location: item.location,
        status: item.status,
        notes: item.notes.unwrap_or_default(),
    };
    let locale = crate::i18n::resolve_locale(conn);
    HtmlTemplate(form_template(
        conn,
        Some(id),
        &draft,
        error,
        documents,
        locale,
        None,
        attach_results,
    ))
    .into_response()
}

/// Re-renders the edit form with an error banner and no attach-results —
/// the common case among `item_form_response`'s callers.
fn item_form_error_response(conn: &rusqlite::Connection, id: i64, error: String) -> Response {
    item_form_response(conn, id, Some(error), Vec::new())
}

async fn list(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let items = qry::list(&conn).unwrap_or_default();
    let rows = items
        .into_iter()
        .map(|i| InventoryRow {
            id: i.id,
            name: i.name,
            category_name: i.category_name,
            location_label: location_label(i.location, &locale),
            status_label: status_label(i.status, &locale),
            source_desc: i.source_desc,
        })
        .collect();
    HtmlTemplate(InventoryListTemplate {
        items: rows,
        locale,
    })
}

/// Query params the "+ New donation" round trip comes back with — the New
/// Item form's own JS populates these onto the link's `return_to` on the
/// way out (`inventory/form.html`'s `updateNewDonationLink`), and
/// `create_donation` appends `donation_id` on the way back in, mirroring
/// `eur_ledger.rs::new_form`'s `NewEntryQuery`/`donor_id` handling. Absent
/// entirely on a normal fresh `/inventory/new` visit, in which case every
/// field below is empty and `donation_id` is `None` — same as
/// `InventoryItemDraft::default()` used to render directly.
#[derive(Deserialize)]
struct NewItemQuery {
    #[serde(default)]
    name: String,
    #[serde(default)]
    category_id: String,
    #[serde(default)]
    location: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    notes: String,
    donation_id: Option<i64>,
}

async fn new_form(
    State(state): State<AppState>,
    Query(query): Query<NewItemQuery>,
) -> impl IntoResponse {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    // Validate the id actually resolves before trusting it — mirrors
    // `eur_ledger.rs::new_form`'s `donor_id` check: a stale link or a
    // hand-edited query string with a nonexistent `donation_id` would
    // otherwise still force the Source radio to Donation around a dropdown
    // selection that doesn't exist.
    let donations = donors_qry::list_donations(&conn).unwrap_or_default();
    let donation_id = query
        .donation_id
        .filter(|id| donations.iter().any(|d| d.id == *id));
    let draft = InventoryItemDraft {
        name: query.name,
        category_id: parsed_id(&query.category_id),
        source_type: SourceType::Donation,
        source_donation_id: donation_id,
        source_purchase_id: None,
        location: Location::from_str(&query.location).unwrap_or(Location::Germany),
        status: ItemStatus::from_str(&query.status).unwrap_or(ItemStatus::Available),
        notes: query.notes,
    };
    // `Some("")` keeps Source unselected for a genuinely fresh visit (no
    // `donation_id` at all — the New Item form's usual mandatory,
    // nothing-checked-yet state); `None` falls back to `draft.source_type`
    // (forced to `Donation` above) once a valid `donation_id` comes back
    // from the round trip, so that radio renders checked and its dropdown
    // shows the newly created donation pre-selected.
    let source_type_override = if donation_id.is_some() {
        None
    } else {
        Some("")
    };
    HtmlTemplate(form_template(
        &conn,
        None,
        &draft,
        None,
        Vec::new(),
        locale,
        source_type_override,
        Vec::new(),
    ))
}

async fn edit_form(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let Some(item) = qry::get(&conn, id).ok().flatten() else {
        return (axum::http::StatusCode::NOT_FOUND, "item not found").into_response();
    };
    let documents = documents_qry::list_for_record(&conn, "item", id).unwrap_or_default();
    let draft = InventoryItemDraft {
        name: item.name,
        category_id: Some(item.category_id),
        source_type: item.source_type,
        source_donation_id: item.source_donation_id,
        source_purchase_id: item.source_purchase_id,
        location: item.location,
        status: item.status,
        notes: item.notes.unwrap_or_default(),
    };
    HtmlTemplate(form_template(
        &conn,
        Some(id),
        &draft,
        None,
        documents,
        locale,
        None,
        Vec::new(),
    ))
    .into_response()
}

#[derive(Deserialize)]
struct InventoryForm {
    name: String,
    category_id: String,
    location: String,
    status: String,
    /// Missing entirely (not just empty) when a crafted POST omits the
    /// field outright — `#[serde(default)]` so `axum::Form` still
    /// deserializes instead of rejecting the request with a raw 422 before
    /// `create()`/`update()` ever run their own mandatory-source check and
    /// can render a translated, in-form error banner instead.
    #[serde(default)]
    source_type: String,
    #[serde(default)]
    source_donation_id: String,
    #[serde(default)]
    source_purchase_id: String,
    #[serde(default)]
    notes: String,
}

fn parsed_id(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        s.parse().ok()
    }
}

/// `source_type` is taken already-parsed rather than derived from
/// `form.source_type` here — the mandatory-source check now happens
/// authoritatively in `create()`/`update()` *before* this is called,
/// mirroring `eur_ledger.rs::create()`'s `tx_type` match. This function no
/// longer has a silent fallback for a missing/invalid source: that used to
/// default to `SourceType::Donation`, exactly the bug class the eur-ledger
/// mandatory-Typ fix closed (a crafted or bugged submission with no source
/// would previously become a Donation-sourced item without the user ever
/// choosing that).
fn draft_from_form(form: InventoryForm, source_type: SourceType) -> InventoryItemDraft {
    let (source_donation_id, source_purchase_id) = match source_type {
        SourceType::Donation => (parsed_id(&form.source_donation_id), None),
        SourceType::Purchase => (None, parsed_id(&form.source_purchase_id)),
    };
    InventoryItemDraft {
        name: form.name,
        category_id: parsed_id(&form.category_id),
        source_type,
        source_donation_id,
        source_purchase_id,
        location: Location::from_str(&form.location).unwrap_or(Location::Germany),
        status: ItemStatus::from_str(&form.status).unwrap_or(ItemStatus::Available),
        notes: form.notes,
    }
}

/// Creates an inventory item and, in the same submission, attaches every
/// document staged in the "Documents?" section of the New Item form — a
/// single `multipart/form-data` POST rather than the urlencoded
/// `Form<InventoryForm>` `update()` still uses, since a file can only
/// travel in a multipart body. Mirrors `purchases.rs::create` and
/// `transfers.rs::create` field-for-field (repeated `doc_label`/`doc_file`
/// pairs paired positionally, not by index).
async fn create(State(state): State<AppState>, mut multipart: Multipart) -> Response {
    let mut name = String::new();
    let mut category_id = String::new();
    let mut location = String::new();
    let mut status = String::new();
    let mut source_type_str = String::new();
    let mut source_donation_id = String::new();
    let mut source_purchase_id = String::new();
    let mut notes = String::new();

    let mut last_label: Option<String> = None;
    let mut pending: Vec<(PathBuf, String, String)> = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        match field.name().unwrap_or("") {
            "name" => name = field.text().await.unwrap_or_default(),
            "category_id" => category_id = field.text().await.unwrap_or_default(),
            "location" => location = field.text().await.unwrap_or_default(),
            "status" => status = field.text().await.unwrap_or_default(),
            "source_type" => source_type_str = field.text().await.unwrap_or_default(),
            "source_donation_id" => source_donation_id = field.text().await.unwrap_or_default(),
            "source_purchase_id" => source_purchase_id = field.text().await.unwrap_or_default(),
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
                            "adm-sfa-web-upload-item-new-{}-{unique}.{ext}",
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

    let form = InventoryForm {
        name,
        category_id,
        location,
        status,
        source_type: source_type_str,
        source_donation_id,
        source_purchase_id,
        notes,
    };

    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let Some(source_type) = SourceType::from_str(&form.source_type) else {
        // `SourceType::Donation` here is a placeholder purely so the rest of
        // what was typed (name, category, location, status, notes) can
        // still be echoed back — `Some("")` below is what actually makes
        // neither Source radio render checked, same as a fresh New Item
        // form.
        let draft = draft_from_form(form, SourceType::Donation);
        let mut error =
            rust_i18n::t!("web.inventory.error.source_type_required", locale = &locale).to_string();
        // A browser cannot repopulate a file input for security reasons, so
        // any staged files are unavoidably lost on this path — tell the
        // user rather than silently dropping them, same as a later
        // create-with-documents failure below.
        if !pending.is_empty() {
            let notice =
                rust_i18n::t!("web.doc.notice.reselect_after_error", locale = &locale).to_string();
            error = format!("{error} {notice}");
        }
        for (path, _, _) in &pending {
            let _ = std::fs::remove_file(path);
        }
        return HtmlTemplate(form_template(
            &conn,
            None,
            &draft,
            Some(error),
            Vec::new(),
            locale,
            Some(""),
            Vec::new(),
        ))
        .into_response();
    };
    let draft = draft_from_form(form, source_type);

    let pending_docs: Vec<PendingDocument> = pending
        .iter()
        .map(|(path, label, _)| PendingDocument {
            path: path.as_path(),
            label: label.as_str(),
        })
        .collect();

    let result =
        service::create_item_with_documents(&conn, &state.documents_dir, &draft, &pending_docs);

    // Unconditional cleanup — the batch call above never keeps a temp path
    // around, whether it attached, failed, or was never reached because the
    // item itself failed to save.
    for (path, _, _) in &pending {
        let _ = std::fs::remove_file(path);
    }

    match result {
        Err(e) => {
            let mut error = e.to_string();
            if !pending.is_empty() {
                let notice = rust_i18n::t!("web.doc.notice.reselect_after_error", locale = &locale)
                    .to_string();
                error = format!("{error} {notice}");
            }
            HtmlTemplate(form_template(
                &conn,
                None,
                &draft,
                Some(error),
                Vec::new(),
                locale,
                None,
                Vec::new(),
            ))
            .into_response()
        }
        Ok(created) if created.attachments.iter().all(|a| a.result.is_ok()) => {
            Redirect::to("/inventory").into_response()
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
            item_form_response(&conn, created.id, None, attach_results)
        }
    }
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<InventoryForm>,
) -> Response {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let Some(source_type) = SourceType::from_str(&form.source_type) else {
        let documents = documents_qry::list_for_record(&conn, "item", id).unwrap_or_default();
        // Same placeholder-draft-plus-override reasoning as `create()`'s
        // equivalent branch.
        let draft = draft_from_form(form, SourceType::Donation);
        let error =
            rust_i18n::t!("web.inventory.error.source_type_required", locale = &locale).to_string();
        return HtmlTemplate(form_template(
            &conn,
            Some(id),
            &draft,
            Some(error),
            documents,
            locale,
            Some(""),
            Vec::new(),
        ))
        .into_response();
    };
    let draft = draft_from_form(form, source_type);
    match qry::update(&conn, id, &draft) {
        Ok(()) => Redirect::to(&format!("/inventory/{id}/edit")).into_response(),
        Err(e) => {
            let documents = documents_qry::list_for_record(&conn, "item", id).unwrap_or_default();
            // A donated item only ever allows `notes` to change — if that's
            // why this was rejected, re-render from what's actually
            // persisted (not the rejected submission) so the hidden inputs
            // carry true values and a notes-only retry can still succeed.
            // Re-rendering the rejected draft verbatim here (as any other
            // validation error does, to preserve in-progress edits) would
            // instead bake the same mismatched values into the hidden
            // inputs, making every retry fail the same way with no visible
            // way out short of a full page reload.
            let persisted = qry::get(&conn, id).ok().flatten();
            let is_locked_rejection = persisted
                .as_ref()
                .is_some_and(|item| item.status == ItemStatus::Donated);
            let render_draft = if is_locked_rejection {
                match persisted {
                    Some(item) => InventoryItemDraft {
                        name: item.name,
                        category_id: Some(item.category_id),
                        source_type: item.source_type,
                        source_donation_id: item.source_donation_id,
                        source_purchase_id: item.source_purchase_id,
                        location: item.location,
                        status: item.status,
                        notes: draft.notes.clone(),
                    },
                    None => draft.clone(),
                }
            } else {
                draft.clone()
            };
            HtmlTemplate(form_template(
                &conn,
                Some(id),
                &render_draft,
                Some(e.to_string()),
                documents,
                locale,
                None,
                Vec::new(),
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
                            "adm-sfa-web-upload-item-{id}-{}-{unique}.{ext}",
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
        return item_form_error_response(&conn, id, error);
    };

    let conn = state.conn();
    // `document.record_id` has no FK (schema.sql: record identity is
    // enforced in application code, not SQL) — without this check a POST to
    // a nonexistent item id would still copy the file and insert a
    // permanently orphaned document row that no UI could ever list or
    // soft-delete, since no item exists to look it up against.
    if qry::get(&conn, id).ok().flatten().is_none() {
        let _ = std::fs::remove_file(&tmp_path);
        return (axum::http::StatusCode::NOT_FOUND, "item not found").into_response();
    }
    // Items have no single "date" field of their own — an empty draft input
    // and no persisted date both fall through `service::attach_document`'s
    // fallback chain to today's date, matching desktop's own call here.
    let existing: Vec<String> = documents_qry::list_for_record(&conn, "item", id)
        .unwrap_or_default()
        .into_iter()
        .map(|d| d.filename)
        .collect();

    let result = service::attach_document(
        &conn,
        &state.documents_dir,
        &tmp_path,
        "",
        None,
        ("item", id),
        &label,
        &existing,
    );
    let _ = std::fs::remove_file(&tmp_path);

    match result {
        Ok(_) => Redirect::to(&format!("/inventory/{id}/edit")).into_response(),
        Err(e) => item_form_error_response(&conn, id, e),
    }
}

async fn remove_document(
    State(state): State<AppState>,
    Path((id, doc_id)): Path<(i64, i64)>,
) -> Response {
    let conn = state.conn();
    let Some(doc) = documents_qry::list_for_record(&conn, "item", id)
        .unwrap_or_default()
        .into_iter()
        .find(|d| d.id == doc_id)
    else {
        return Redirect::to(&format!("/inventory/{id}/edit")).into_response();
    };
    match docs_fs::remove_document(&conn, &state.documents_dir, doc.id, &doc.filename) {
        Ok(()) => Redirect::to(&format!("/inventory/{id}/edit")).into_response(),
        Err(e) => item_form_error_response(&conn, id, e),
    }
}

/// Physical donation records (`physical_donation`) have no dedicated
/// section anywhere in desktop either — they're only ever created inline
/// from Inventory's "+ New donation" sub-form. `web` has no equivalent
/// inline sub-form (no JS to toggle a nested group without a page reload,
/// and a plain HTML form can't nest one form inside another), so this is a
/// standalone create-only mini page instead: create a donation record here
/// first, then it appears in the Donation dropdown back on the item form.
///
/// Round-trips two levels deep: the item form's "+ New donation" link
/// carries its own in-progress fields via `return_to`
/// (`new_form`'s `NewItemQuery`), and this page's own "+ New donor" link
/// (`donations.html`'s `updateNewDonorLink`) carries *its* in-progress
/// fields (`date_received`/`notes`) plus that same incoming `return_to`
/// onward to `/donors/new` — so `donor_id`, once created there, flows all
/// the way back here via `donors.rs::create`'s already-generic `return_to`
/// handling (no changes needed there), and the item page's own fields are
/// never lost in between. Mirrors `eur_ledger.rs`'s `NewEntryQuery`/
/// `donor_options`.
#[derive(Deserialize)]
struct DonationsQuery {
    #[serde(default)]
    return_to: Option<String>,
    #[serde(default)]
    date_received: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    donor_id: Option<i64>,
}

/// `(id, name, selected)` — mirrors `eur_ledger.rs::donor_options`.
fn donor_options(conn: &rusqlite::Connection, selected: Option<i64>) -> Vec<(i64, String, bool)> {
    donors_qry::list(conn)
        .unwrap_or_default()
        .into_iter()
        .map(|d| {
            let is_selected = selected == Some(d.id);
            (d.id, d.name, is_selected)
        })
        .collect()
}

async fn donations(
    State(state): State<AppState>,
    Query(query): Query<DonationsQuery>,
) -> impl IntoResponse {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let anonymous = rust_i18n::t!("common.anonymous", locale = &locale).to_string();
    let donations = donors_qry::list_donations(&conn).unwrap_or_default();
    let rows = donations
        .into_iter()
        .map(|d| DonationRow {
            date_display: format::date(&d.date_received),
            donor_display: d.donor_name.unwrap_or_else(|| anonymous.clone()),
        })
        .collect();
    // Validate the id actually resolves before trusting it — mirrors
    // `eur_ledger.rs::new_form`'s `donor_id` check: a stale link or a
    // hand-edited query string with a nonexistent donor_id would otherwise
    // silently attribute the donation to the browser's default (usually
    // first) option instead of leaving the selection visibly unset.
    let donor_id = query
        .donor_id
        .filter(|id| donors_qry::get(&conn, *id).ok().flatten().is_some());
    HtmlTemplate(DonationsTemplate {
        donations: rows,
        date: query
            .date_received
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string()),
        notes: query.notes.unwrap_or_default(),
        donors: donor_options(&conn, donor_id),
        error: None,
        return_to: query.return_to.filter(|s| safe_return_to(s)),
        locale,
    })
}

#[derive(Deserialize)]
struct DonationForm {
    date_received: String,
    #[serde(default)]
    donor_id: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    return_to: String,
}

async fn create_donation(
    State(state): State<AppState>,
    Form(form): Form<DonationForm>,
) -> Response {
    let return_to = form.return_to;
    let draft = PhysicalDonationDraft {
        donor_id: parsed_id(&form.donor_id),
        date_received: form.date_received,
        notes: form.notes,
    };
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    match donors_qry::insert_donation(&conn, &draft) {
        Ok(id) => {
            if safe_return_to(&return_to) {
                // Assumes return_to carries no #fragment (none of today's
                // callers emit one) — appending a query after a fragment
                // would produce a syntactically-wrong-order URL. Mirrors
                // `donors.rs::create`.
                let sep = if return_to.contains('?') { '&' } else { '?' };
                Redirect::to(&format!("{return_to}{sep}donation_id={id}")).into_response()
            } else {
                // No caller-supplied return path (this page's own nav
                // entry) or an unsafe one (rejected above, falls back here
                // too) — either way lands on the donations list itself,
                // matching the pre-existing behavior for every visit that
                // isn't a "+ New donation" round trip.
                Redirect::to("/inventory/donations").into_response()
            }
        }
        Err(e) => {
            let anonymous = rust_i18n::t!("common.anonymous", locale = &locale).to_string();
            let donations = donors_qry::list_donations(&conn).unwrap_or_default();
            let rows = donations
                .into_iter()
                .map(|d| DonationRow {
                    date_display: format::date(&d.date_received),
                    donor_display: d.donor_name.unwrap_or_else(|| anonymous.clone()),
                })
                .collect();
            HtmlTemplate(DonationsTemplate {
                donations: rows,
                date: draft.date_received,
                notes: draft.notes,
                donors: donor_options(&conn, draft.donor_id),
                error: Some(e.to_string()),
                return_to: Some(return_to).filter(|s| safe_return_to(s)),
                locale,
            })
            .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support;
    use adm_sfa_core::db::queries::donors as donors_qry;
    use adm_sfa_core::db::queries::{
        categories as cat_qry, documents as documents_qry, inventory as qry,
        purchases as purchases_qry,
    };
    use adm_sfa_core::model::donor::{DonorDraft, PhysicalDonationDraft};
    use adm_sfa_core::model::inventory::{InventoryItemDraft, ItemStatus, Location, SourceType};
    use adm_sfa_core::model::purchase::{Currency, PurchaseDraft, PurchaseStatus};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};

    /// Sets up a donated item directly through `core` (not through the web
    /// form) — this is regression coverage for the donated-item field-lock
    /// backlog item, not for the create/insert path itself.
    fn setup_donated_item(conn: &rusqlite::Connection) -> (i64, i64) {
        let cat_id = cat_qry::insert(conn, "Decks").unwrap();
        let purchase_id = purchases_qry::insert(
            conn,
            &PurchaseDraft {
                date: "2026-01-01".to_string(),
                currency: Currency::Eur,
                cost_str: "50.00".to_string(),
                channel: "Kleinanzeigen".to_string(),
                seller_info: String::new(),
                multiple_items: false,
                status: PurchaseStatus::Bought,
            },
        )
        .unwrap();
        let item_id = qry::insert(
            conn,
            &InventoryItemDraft {
                name: "Deck".to_string(),
                category_id: Some(cat_id),
                source_type: SourceType::Purchase,
                source_donation_id: None,
                source_purchase_id: Some(purchase_id),
                location: Location::Germany,
                status: ItemStatus::Available,
                notes: String::new(),
            },
        )
        .unwrap();
        // Transition to donated through a real update — the item starts
        // editable, and this is the only legitimate way to reach `donated`.
        qry::update(
            conn,
            item_id,
            &InventoryItemDraft {
                name: "Deck".to_string(),
                category_id: Some(cat_id),
                source_type: SourceType::Purchase,
                source_donation_id: None,
                source_purchase_id: Some(purchase_id),
                location: Location::Germany,
                status: ItemStatus::Donated,
                notes: String::new(),
            },
        )
        .unwrap();
        (item_id, cat_id)
    }

    fn update_form_body(
        name: &str,
        category_id: i64,
        location: &str,
        status: &str,
        source_type: &str,
        source_purchase_id: i64,
        notes: &str,
    ) -> String {
        format!(
            "name={name}&category_id={category_id}&location={location}&status={status}\
             &source_type={source_type}&source_purchase_id={source_purchase_id}&notes={notes}"
        )
    }

    #[tokio::test]
    async fn editing_a_locked_field_on_a_donated_item_is_rejected() {
        let (state, dir) = test_support::test_app("inventory-locked-field-rejected");
        let (item_id, cat_id) = setup_donated_item(&state.conn());
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body = update_form_body(
            "Renamed deck", // name changed — locked
            cat_id,
            "germany",
            "donated",
            "purchase",
            1,
            "",
        );
        let req = Request::builder()
            .method("POST")
            .uri(format!("/inventory/{item_id}"))
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body_text = test_support::body_text(res).await;
        assert!(body_text.contains("already been donated"));
        assert_eq!(
            qry::get(&state.conn(), item_id).unwrap().unwrap().name,
            "Deck"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Regression test for a bug the reviewer caught before this branch was
    /// committed: the rejected-update re-render used to bake the *rejected*
    /// submission's values into the locked fields' hidden inputs (not the
    /// true persisted values), so a real browser doing exactly what the
    /// error banner invites — leave the page as shown, fix notes, resubmit
    /// — would keep failing for the same reason forever, with no visible
    /// way out short of a full page reload.
    #[tokio::test]
    async fn a_rejected_update_rerenders_locked_fields_from_persisted_values_not_the_rejected_submission(
    ) {
        let (state, dir) = test_support::test_app("inventory-locked-rerender-uses-persisted");
        let (item_id, cat_id) = setup_donated_item(&state.conn());
        let other_cat_id = cat_qry::insert(&state.conn(), "Wheels").unwrap();
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        // Submit a bad category_id alongside a notes change — rejected.
        let body = update_form_body(
            "Deck",
            other_cat_id,
            "germany",
            "donated",
            "purchase",
            1,
            "attempted while also changing category",
        );
        let req = Request::builder()
            .method("POST")
            .uri(format!("/inventory/{item_id}"))
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app.clone(), req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body_text = test_support::body_text(res).await;
        // The re-rendered hidden input must carry the *true* persisted
        // category, not the bad one that was just rejected.
        assert!(body_text.contains(&format!("name=\"category_id\" value=\"{cat_id}\"")));
        assert!(!body_text.contains(&format!("name=\"category_id\" value=\"{other_cat_id}\"")));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn notes_can_still_be_updated_on_a_donated_item() {
        let (state, dir) = test_support::test_app("inventory-locked-notes-allowed");
        let (item_id, cat_id) = setup_donated_item(&state.conn());
        let purchase_id = qry::get(&state.conn(), item_id)
            .unwrap()
            .unwrap()
            .source_purchase_id
            .unwrap();
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body = update_form_body(
            "Deck",
            cat_id,
            "germany",
            "donated",
            "purchase",
            purchase_id,
            "handled with care",
        );
        let req = Request::builder()
            .method("POST")
            .uri(format!("/inventory/{item_id}"))
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            qry::get(&state.conn(), item_id)
                .unwrap()
                .unwrap()
                .notes
                .as_deref(),
            Some("handled with care")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn edit_form_shows_the_donated_lock_notice() {
        let (state, dir) = test_support::test_app("inventory-locked-form-render");
        let (item_id, _cat_id) = setup_donated_item(&state.conn());
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .uri(format!("/inventory/{item_id}/edit"))
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body_text = test_support::body_text(res).await;
        assert!(body_text.contains("has been donated"));
        assert!(body_text.contains("disabled"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A purchase-sourced item to POST a create/update against — mirrors
    /// `setup_donated_item`'s purchase setup, minus the donated transition.
    fn setup_purchase(conn: &rusqlite::Connection) -> (i64, i64) {
        let cat_id = cat_qry::insert(conn, "Decks").unwrap();
        let purchase_id = purchases_qry::insert(
            conn,
            &PurchaseDraft {
                date: "2026-01-01".to_string(),
                currency: Currency::Eur,
                cost_str: "50.00".to_string(),
                channel: "Kleinanzeigen".to_string(),
                seller_info: String::new(),
                multiple_items: true,
                status: PurchaseStatus::Bought,
            },
        )
        .unwrap();
        (cat_id, purchase_id)
    }

    /// Base item-fields-only parts for `create`'s multipart body, shared by
    /// the tests below — mirrors `purchases.rs`'s `base_create_parts`.
    /// `name`/`category_id_str` are owned by the caller (not `String`
    /// literals baked in here) since every test needs a distinct
    /// `category_id`.
    fn base_create_parts<'a>(
        name: &'a str,
        category_id_str: &'a str,
    ) -> Vec<test_support::MultipartPart<'a>> {
        use test_support::MultipartPart::Text;
        vec![
            Text {
                name: "name",
                value: name,
            },
            Text {
                name: "category_id",
                value: category_id_str,
            },
            Text {
                name: "location",
                value: "germany",
            },
            Text {
                name: "status",
                value: "available",
            },
            Text {
                name: "notes",
                value: "",
            },
        ]
    }

    /// A valid `source_type` should still redirect and insert a row —
    /// guards the happy path through `create()`'s new early-return match,
    /// mirroring `eur_ledger.rs`'s equivalent regression test.
    #[tokio::test]
    async fn create_with_a_valid_source_type_redirects_and_inserts_a_row() {
        use test_support::MultipartPart::Text;
        let (state, dir) = test_support::test_app("inventory-create-valid-source-type");
        let (cat_id, purchase_id) = setup_purchase(&state.conn());
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let cat_id_str = cat_id.to_string();
        let purchase_id_str = purchase_id.to_string();
        let mut parts = base_create_parts("Deck", &cat_id_str);
        parts.push(Text {
            name: "source_type",
            value: "purchase",
        });
        parts.push(Text {
            name: "source_purchase_id",
            value: &purchase_id_str,
        });

        let req = test_support::multipart_request_with_parts("/inventory", &cookie, &parts);
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get("location").unwrap(), "/inventory");
        assert_eq!(qry::list(&state.conn()).unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A `source_donation_id` submitted alongside `source_type=purchase`
    /// must never leak into the persisted row — `draft_from_form` forces it
    /// to `None` unless `source_type == Donation`, keyed off the
    /// authoritative parsed type rather than which `<select>`s happened to
    /// be present in the body. The donation `<select>` is always present in
    /// the DOM now (hidden via inline style, not removed via Askama) so the
    /// JS toggle works, which is exactly why this guard matters: a
    /// hidden-but-present field still submits its value. Mirrors
    /// `eur_ledger.rs`'s `create_ignores_a_submitted_donor_id_for_self_funding`.
    #[tokio::test]
    async fn create_ignores_a_submitted_source_donation_id_for_purchase_source() {
        use test_support::MultipartPart::Text;
        let (state, dir) = test_support::test_app("inventory-create-stray-donation-id-ignored");
        let (cat_id, purchase_id) = setup_purchase(&state.conn());
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let cat_id_str = cat_id.to_string();
        let purchase_id_str = purchase_id.to_string();
        let mut parts = base_create_parts("Deck", &cat_id_str);
        parts.push(Text {
            name: "source_type",
            value: "purchase",
        });
        parts.push(Text {
            name: "source_purchase_id",
            value: &purchase_id_str,
        });
        parts.push(Text {
            name: "source_donation_id",
            value: "999999",
        });

        let req = test_support::multipart_request_with_parts("/inventory", &cookie, &parts);
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let items = qry::list(&state.conn()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source_donation_id, None);
        assert_eq!(items[0].source_purchase_id, Some(purchase_id));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Regression coverage for making the Source radios actually mandatory
    /// (CLAUDE.md-tracked follow-up to the eur-ledger Typ fix): a missing
    /// `source_type` used to silently default to Donation server-side even
    /// though the HTML `required` attribute is trivially bypassable.
    #[tokio::test]
    async fn create_rejects_a_missing_source_type_and_inserts_nothing() {
        let (state, dir) = test_support::test_app("inventory-create-missing-source-type");
        let (cat_id, _purchase_id) = setup_purchase(&state.conn());
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let cat_id_str = cat_id.to_string();
        let parts = base_create_parts("Deck", &cat_id_str);

        let req = test_support::multipart_request_with_parts("/inventory", &cookie, &parts);
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body_text = test_support::body_text(res).await;
        assert!(body_text.contains("Please select a source."));
        assert!(qry::list(&state.conn()).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Same guard, but for a non-empty value that isn't one of the two
    /// allowed `source_type`s — confirms the catch-all arm (via
    /// `SourceType::from_str` returning `None`) rejects garbage too, not
    /// just the empty-string/missing-field case above.
    #[tokio::test]
    async fn create_rejects_an_invalid_source_type_and_inserts_nothing() {
        use test_support::MultipartPart::Text;
        let (state, dir) = test_support::test_app("inventory-create-invalid-source-type");
        let (cat_id, purchase_id) = setup_purchase(&state.conn());
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let cat_id_str = cat_id.to_string();
        let purchase_id_str = purchase_id.to_string();
        let mut parts = base_create_parts("Deck", &cat_id_str);
        parts.push(Text {
            name: "source_type",
            value: "nonsense-value",
        });
        parts.push(Text {
            name: "source_purchase_id",
            value: &purchase_id_str,
        });

        let req = test_support::multipart_request_with_parts("/inventory", &cookie, &parts);
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body_text = test_support::body_text(res).await;
        assert!(body_text.contains("Please select a source."));
        assert!(qry::list(&state.conn()).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn create_with_two_documents_saves_the_item_and_both_files_then_redirects_to_the_list() {
        use test_support::MultipartPart::{File, Text};
        let (state, dir) = test_support::test_app("inventory-create-with-two-documents");
        let (cat_id, purchase_id) = setup_purchase(&state.conn());
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let cat_id_str = cat_id.to_string();
        let purchase_id_str = purchase_id.to_string();
        let mut parts = base_create_parts("Deck", &cat_id_str);
        parts.push(Text {
            name: "source_type",
            value: "purchase",
        });
        parts.push(Text {
            name: "source_purchase_id",
            value: &purchase_id_str,
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
        parts.push(Text {
            name: "doc_label",
            value: "receipt",
        });
        parts.push(File {
            name: "doc_file",
            filename: "receipt.pdf",
            bytes: b"fake receipt bytes",
        });

        let req = test_support::multipart_request_with_parts("/inventory", &cookie, &parts);
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get("location").unwrap(), "/inventory");
        let conn = state.conn();
        let items = qry::list(&conn).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(
            documents_qry::list_for_record(&conn, "item", items[0].id)
                .unwrap()
                .len(),
            2
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn create_without_documents_still_redirects_to_the_list() {
        use test_support::MultipartPart::Text;
        let (state, dir) = test_support::test_app("inventory-create-without-documents");
        let (cat_id, purchase_id) = setup_purchase(&state.conn());
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let cat_id_str = cat_id.to_string();
        let purchase_id_str = purchase_id.to_string();
        let mut parts = base_create_parts("Deck", &cat_id_str);
        parts.push(Text {
            name: "source_type",
            value: "purchase",
        });
        parts.push(Text {
            name: "source_purchase_id",
            value: &purchase_id_str,
        });

        let req = test_support::multipart_request_with_parts("/inventory", &cookie, &parts);
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get("location").unwrap(), "/inventory");
        let conn = state.conn();
        assert_eq!(qry::list(&conn).unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn create_with_an_empty_file_row_ignores_it() {
        use test_support::MultipartPart::{File, Text};
        let (state, dir) = test_support::test_app("inventory-create-empty-file-row");
        let (cat_id, purchase_id) = setup_purchase(&state.conn());
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let cat_id_str = cat_id.to_string();
        let purchase_id_str = purchase_id.to_string();
        let mut parts = base_create_parts("Deck", &cat_id_str);
        parts.push(Text {
            name: "source_type",
            value: "purchase",
        });
        parts.push(Text {
            name: "source_purchase_id",
            value: &purchase_id_str,
        });
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

        let req = test_support::multipart_request_with_parts("/inventory", &cookie, &parts);
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get("location").unwrap(), "/inventory");
        let conn = state.conn();
        let items = qry::list(&conn).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(
            documents_qry::list_for_record(&conn, "item", items[0].id)
                .unwrap()
                .len(),
            0
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn create_with_one_unknown_label_keeps_the_item_and_reports_the_failure() {
        use test_support::MultipartPart::{File, Text};
        let (state, dir) = test_support::test_app("inventory-create-one-unknown-label");
        let (cat_id, purchase_id) = setup_purchase(&state.conn());
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let cat_id_str = cat_id.to_string();
        let purchase_id_str = purchase_id.to_string();
        let mut parts = base_create_parts("Deck", &cat_id_str);
        parts.push(Text {
            name: "source_type",
            value: "purchase",
        });
        parts.push(Text {
            name: "source_purchase_id",
            value: &purchase_id_str,
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
        parts.push(Text {
            name: "doc_label",
            value: "not-a-real-label",
        });
        parts.push(File {
            name: "doc_file",
            filename: "bad.png",
            bytes: b"fake bad bytes",
        });

        let req = test_support::multipart_request_with_parts("/inventory", &cookie, &parts);
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        assert!(body.contains("Unknown document label"));
        assert!(body.contains("Edit Item"));
        // The status list must name the file the user actually picked
        // (`chat.png`/`bad.png`), not the internal generated temp upload
        // path it was written to on the way to `service::attach_documents`.
        assert!(body.contains("chat.png"));
        assert!(body.contains("bad.png"));
        assert!(!body.contains("adm-sfa-web-upload-item-new-"));
        let conn = state.conn();
        let items = qry::list(&conn).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(
            documents_qry::list_for_record(&conn, "item", items[0].id)
                .unwrap()
                .len(),
            1
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn create_without_a_session_cookie_redirects_to_login() {
        let (state, dir) = test_support::test_app("inventory-create-unauthenticated");
        let (cat_id, purchase_id) = setup_purchase(&state.conn());
        let app = crate::build_app(state.clone());

        let cat_id_str = cat_id.to_string();
        let purchase_id_str = purchase_id.to_string();
        let mut parts = base_create_parts("Deck", &cat_id_str);
        parts.push(test_support::MultipartPart::Text {
            name: "source_type",
            value: "purchase",
        });
        parts.push(test_support::MultipartPart::Text {
            name: "source_purchase_id",
            value: &purchase_id_str,
        });

        let req = test_support::multipart_request_with_parts("/inventory", "", &parts);
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get("location").unwrap(), "/login");
        let conn = state.conn();
        assert_eq!(qry::list(&conn).unwrap().len(), 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Same mandatory-source guard, applied to `update()` too (per the
    /// owner's explicit choice — not just `create()`).
    #[tokio::test]
    async fn update_rejects_a_missing_source_type_and_leaves_the_item_unchanged() {
        let (state, dir) = test_support::test_app("inventory-update-missing-source-type");
        let (cat_id, purchase_id) = setup_purchase(&state.conn());
        let item_id = qry::insert(
            &state.conn(),
            &InventoryItemDraft {
                name: "Deck".to_string(),
                category_id: Some(cat_id),
                source_type: SourceType::Purchase,
                source_donation_id: None,
                source_purchase_id: Some(purchase_id),
                location: Location::Germany,
                status: ItemStatus::Available,
                notes: String::new(),
            },
        )
        .unwrap();
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body =
            format!("name=Renamed&category_id={cat_id}&location=germany&status=available&notes=");
        let req = Request::builder()
            .method("POST")
            .uri(format!("/inventory/{item_id}"))
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body_text = test_support::body_text(res).await;
        assert!(body_text.contains("Please select a source."));
        assert_eq!(
            qry::get(&state.conn(), item_id).unwrap().unwrap().name,
            "Deck"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The "+ New donation" flow: creating a donation with a `return_to`
    /// carried over from `/inventory/new` should redirect there (with the
    /// new donation's id appended) instead of always landing on the
    /// donations list. Mirrors `donors.rs`'s
    /// `create_with_a_return_to_redirects_there_with_donor_id_appended`.
    #[tokio::test]
    async fn create_donation_with_a_return_to_redirects_there_with_donation_id_appended() {
        let (state, dir) = test_support::test_app("inventory-donation-return-to");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body = "date_received=2026-01-01&donor_id=&notes=\
                     &return_to=%2Finventory%2Fnew%3Fname%3DDeck";
        let req = Request::builder()
            .method("POST")
            .uri("/inventory/donations")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let location = res.headers().get("location").unwrap().to_str().unwrap();
        assert_eq!(location, "/inventory/new?name=Deck&donation_id=1");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn create_donation_without_a_return_to_redirects_to_the_donations_list() {
        let (state, dir) = test_support::test_app("inventory-donation-no-return-to");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body = "date_received=2026-01-01&donor_id=&notes=";
        let req = Request::builder()
            .method("POST")
            .uri("/inventory/donations")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let location = res.headers().get("location").unwrap().to_str().unwrap();
        assert_eq!(location, "/inventory/donations");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A `return_to` that isn't a root-relative path (an open-redirect
    /// attempt) is rejected — falls back to the donations list instead of
    /// ever being handed to `Redirect::to`. Mirrors `donors.rs`'s
    /// `create_ignores_an_unsafe_return_to`.
    #[tokio::test]
    async fn create_donation_ignores_an_unsafe_return_to() {
        let (state, dir) = test_support::test_app("inventory-donation-unsafe-return-to");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body = "date_received=2026-01-01&donor_id=&notes=\
                     &return_to=https%3A%2F%2Fevil.example";
        let req = Request::builder()
            .method("POST")
            .uri("/inventory/donations")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let location = res.headers().get("location").unwrap().to_str().unwrap();
        assert_eq!(location, "/inventory/donations");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The read-back side of the third-level "+ New donor" round trip:
    /// `donors.rs::create` appends `donor_id` onto whatever `return_to` it
    /// was handed, which for this page is `/inventory/donations` carrying
    /// `date_received`/`notes`/its own incoming `return_to` (the item page,
    /// one level further up). `GET /inventory/donations` with all of that
    /// on the query string should prefill the donation form and preselect
    /// the new donor, without losing the outer `return_to` for the
    /// donation's own eventual redirect back to the item page.
    #[tokio::test]
    async fn donations_page_with_a_donor_id_query_param_preselects_and_prefills() {
        let (state, dir) = test_support::test_app("inventory-donations-donor-prefill");
        let donor_id = donors_qry::insert(
            &state.conn(),
            &DonorDraft {
                name: "Alex".to_string(),
                contact_info: String::new(),
                notes: String::new(),
            },
        )
        .unwrap();
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("GET")
            .uri(format!(
                "/inventory/donations?date_received=2026-02-01&notes=from+alex\
                 &donor_id={donor_id}&return_to=%2Finventory%2Fnew%3Fname%3DDeck"
            ))
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body_text = test_support::body_text(res).await;
        assert!(body_text.contains(r#"value="2026-02-01""#));
        assert!(body_text.contains("from alex"));
        assert!(body_text.contains(&format!(r#"value="{donor_id}" selected"#)));
        assert!(body_text.contains(r#"name="return_to" value="/inventory/new?name=Deck""#));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A `donor_id` that doesn't resolve to a real donor (stale link, or a
    /// hand-edited query string) must not silently preselect the browser's
    /// default option — mirrors `new_form_with_a_nonexistent_donation_id_
    /// does_not_preselect_source` below and `eur_ledger.rs`'s equivalent
    /// donor check.
    #[tokio::test]
    async fn donations_page_with_a_nonexistent_donor_id_does_not_preselect_a_donor() {
        let (state, dir) = test_support::test_app("inventory-donations-bad-donor-id");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("GET")
            .uri("/inventory/donations?donor_id=999999")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body_text = test_support::body_text(res).await;
        assert!(!body_text.contains("selected"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The read-back side of the round trip: a valid `donation_id` query
    /// param on `/inventory/new` should preselect Source=Donation, show the
    /// dropdown, and select the new donation — plus the other in-progress
    /// fields (name, category, location, status, notes) the New Item form's
    /// own JS carried along on the way out. Mirrors `eur_ledger.rs`'s
    /// `new_form_with_a_donor_id_query_param_preselects_and_shows_donor`.
    #[tokio::test]
    async fn new_form_with_a_donation_id_query_param_preselects_and_shows_donation() {
        let (state, dir) = test_support::test_app("inventory-new-form-donation-prefill");
        let cat_id = cat_qry::insert(&state.conn(), "Decks").unwrap();
        let donation_id = donors_qry::insert_donation(
            &state.conn(),
            &PhysicalDonationDraft {
                donor_id: None,
                date_received: "2026-01-05".to_string(),
                notes: String::new(),
            },
        )
        .unwrap();
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("GET")
            .uri(format!(
                "/inventory/new?name=Deck&category_id={cat_id}&location=brazil&status=reserved\
                 &notes=handled+with+care&donation_id={donation_id}"
            ))
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body_text = test_support::body_text(res).await;
        assert!(body_text.contains(r#"value="Deck""#));
        assert!(body_text.contains(&format!(r#"value="{cat_id}" selected"#)));
        assert!(body_text.contains(r#"value="brazil" checked"#));
        assert!(body_text.contains(r#"value="reserved" checked"#));
        assert!(body_text.contains("handled with care"));
        assert!(body_text.contains(r#"value="donation" required checked"#));
        assert!(body_text.contains(&format!(r#"value="{donation_id}" selected"#)));
        assert!(!body_text.contains(r#"id="source_donation_field" style="display:none""#));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A `donation_id` that doesn't resolve to a real donation (stale link,
    /// or a hand-edited query string) must not force Source open — mirrors
    /// `eur_ledger.rs`'s
    /// `new_form_with_a_nonexistent_donor_id_does_not_show_the_donor_field`.
    #[tokio::test]
    async fn new_form_with_a_nonexistent_donation_id_does_not_preselect_source() {
        let (state, dir) = test_support::test_app("inventory-new-form-bad-donation-id");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("GET")
            .uri("/inventory/new?donation_id=999999")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body_text = test_support::body_text(res).await;
        assert!(body_text.contains(r#"id="source_donation_field" style="display:none""#));
        assert!(!body_text.contains(r#"value="donation" required checked"#));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Regression test: `purchase_label()` used to call `format::amount()`
    /// (the *ambient* locale, which `web` never sets) instead of
    /// `format::amount_in(p.cost, locale)` — mirrors `brl_ledger.rs`'s
    /// identical regression test. With `ui_locale` set to German, the
    /// purchase picker's option text must render the cost with a comma
    /// decimal separator (`1.234,56`), not the English default
    /// (`1,234.56`).
    #[tokio::test]
    async fn purchase_picker_formats_cost_using_the_resolved_ui_locale() {
        let (state, dir) = test_support::test_app("inventory-purchase-picker-locale-format");
        let conn = state.conn();
        adm_sfa_core::db::queries::settings::set(&conn, "ui_locale", "de").unwrap();
        purchases_qry::insert(
            &conn,
            &PurchaseDraft {
                date: "2026-01-01".to_string(),
                currency: Currency::Eur,
                cost_str: "1234.56".to_string(),
                channel: "Kleinanzeigen".to_string(),
                seller_info: String::new(),
                multiple_items: false,
                status: PurchaseStatus::Bought,
            },
        )
        .unwrap();
        drop(conn);
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("GET")
            .uri("/inventory/new")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body_text = test_support::body_text(res).await;
        assert!(
            body_text.contains("1.234,56"),
            "expected the German-formatted cost in the purchase picker: {body_text}"
        );
        assert!(
            !body_text.contains("1,234.56"),
            "cost rendered in English format despite ui_locale=de: {body_text}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
