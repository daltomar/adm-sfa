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

fn location_label(l: Location) -> &'static str {
    match l {
        Location::Germany => "Germany",
        Location::Brazil => "Brazil",
    }
}

fn status_label(s: ItemStatus) -> &'static str {
    match s {
        ItemStatus::Available => "Available",
        ItemStatus::Reserved => "Reserved",
        ItemStatus::Donated => "Donated",
    }
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
) -> Vec<PurchaseOption> {
    purchases
        .iter()
        .filter(|p| p.status == PurchaseStatus::Bought)
        .map(|p| PurchaseOption {
            id: p.id,
            label: purchase_label(p),
            selected: selected == Some(p.id),
            blocked: purchase_source_blocked(items, p, edit_id),
        })
        .collect()
}

fn donation_label(d: &adm_sfa_core::model::donor::PhysicalDonation) -> String {
    match &d.donor_name {
        Some(name) => format!("{} \u{2014} {name}", format::date(&d.date_received)),
        None => format!("{} \u{2014} Anonymous", format::date(&d.date_received)),
    }
}

fn donation_options(
    donations: &[adm_sfa_core::model::donor::PhysicalDonation],
    selected: Option<i64>,
) -> Vec<DonationOption> {
    donations
        .iter()
        .map(|d| DonationOption {
            id: d.id,
            label: donation_label(d),
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
) -> InventoryFormTemplate {
    let categories = cat_qry::list(conn).unwrap_or_default();
    let donations = donors_qry::list_donations(conn).unwrap_or_default();
    let purchases = purchases_qry::list(conn).unwrap_or_default();
    let items = qry::list(conn).unwrap_or_default();

    InventoryFormTemplate {
        id,
        name: draft.name.clone(),
        location: draft.location.as_str().to_string(),
        status: draft.status.as_str().to_string(),
        source_type: draft.source_type.as_str().to_string(),
        categories: category_options(&categories, draft.category_id),
        donations: donation_options(&donations, draft.source_donation_id),
        purchases: purchase_options(&purchases, &items, id, draft.source_purchase_id),
        notes: draft.notes.clone(),
        error,
        documents,
    }
}

async fn list(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.conn();
    let items = qry::list(&conn).unwrap_or_default();
    let rows = items
        .into_iter()
        .map(|i| InventoryRow {
            id: i.id,
            name: i.name,
            category_name: i.category_name,
            location_label: location_label(i.location),
            status_label: status_label(i.status),
            source_desc: i.source_desc,
        })
        .collect();
    HtmlTemplate(InventoryListTemplate { items: rows })
}

async fn new_form(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.conn();
    HtmlTemplate(form_template(
        &conn,
        None,
        &InventoryItemDraft::default(),
        None,
        Vec::new(),
    ))
}

async fn edit_form(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let conn = state.conn();
    let Some(item) = qry::list(&conn)
        .unwrap_or_default()
        .into_iter()
        .find(|i| i.id == id)
    else {
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
    HtmlTemplate(form_template(&conn, Some(id), &draft, None, documents)).into_response()
}

#[derive(Deserialize)]
struct InventoryForm {
    name: String,
    category_id: String,
    location: String,
    status: String,
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

fn draft_from_form(form: InventoryForm) -> InventoryItemDraft {
    let source_type = if form.source_type == "purchase" {
        SourceType::Purchase
    } else {
        SourceType::Donation
    };
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
    let draft = draft_from_form(form);
    let conn = state.conn();
    match qry::insert(&conn, &draft) {
        Ok(id) => Redirect::to(&format!("/inventory/{id}/edit")).into_response(),
        Err(e) => HtmlTemplate(form_template(
            &conn,
            None,
            &draft,
            Some(e.to_string()),
            Vec::new(),
        ))
        .into_response(),
    }
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<InventoryForm>,
) -> Response {
    let draft = draft_from_form(form);
    let conn = state.conn();
    match qry::update(&conn, id, &draft) {
        Ok(()) => Redirect::to(&format!("/inventory/{id}/edit")).into_response(),
        Err(e) => {
            let documents = documents_qry::list_for_record(&conn, "item", id).unwrap_or_default();
            HtmlTemplate(form_template(
                &conn,
                Some(id),
                &draft,
                Some(e.to_string()),
                documents,
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
        let Some(item) = qry::list(&conn)
            .unwrap_or_default()
            .into_iter()
            .find(|i| i.id == id)
        else {
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
        return HtmlTemplate(form_template(
            &conn,
            Some(id),
            &draft,
            Some("No file was selected, or the upload could not be saved.".to_string()),
            documents,
        ))
        .into_response();
    };

    let conn = state.conn();
    // `document.record_id` has no FK (schema.sql: record identity is
    // enforced in application code, not SQL) — without this check a POST to
    // a nonexistent item id would still copy the file and insert a
    // permanently orphaned document row that no UI could ever list or
    // soft-delete, since no item exists to look it up against.
    if !qry::list(&conn)
        .unwrap_or_default()
        .iter()
        .any(|i| i.id == id)
    {
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
        Err(e) => (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
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
        Err(e) => {
            let documents = documents_qry::list_for_record(&conn, "item", id).unwrap_or_default();
            let Some(item) = qry::list(&conn)
                .unwrap_or_default()
                .into_iter()
                .find(|i| i.id == id)
            else {
                return (axum::http::StatusCode::NOT_FOUND, "item not found").into_response();
            };
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
            HtmlTemplate(form_template(&conn, Some(id), &draft, Some(e), documents)).into_response()
        }
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
    let donations = donors_qry::list_donations(&conn).unwrap_or_default();
    let rows = donations
        .into_iter()
        .map(|d| DonationRow {
            date_display: format::date(&d.date_received),
            donor_display: d.donor_name.unwrap_or_else(|| "Anonymous".to_string()),
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
    match donors_qry::insert_donation(&conn, &draft) {
        Ok(_) => Redirect::to("/inventory/donations").into_response(),
        Err(e) => {
            let donations = donors_qry::list_donations(&conn).unwrap_or_default();
            let rows = donations
                .into_iter()
                .map(|d| DonationRow {
                    date_display: format::date(&d.date_received),
                    donor_display: d.donor_name.unwrap_or_else(|| "Anonymous".to_string()),
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
            })
            .into_response()
        }
    }
}
