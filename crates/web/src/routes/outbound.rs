use axum::extract::{Path, RawForm, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Form;
use axum::Router;
use rust_decimal::Decimal;
use serde::Deserialize;

use adm_sfa_core::db::queries::{inventory as inventory_qry, outbound as qry};
use adm_sfa_core::format;
use adm_sfa_core::model::inventory::{InventoryItemRow, ItemStatus};
use adm_sfa_core::model::outbound::{OutboundEventDraft, RecipientProject, RecipientProjectDraft};
use adm_sfa_core::service;

use crate::state::AppState;
use crate::templates::{
    HtmlTemplate, ItemOption, OutboundFormTemplate, OutboundListTemplate, OutboundRow,
    RecipientOption, RecipientRow, RecipientsTemplate,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/outbound", get(list).post(create))
        .route("/outbound/new", get(new_form))
        .route("/outbound/{id}/edit", get(edit_form))
        .route("/outbound/{id}", axum::routing::post(update))
        .route(
            "/outbound/recipients",
            get(recipients).post(create_recipient),
        )
}

fn recipient_options(
    recipients: &[RecipientProject],
    selected: Option<i64>,
) -> Vec<RecipientOption> {
    recipients
        .iter()
        // Same rule as desktop's show_recipient_project_picker: an inactive
        // recipient is hidden from new selections but still shown (and thus
        // still selectable/keepable) if it's the event's current recipient.
        .filter(|p| p.active || selected == Some(p.id))
        .map(|p| RecipientOption {
            id: p.id,
            name: p.name.clone(),
            selected: selected == Some(p.id),
        })
        .collect()
}

fn item_options(items: &[InventoryItemRow], selected_ids: &[i64]) -> Vec<ItemOption> {
    items
        .iter()
        .filter(|item| item.status == ItemStatus::Available || selected_ids.contains(&item.id))
        .map(|item| ItemOption {
            id: item.id,
            label: format!(
                "{} \u{2014} {} \u{2014} {}",
                item.name, item.category_name, item.source_desc
            ),
            selected: selected_ids.contains(&item.id),
        })
        .collect()
}

fn form_template(
    conn: &rusqlite::Connection,
    id: Option<i64>,
    draft: &OutboundEventDraft,
    selected_item_ids: &[i64],
    error: Option<String>,
) -> OutboundFormTemplate {
    let recipients = qry::list_recipient_projects(conn).unwrap_or_default();
    let items = inventory_qry::list(conn).unwrap_or_default();
    OutboundFormTemplate {
        id,
        date: draft.date.clone(),
        recipients: recipient_options(&recipients, draft.recipient_project_id),
        cash_amount_brl_str: draft.cash_amount_brl_str.clone(),
        notes: draft.notes.clone(),
        items: item_options(&items, selected_item_ids),
        error,
    }
}

fn event_summary(item_count: i64, cash: Option<Decimal>) -> String {
    let mut s = if item_count == 1 {
        "1 item".to_string()
    } else {
        format!("{item_count} items")
    };
    if let Some(cash) = cash {
        if cash > Decimal::ZERO {
            s.push_str(&format!(", R$ {}", format::amount(cash)));
        }
    }
    s
}

async fn list(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.conn();
    let events = qry::list(&conn).unwrap_or_default();
    let rows = events
        .into_iter()
        .map(|e| OutboundRow {
            id: e.id,
            date_display: format::date(&e.date),
            recipient_name: e.recipient_name,
            summary: event_summary(e.item_count, e.cash_amount_brl),
        })
        .collect();
    HtmlTemplate(OutboundListTemplate { events: rows })
}

async fn new_form(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.conn();
    let draft = OutboundEventDraft {
        date: chrono::Local::now().format("%Y-%m-%d").to_string(),
        ..OutboundEventDraft::default()
    };
    HtmlTemplate(form_template(&conn, None, &draft, &[], None))
}

async fn edit_form(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let conn = state.conn();
    let Some(event) = qry::list(&conn)
        .unwrap_or_default()
        .into_iter()
        .find(|e| e.id == id)
    else {
        return (axum::http::StatusCode::NOT_FOUND, "event not found").into_response();
    };
    let selected_item_ids = qry::item_ids_for_event(&conn, id).unwrap_or_default();
    let draft = OutboundEventDraft {
        date: event.date,
        recipient_project_id: Some(event.recipient_project_id),
        cash_amount_brl_str: event
            .cash_amount_brl
            .map(|d| d.to_string())
            .unwrap_or_default(),
        notes: event.notes.unwrap_or_default(),
    };
    HtmlTemplate(form_template(
        &conn,
        Some(id),
        &draft,
        &selected_item_ids,
        None,
    ))
    .into_response()
}

/// Outbound's form needs a repeated `item_ids` field (one checkbox per
/// eligible item, all sharing the same `name`) — `axum::Form`'s
/// `serde_urlencoded` backend rejects that outright (`Vec<T>` from
/// duplicate keys isn't something it supports, confirmed empirically: it
/// errors "invalid type: string, expected a sequence" rather than
/// collecting). `RawForm` + `form_urlencoded::parse` reads the raw
/// key-value pairs directly instead, so a manual pass here replaces
/// `#[derive(Deserialize)] struct ...Form` used by every other section's
/// route in this crate.
struct OutboundForm {
    date: String,
    recipient_project_id: Option<i64>,
    cash_amount_brl_str: String,
    notes: String,
    item_ids: Vec<i64>,
}

fn parse_outbound_form(bytes: &[u8]) -> OutboundForm {
    let mut date = String::new();
    let mut recipient_project_id = None;
    let mut cash_amount_brl_str = String::new();
    let mut notes = String::new();
    let mut item_ids = Vec::new();

    for (key, value) in form_urlencoded::parse(bytes) {
        match key.as_ref() {
            "date" => date = value.into_owned(),
            "recipient_project_id" => {
                recipient_project_id = value.trim().parse().ok();
            }
            "cash_amount_brl_str" => cash_amount_brl_str = value.into_owned(),
            "notes" => notes = value.into_owned(),
            "item_ids" => {
                if let Ok(id) = value.trim().parse() {
                    item_ids.push(id);
                }
            }
            _ => {}
        }
    }

    OutboundForm {
        date,
        recipient_project_id,
        cash_amount_brl_str,
        notes,
        item_ids,
    }
}

async fn create(State(state): State<AppState>, RawForm(bytes): RawForm) -> Response {
    let form = parse_outbound_form(&bytes);
    let draft = OutboundEventDraft {
        date: form.date,
        recipient_project_id: form.recipient_project_id,
        cash_amount_brl_str: form.cash_amount_brl_str,
        notes: form.notes,
    };
    let conn = state.conn();
    match service::donate_items(&conn, &draft, &form.item_ids) {
        Ok(id) => Redirect::to(&format!("/outbound/{id}/edit")).into_response(),
        Err(e) => HtmlTemplate(form_template(
            &conn,
            None,
            &draft,
            &form.item_ids,
            Some(e.to_string()),
        ))
        .into_response(),
    }
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RawForm(bytes): RawForm,
) -> Response {
    let form = parse_outbound_form(&bytes);
    let draft = OutboundEventDraft {
        date: form.date,
        recipient_project_id: form.recipient_project_id,
        cash_amount_brl_str: form.cash_amount_brl_str,
        notes: form.notes,
    };
    let conn = state.conn();
    match qry::update(&conn, id, &draft, &form.item_ids) {
        Ok(()) => Redirect::to(&format!("/outbound/{id}/edit")).into_response(),
        Err(e) => HtmlTemplate(form_template(
            &conn,
            Some(id),
            &draft,
            &form.item_ids,
            Some(e.to_string()),
        ))
        .into_response(),
    }
}

/// Recipient projects, like physical donations (see `inventory.rs`'s
/// `/inventory/donations`), are only ever created inline from this
/// section's own sub-form in desktop — no standalone CRUD page. Same
/// no-JS constraint, same standalone-detour solution: create here, then
/// pick it from the dropdown back on `/outbound/new`. Unlike donations,
/// recipient projects *can* be edited in principle (an `active` flag
/// exists), but `core` has no `update` for this table either — matching
/// what's actually implemented rather than adding one speculatively.
async fn recipients(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.conn();
    let recipients = qry::list_recipient_projects(&conn).unwrap_or_default();
    let rows = recipients
        .into_iter()
        .map(|p| RecipientRow {
            name: p.name,
            contact_info: p.contact_info.unwrap_or_default(),
            location: p.location.unwrap_or_default(),
            active: p.active,
        })
        .collect();
    HtmlTemplate(RecipientsTemplate {
        recipients: rows,
        error: None,
    })
}

#[derive(Deserialize)]
struct RecipientForm {
    name: String,
    #[serde(default)]
    contact_info: String,
    #[serde(default)]
    location: String,
}

async fn create_recipient(
    State(state): State<AppState>,
    Form(form): Form<RecipientForm>,
) -> Response {
    let draft = RecipientProjectDraft {
        name: form.name,
        contact_info: form.contact_info,
        location: form.location,
        active: true,
    };
    let conn = state.conn();
    match qry::insert_recipient_project(&conn, &draft) {
        Ok(_) => Redirect::to("/outbound/recipients").into_response(),
        Err(e) => {
            let recipients = qry::list_recipient_projects(&conn).unwrap_or_default();
            let rows = recipients
                .into_iter()
                .map(|p| RecipientRow {
                    name: p.name,
                    contact_info: p.contact_info.unwrap_or_default(),
                    location: p.location.unwrap_or_default(),
                    active: p.active,
                })
                .collect();
            HtmlTemplate(RecipientsTemplate {
                recipients: rows,
                error: Some(e.to_string()),
            })
            .into_response()
        }
    }
}
