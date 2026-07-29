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

/// Same keys `core::model::transaction::BrlTxType::label()` uses, but with
/// an explicit locale — see `eur_ledger.rs`'s `type_label` doc comment.
fn type_label(tx_type: BrlTxType, locale: &str) -> String {
    let key = match tx_type {
        BrlTxType::TransferIn => "status.brl_tx.transfer_in",
        BrlTxType::BrazilPurchaseOut => "status.source_type.purchase",
        BrlTxType::CashGiftOut => "status.brl_tx.cash_gift_out",
    };
    rust_i18n::t!(key, locale = locale).to_string()
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
    let locale = crate::i18n::resolve_locale(&conn);
    let rows = qry::list(&conn).unwrap_or_default();
    let balance = compute_balance(rows.iter().map(|r| (r.tx_type.is_inflow(), r.amount)));
    let balance_display = format::amount(balance);

    let view_rows = rows
        .iter()
        .map(|r| BrlLedgerRow {
            date_display: format::date(&r.date),
            type_label: type_label(r.tx_type, &locale),
            sign: if r.tx_type.is_inflow() { "+" } else { "-" },
            amount_display: format::amount(r.amount),
            desc: row_desc(r),
        })
        .collect();

    let balance_label = rust_i18n::t!(
        "common.balance",
        locale = &locale,
        symbol = "R$",
        amount = &balance_display
    )
    .to_string();

    HtmlTemplate(BrlLedgerListTemplate {
        rows: view_rows,
        balance_positive: balance >= Decimal::ZERO,
        balance_label,
        locale,
    })
}
