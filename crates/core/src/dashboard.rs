use rusqlite::Connection;
use rust_decimal::Decimal;

use crate::db::queries::{brl_ledger, eur_ledger, inventory, outbound, purchases};
use crate::model::inventory::ItemStatus;
use crate::model::purchase::PurchaseStatus;
use crate::reporting::compute_balance;

#[derive(Default)]
pub struct DashboardData {
    pub eur_balance: Decimal,
    pub brl_balance: Decimal,
    pub inventory_available: i64,
    pub inventory_reserved: i64,
    pub inventory_donated: i64,
    pub outbound_items_this_year: i64,
    pub outbound_cash_this_year: Decimal,
    pub negotiating_count: i64,
}

pub fn load(conn: &Connection) -> rusqlite::Result<DashboardData> {
    let eur_rows = eur_ledger::list(conn)?;
    let eur_balance = compute_balance(eur_rows.iter().map(|r| (r.tx_type.is_inflow(), r.amount)));

    let brl_rows = brl_ledger::list(conn)?;
    let brl_balance = compute_balance(brl_rows.iter().map(|r| (r.tx_type.is_inflow(), r.amount)));

    let items = inventory::list(conn)?;
    let inventory_available = items
        .iter()
        .filter(|i| i.status == ItemStatus::Available)
        .count() as i64;
    let inventory_reserved = items
        .iter()
        .filter(|i| i.status == ItemStatus::Reserved)
        .count() as i64;
    let inventory_donated = items
        .iter()
        .filter(|i| i.status == ItemStatus::Donated)
        .count() as i64;

    let current_year = chrono::Local::now().format("%Y").to_string();
    let outbound_events = outbound::list(conn)?;
    let (outbound_items_this_year, outbound_cash_this_year) = outbound_events
        .iter()
        .filter(|e| e.date.starts_with(&current_year))
        .fold((0i64, Decimal::ZERO), |(items, cash), e| {
            (
                items + e.item_count,
                cash + e.cash_amount_brl.unwrap_or(Decimal::ZERO),
            )
        });

    let all_purchases = purchases::list(conn)?;
    let negotiating_count = all_purchases
        .iter()
        .filter(|p| p.status == PurchaseStatus::Negotiating)
        .count() as i64;

    Ok(DashboardData {
        eur_balance,
        brl_balance,
        inventory_available,
        inventory_reserved,
        inventory_donated,
        outbound_items_this_year,
        outbound_cash_this_year,
        negotiating_count,
    })
}
