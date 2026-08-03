use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::{Multipart, Path, State};
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
use adm_sfa_core::service;

use crate::state::AppState;
use crate::templates::{
    CategoryOption, DonationOption, DonationRow, DonationsTemplate, HtmlTemplate,
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

fn purchase_label(p: &Purchase) -> String {
    let multi = if p.multiple_items { " (multi)" } else { "" };
    format!(
        "{} \u{2014} {} \u{2014} {} {}{multi}",
        format::date(&p.date),
        p.channel,
        p.currency.symbol(),
        format::amount(p.cost)
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
            let base_label = purchase_label(p);
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
        locale,
    }
}

/// Re-fetches the persisted item and re-renders its edit form with an error
/// banner — same pattern as `purchases.rs::purchase_form_error_response`.
/// `attach_document` has no in-flight draft of its own (it's a
/// document-only upload, not a full item edit), so this is the shared
/// "rebuild the draft from what's in the DB" path used both when no file
/// was submitted and when `service::attach_document` rejects the upload.
fn item_form_error_response(conn: &rusqlite::Connection, id: i64, error: String) -> Response {
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
        Some(error),
        documents,
        locale,
        None,
    ))
    .into_response()
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

async fn new_form(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    HtmlTemplate(form_template(
        &conn,
        None,
        &InventoryItemDraft::default(),
        None,
        Vec::new(),
        locale,
        Some(""),
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

async fn create(State(state): State<AppState>, Form(form): Form<InventoryForm>) -> Response {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let Some(source_type) = SourceType::from_str(&form.source_type) else {
        // `SourceType::Donation` here is a placeholder purely so the rest of
        // what was typed (name, category, location, status, notes) can
        // still be echoed back — `Some("")` below is what actually makes
        // neither Source radio render checked, same as a fresh New Item
        // form.
        let draft = draft_from_form(form, SourceType::Donation);
        let error =
            rust_i18n::t!("web.inventory.error.source_type_required", locale = &locale).to_string();
        return HtmlTemplate(form_template(
            &conn,
            None,
            &draft,
            Some(error),
            Vec::new(),
            locale,
            Some(""),
        ))
        .into_response();
    };
    let draft = draft_from_form(form, source_type);
    match qry::insert(&conn, &draft) {
        Ok(id) => Redirect::to(&format!("/inventory/{id}/edit")).into_response(),
        Err(e) => HtmlTemplate(form_template(
            &conn,
            None,
            &draft,
            Some(e.to_string()),
            Vec::new(),
            locale,
            None,
        ))
        .into_response(),
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
/// Loses the desktop convenience of auto-selecting the new donation and
/// preserving other in-progress item fields across the detour — a
/// deliberate, documented reduced-scope tradeoff for this pass, consistent
/// with phase 5's existing scope reductions (CLAUDE.md).
async fn donations(State(state): State<AppState>) -> impl IntoResponse {
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
    let donors = donors_qry::list(&conn)
        .unwrap_or_default()
        .into_iter()
        .map(|d| (d.id, d.name))
        .collect();
    HtmlTemplate(DonationsTemplate {
        donations: rows,
        date: chrono::Local::now().format("%Y-%m-%d").to_string(),
        donors,
        error: None,
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
}

async fn create_donation(
    State(state): State<AppState>,
    Form(form): Form<DonationForm>,
) -> Response {
    let draft = PhysicalDonationDraft {
        donor_id: parsed_id(&form.donor_id),
        date_received: form.date_received,
        notes: form.notes,
    };
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    match donors_qry::insert_donation(&conn, &draft) {
        Ok(_) => Redirect::to("/inventory/donations").into_response(),
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
            let donors = donors_qry::list(&conn)
                .unwrap_or_default()
                .into_iter()
                .map(|d| (d.id, d.name))
                .collect();
            HtmlTemplate(DonationsTemplate {
                donations: rows,
                date: draft.date_received,
                donors,
                error: Some(e.to_string()),
                locale,
            })
            .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support;
    use adm_sfa_core::db::queries::{
        categories as cat_qry, inventory as qry, purchases as purchases_qry,
    };
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

    fn create_form_body(
        name: &str,
        category_id: i64,
        source_type: &str,
        source_purchase_id: i64,
    ) -> String {
        format!(
            "name={name}&category_id={category_id}&location=germany&status=available\
             &source_type={source_type}&source_purchase_id={source_purchase_id}&notes="
        )
    }

    /// A valid `source_type` should still redirect and insert a row —
    /// guards the happy path through `create()`'s new early-return match,
    /// mirroring `eur_ledger.rs`'s equivalent regression test.
    #[tokio::test]
    async fn create_with_a_valid_source_type_redirects_and_inserts_a_row() {
        let (state, dir) = test_support::test_app("inventory-create-valid-source-type");
        let (cat_id, purchase_id) = setup_purchase(&state.conn());
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body = create_form_body("Deck", cat_id, "purchase", purchase_id);
        let req = Request::builder()
            .method("POST")
            .uri("/inventory")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
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
        let (state, dir) = test_support::test_app("inventory-create-stray-donation-id-ignored");
        let (cat_id, purchase_id) = setup_purchase(&state.conn());
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body = format!(
            "name=Deck&category_id={cat_id}&location=germany&status=available\
             &source_type=purchase&source_purchase_id={purchase_id}&source_donation_id=999999&notes="
        );
        let req = Request::builder()
            .method("POST")
            .uri("/inventory")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
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

        let body =
            format!("name=Deck&category_id={cat_id}&location=germany&status=available&notes=");
        let req = Request::builder()
            .method("POST")
            .uri("/inventory")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
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
        let (state, dir) = test_support::test_app("inventory-create-invalid-source-type");
        let (cat_id, purchase_id) = setup_purchase(&state.conn());
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body = create_form_body("Deck", cat_id, "nonsense-value", purchase_id);
        let req = Request::builder()
            .method("POST")
            .uri("/inventory")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body_text = test_support::body_text(res).await;
        assert!(body_text.contains("Please select a source."));
        assert!(qry::list(&state.conn()).unwrap().is_empty());
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
}
