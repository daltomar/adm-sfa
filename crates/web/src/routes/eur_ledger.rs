use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Form;
use axum::Router;
use rust_decimal::Decimal;
use serde::Deserialize;

use adm_sfa_core::db::queries::{donors as donors_qry, eur_ledger as qry};
use adm_sfa_core::format;
use adm_sfa_core::model::transaction::{EurTxDraft, EurTxRow, EurTxType, ManualEurTxType};
use adm_sfa_core::reporting::compute_balance;

use crate::state::AppState;
use crate::templates::{EurLedgerListTemplate, EurLedgerRow, EurTxFormTemplate, HtmlTemplate};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/eur-ledger", get(list).post(create))
        .route("/eur-ledger/new", get(new_form))
        .route("/eur-ledger/{id}/edit", get(edit_form))
        .route("/eur-ledger/{id}", axum::routing::post(update))
}

/// Same keys `core::model::transaction::EurTxType::label()` uses, but with
/// an *explicit* locale rather than that method's ambient
/// `rust_i18n::locale()` — see `main.rs`'s doc comment on why `web` can't
/// use the ambient form. (`web` previously didn't call this function at
/// all here; it used to inline plain English literals instead.)
fn type_label(tx_type: EurTxType, locale: &str) -> String {
    let key = match tx_type {
        EurTxType::DonationIn => "status.source_type.donation",
        EurTxType::SelfFundingIn => "status.eur_tx.self_funding_in",
        EurTxType::PurchaseOut => "status.source_type.purchase",
        EurTxType::TransferToBrlOut => "status.eur_tx.transfer_to_brl_out",
    };
    rust_i18n::t!(key, locale = locale).to_string()
}

fn row_desc(row: &EurTxRow) -> String {
    match row.tx_type {
        EurTxType::DonationIn => row.donor_name.clone().unwrap_or_default(),
        EurTxType::SelfFundingIn => row.note.clone().unwrap_or_default(),
        EurTxType::PurchaseOut => row.purchase_channel.clone().unwrap_or_default(),
        EurTxType::TransferToBrlOut => row
            .note
            .clone()
            .unwrap_or_else(|| "EUR\u{2192}BRL".to_string()),
    }
}

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

async fn list(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let rows = qry::list(&conn).unwrap_or_default();
    let balance = compute_balance(rows.iter().map(|r| (r.tx_type.is_inflow(), r.amount)));
    let balance_display = format::amount(balance);

    let view_rows = rows
        .iter()
        .map(|r| EurLedgerRow {
            id: r.id,
            date_display: format::date(&r.date),
            type_label: type_label(r.tx_type, &locale),
            sign: if r.tx_type.is_inflow() { "+" } else { "-" },
            amount_display: format::amount(r.amount),
            desc: row_desc(r),
            editable: r.tx_type.is_manual(),
        })
        .collect();

    let balance_label = rust_i18n::t!(
        "common.balance",
        locale = &locale,
        symbol = "\u{20ac}",
        amount = &balance_display
    )
    .to_string();

    HtmlTemplate(EurLedgerListTemplate {
        rows: view_rows,
        balance_positive: balance >= Decimal::ZERO,
        balance_label,
        locale,
    })
}

async fn new_form(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let donors = donor_options(&conn, None);
    HtmlTemplate(EurTxFormTemplate {
        id: None,
        date: chrono::Local::now().format("%Y-%m-%d").to_string(),
        type_label: None,
        show_donor: true,
        amount_str: String::new(),
        donors,
        note: String::new(),
        error: None,
        locale,
    })
}

async fn edit_form(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let Some(row) = qry::get(&conn, id).ok().flatten() else {
        return (axum::http::StatusCode::NOT_FOUND, "entry not found").into_response();
    };
    if !row.tx_type.is_manual() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "this entry was created automatically and can't be edited here",
        )
            .into_response();
    }
    let is_donation = row.tx_type == EurTxType::DonationIn;
    let donors = donor_options(&conn, row.donor_id);
    HtmlTemplate(EurTxFormTemplate {
        id: Some(id),
        date: row.date,
        type_label: Some(type_label(row.tx_type, &locale)),
        show_donor: is_donation,
        amount_str: row.amount.to_string(),
        donors,
        note: row.note.unwrap_or_default(),
        error: None,
        locale,
    })
    .into_response()
}

#[derive(Deserialize)]
struct EurTxForm {
    date: String,
    /// Create-form only: "donation_in" or "self_funding_in". Absent on the
    /// edit form, since type is fixed after creation.
    #[serde(default)]
    tx_type: String,
    amount_str: String,
    #[serde(default)]
    donor_id: String,
    #[serde(default)]
    note: String,
}

fn parsed_donor_id(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        s.parse().ok()
    }
}

async fn create(State(state): State<AppState>, Form(form): Form<EurTxForm>) -> Response {
    let manual_type = if form.tx_type == "self_funding_in" {
        ManualEurTxType::SelfFundingIn
    } else {
        ManualEurTxType::DonationIn
    };
    let donor_id = if manual_type == ManualEurTxType::DonationIn {
        parsed_donor_id(&form.donor_id)
    } else {
        None
    };
    let draft = EurTxDraft {
        date: form.date,
        tx_type: manual_type,
        amount_str: form.amount_str,
        donor_id,
        note: form.note,
    };
    let conn = state.conn();
    match qry::insert(&conn, &draft) {
        Ok(_) => Redirect::to("/eur-ledger").into_response(),
        Err(e) => {
            let locale = crate::i18n::resolve_locale(&conn);
            let donors = donor_options(&conn, draft.donor_id);
            HtmlTemplate(EurTxFormTemplate {
                id: None,
                date: draft.date,
                type_label: None,
                show_donor: true,
                amount_str: draft.amount_str,
                donors,
                note: draft.note,
                error: Some(e.to_string()),
                locale,
            })
            .into_response()
        }
    }
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<EurTxForm>,
) -> Response {
    let conn = state.conn();
    let Some(existing) = qry::get(&conn, id).ok().flatten() else {
        return (axum::http::StatusCode::NOT_FOUND, "entry not found").into_response();
    };
    if !existing.tx_type.is_manual() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "this entry was created automatically and can't be edited here",
        )
            .into_response();
    }
    let manual_type = if existing.tx_type == EurTxType::DonationIn {
        ManualEurTxType::DonationIn
    } else {
        ManualEurTxType::SelfFundingIn
    };
    let donor_id = if manual_type == ManualEurTxType::DonationIn {
        parsed_donor_id(&form.donor_id)
    } else {
        None
    };
    let draft = EurTxDraft {
        date: form.date,
        tx_type: manual_type,
        amount_str: form.amount_str,
        donor_id,
        note: form.note,
    };
    match qry::update(&conn, id, &draft) {
        Ok(()) => Redirect::to("/eur-ledger").into_response(),
        Err(e) => {
            let locale = crate::i18n::resolve_locale(&conn);
            let donors = donor_options(&conn, draft.donor_id);
            HtmlTemplate(EurTxFormTemplate {
                id: Some(id),
                date: draft.date,
                type_label: Some(type_label(existing.tx_type, &locale)),
                show_donor: manual_type == ManualEurTxType::DonationIn,
                amount_str: draft.amount_str,
                donors,
                note: draft.note,
                error: Some(e.to_string()),
                locale,
            })
            .into_response()
        }
    }
}
