use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use rust_decimal::Decimal;

use adm_sfa_core::db::queries::brl_ledger as qry;
use adm_sfa_core::format;
use adm_sfa_core::model::transaction::{BrlTxRow, BrlTxType};
use adm_sfa_core::reporting::compute_balance;

use crate::state::AppState;
use crate::templates::{BrlLedgerListTemplate, BrlLedgerRow, HtmlTemplate};

pub fn router() -> Router<AppState> {
    Router::new().route("/brl-ledger", get(list))
}

fn type_label(tx_type: BrlTxType) -> &'static str {
    match tx_type {
        BrlTxType::TransferIn => "Transfer in",
        BrlTxType::BrazilPurchaseOut => "Purchase",
        BrlTxType::CashGiftOut => "Cash gift",
    }
}

fn row_desc(row: &BrlTxRow) -> String {
    match row.tx_type {
        BrlTxType::TransferIn => String::new(),
        BrlTxType::BrazilPurchaseOut => row.purchase_channel.clone().unwrap_or_default(),
        BrlTxType::CashGiftOut => row.recipient_name.clone().unwrap_or_default(),
    }
}

async fn list(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.conn();
    let rows = qry::list(&conn).unwrap_or_default();
    let balance = compute_balance(rows.iter().map(|r| (r.tx_type.is_inflow(), r.amount)));

    let view_rows = rows
        .iter()
        .map(|r| BrlLedgerRow {
            date_display: format::date(&r.date),
            type_label: type_label(r.tx_type).to_string(),
            sign: if r.tx_type.is_inflow() { "+" } else { "-" },
            amount_display: format::amount(r.amount),
            desc: row_desc(r),
        })
        .collect();

    HtmlTemplate(BrlLedgerListTemplate {
        rows: view_rows,
        balance_display: format::amount(balance),
        balance_positive: balance >= Decimal::ZERO,
    })
}
