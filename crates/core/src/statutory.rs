use rusqlite::Connection;
/// Statutory report data and computation for the annual German tax submission.
///
/// Four documents are produced each year:
///   1. Aufstellung über sämtliche Einnahmen und Ausgaben
///   2. Aufstellung über das Vereinsvermögen
///   3. Tätigkeitsbericht
///   4. Protokoll der Mitgliederversammlung
///
/// The financial figures (1 & 2) are fully derived from the DB.
/// The narrative prose (3 & 4) is stored in `annual_report_draft` and
/// combined with the DB data at PDF-generation time.
use rust_decimal::Decimal;

use crate::db::queries::{annual_report_draft, eur_ledger, outbound, purchases, settings};
use crate::model::{outbound::OutboundEventRow, transaction::EurTxRow};
use crate::reporting::in_range;

// ── Public data structs ──────────────────────────────────────────────────────

/// Income/expense statement mapped to the German statutory categories.
/// Only the Ideeller Bereich rows are non-zero for this org; all
/// Vermögensverwaltung / Zweckbetrieb / Wirtschaftsbetrieb rows stay at zero.
#[derive(Debug, Default)]
pub struct EinnahmenAusgaben {
    pub membership_fees: Decimal,
    pub donations: Decimal,
    pub earmarked_donations: Decimal,
    pub subsidies: Decimal,
    pub other_income: Decimal,
    pub project_expenses: Decimal,
    pub personnel_costs: Decimal,
    pub material_admin_costs: Decimal,
    pub rental_costs: Decimal,
    pub other_expenses: Decimal,
    pub income_total: Decimal,
    pub expense_total: Decimal,
    pub surplus: Decimal,
}

/// Asset statement as of 31.12.{year}.
#[derive(Debug, Default)]
pub struct Vermoegensaufstellung {
    pub bank_balance: Decimal,
    pub inventory_value: Decimal,
    pub total_assets: Decimal,
}

/// User-authored narrative prose for one fiscal year, persisted in
/// `annual_report_draft`. Financial figures are recomputed from the live DB
/// at generation time; only the prose is stored here.
#[derive(Debug)]
pub struct AnnualReportDraft {
    pub year: i32,
    pub member_count: i64,
    pub meeting_date: String,
    pub meeting_time_from: String,
    pub meeting_time_to: String,
    pub tb_activities: String,
    pub tb_continuous: String,
    pub tb_outlook: String,
    pub pk_decisions: String,
    pub pk_activities_next_year: String,
}

impl AnnualReportDraft {
    pub fn default_for_year(year: i32) -> Self {
        Self {
            year,
            member_count: 0,
            meeting_date: String::new(),
            meeting_time_from: String::new(),
            meeting_time_to: String::new(),
            tb_activities: String::new(),
            tb_continuous: String::new(),
            tb_outlook: String::new(),
            pk_decisions: String::new(),
            pk_activities_next_year: String::new(),
        }
    }
}

/// All data required to render the four statutory PDFs for one fiscal year.
pub struct StatutoryReportData {
    pub year: i32,
    pub org_name: String,
    pub org_address: String,
    pub org_tax_number: String,
    pub einnahmen_ausgaben: EinnahmenAusgaben,
    pub vermoegen: Vermoegensaufstellung,
    /// Outbound events that fall within the fiscal year, oldest first.
    pub outbound_events: Vec<OutboundEventRow>,
    pub draft: AnnualReportDraft,
}

// ── Computation functions ────────────────────────────────────────────────────

/// Maps EUR ledger rows for the given year to the German statutory income/
/// expense categories.
///
/// Category mapping (this org only uses Ideeller Bereich):
/// - DonationIn  → donations
/// - SelfFundingIn → other_income
/// - PurchaseOut + TransferToBrlOut → project_expenses
pub fn compute_einnahmen_ausgaben(eur_rows: &[EurTxRow], year: i32) -> EinnahmenAusgaben {
    let from = format!("{year}-01-01");
    let to = format!("{year}-12-31");

    let mut ea = EinnahmenAusgaben::default();
    for r in eur_rows.iter().filter(|r| in_range(&r.date, &from, &to)) {
        use crate::model::transaction::EurTxType::*;
        match r.tx_type {
            DonationIn => ea.donations += r.amount,
            SelfFundingIn => ea.other_income += r.amount,
            PurchaseOut | TransferToBrlOut => ea.project_expenses += r.amount,
        }
    }
    ea.income_total =
        ea.membership_fees + ea.donations + ea.earmarked_donations + ea.subsidies + ea.other_income;
    ea.expense_total = ea.project_expenses
        + ea.personnel_costs
        + ea.material_admin_costs
        + ea.rental_costs
        + ea.other_expenses;
    ea.surplus = ea.income_total - ea.expense_total;
    ea
}

/// Computes the inventory asset value as the sum of purchase costs of all
/// available and reserved items, splitting the cost evenly across items when
/// a single purchase backs multiple items.
///
/// Items sourced from donations (no purchase cost) contribute zero.
/// `cost` is stored as TEXT/Decimal in the DB, so arithmetic is done in Rust.
pub fn compute_inventory_asset_value(conn: &Connection) -> rusqlite::Result<Decimal> {
    // For each purchase that backs at least one available/reserved item:
    //   per_item_value = purchase.cost / total_items_on_that_purchase
    //   contribution   = per_item_value * available_or_reserved_items_on_that_purchase
    //
    // Two separate counts are needed: total items (for cost allocation) and
    // available/reserved items (for which share of the cost we still own).
    let mut stmt = conn.prepare(
        "SELECT
             avail.source_purchase_id,
             avail.avail_count,
             total.total_count
         FROM (
             SELECT source_purchase_id, COUNT(*) AS avail_count
             FROM inventory_item
             WHERE source_type = 'purchase'
               AND status IN ('available', 'reserved')
             GROUP BY source_purchase_id
         ) avail
         JOIN (
             SELECT source_purchase_id, COUNT(*) AS total_count
             FROM inventory_item
             WHERE source_type = 'purchase'
             GROUP BY source_purchase_id
         ) total ON total.source_purchase_id = avail.source_purchase_id",
    )?;
    let groups: Vec<(i64, i64, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;

    // Only EUR purchases contribute to the EUR asset value — BRL purchases
    // would mix currencies and produce a meaningless total.
    let all_purchases = purchases::list(conn)?;
    let mut total = Decimal::ZERO;
    for (purchase_id, avail_count, total_count) in groups {
        if let Some(p) = all_purchases
            .iter()
            .find(|p| p.id == purchase_id && p.currency == crate::model::purchase::Currency::Eur)
        {
            if total_count > 0 {
                let per_item = p.cost / Decimal::from(total_count);
                total += per_item * Decimal::from(avail_count);
            }
        }
    }
    Ok(total)
}

/// Assembles all data needed to generate the four statutory reports for
/// `year`. Returns an error if any DB query fails.
pub fn load(conn: &Connection, year: i32) -> rusqlite::Result<StatutoryReportData> {
    let org_name = settings::get(conn, "org_name")?.unwrap_or_default();
    let org_address = settings::get(conn, "org_address")?.unwrap_or_default();
    let org_tax_number = settings::get(conn, "org_tax_number")?.unwrap_or_default();

    let eur_rows = eur_ledger::list(conn)?;

    // EUR balance = running total of all ledger rows up to and including year-end.
    let year_end = format!("{year}-12-31");
    let bank_balance = crate::reporting::compute_balance(
        eur_rows
            .iter()
            .filter(|r| r.date.as_str() <= year_end.as_str())
            .map(|r| (r.tx_type.is_inflow(), r.amount)),
    );

    let einnahmen_ausgaben = compute_einnahmen_ausgaben(&eur_rows, year);
    let inventory_value = compute_inventory_asset_value(conn)?;

    let vermoegen = Vermoegensaufstellung {
        bank_balance,
        inventory_value,
        total_assets: bank_balance + inventory_value,
    };

    let from = format!("{year}-01-01");
    let to = format!("{year}-12-31");
    let outbound_events: Vec<OutboundEventRow> = outbound::list(conn)?
        .into_iter()
        .filter(|e| in_range(&e.date, &from, &to))
        .collect();

    let draft = annual_report_draft::get(conn, year)?
        .unwrap_or_else(|| AnnualReportDraft::default_for_year(year));

    Ok(StatutoryReportData {
        year,
        org_name,
        org_address,
        org_tax_number,
        einnahmen_ausgaben,
        vermoegen,
        outbound_events,
        draft,
    })
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::mod_tests::open_full_db;
    use crate::db::queries::{
        eur_ledger as eur_qry, inventory, purchases as purchases_qry, transfers as transfers_qry,
    };
    use crate::model::{
        inventory::{InventoryItemDraft, ItemStatus, Location, SourceType},
        purchase::{Currency, PurchaseDraft, PurchaseStatus},
        transaction::{EurTxDraft, ManualEurTxType},
    };
    use rust_decimal_macros::dec;

    #[test]
    fn einnahmen_ausgaben_maps_eur_types_correctly() {
        let conn = open_full_db();
        // Donation: 1000 EUR
        eur_qry::insert(
            &conn,
            &EurTxDraft {
                date: "2025-03-01".to_string(),
                tx_type: ManualEurTxType::DonationIn,
                amount_str: "1000.00".to_string(),
                donor_id: None,
                note: String::new(),
            },
        )
        .unwrap();
        // Self-funding: 56 EUR
        eur_qry::insert(
            &conn,
            &EurTxDraft {
                date: "2025-06-01".to_string(),
                tx_type: ManualEurTxType::SelfFundingIn,
                amount_str: "56.00".to_string(),
                donor_id: None,
                note: String::new(),
            },
        )
        .unwrap();
        // Purchase out: 955 EUR (auto-created by purchases::insert)
        purchases_qry::insert(
            &conn,
            &PurchaseDraft {
                date: "2025-04-01".to_string(),
                currency: Currency::Eur,
                cost_str: "955.00".to_string(),
                channel: "Kleinanzeigen".to_string(),
                seller_info: String::new(),
                multiple_items: false,
                status: PurchaseStatus::Bought,
            },
        )
        .unwrap();

        let rows = eur_qry::list(&conn).unwrap();
        let ea = compute_einnahmen_ausgaben(&rows, 2025);

        assert_eq!(ea.donations, dec!(1000.00));
        assert_eq!(ea.other_income, dec!(56.00));
        assert_eq!(ea.project_expenses, dec!(955.00));
        assert_eq!(ea.income_total, dec!(1056.00));
        assert_eq!(ea.expense_total, dec!(955.00));
        assert_eq!(ea.surplus, dec!(101.00));
    }

    #[test]
    fn einnahmen_ausgaben_excludes_other_years() {
        let conn = open_full_db();
        eur_qry::insert(
            &conn,
            &EurTxDraft {
                date: "2024-12-31".to_string(),
                tx_type: ManualEurTxType::DonationIn,
                amount_str: "500.00".to_string(),
                donor_id: None,
                note: String::new(),
            },
        )
        .unwrap();
        eur_qry::insert(
            &conn,
            &EurTxDraft {
                date: "2026-01-01".to_string(),
                tx_type: ManualEurTxType::DonationIn,
                amount_str: "200.00".to_string(),
                donor_id: None,
                note: String::new(),
            },
        )
        .unwrap();
        let rows = eur_qry::list(&conn).unwrap();
        let ea = compute_einnahmen_ausgaben(&rows, 2025);
        assert_eq!(ea.income_total, Decimal::ZERO);
    }

    fn any_category_id(conn: &Connection) -> i64 {
        conn.query_row("SELECT id FROM category LIMIT 1", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn inventory_asset_value_splits_cost_for_multi_item_purchase() {
        let conn = open_full_db();
        let cat_id = any_category_id(&conn);

        // Purchase covering 2 items: cost 100 EUR → 50 EUR each
        let purchase_id = purchases_qry::insert(
            &conn,
            &PurchaseDraft {
                date: "2025-01-01".to_string(),
                currency: Currency::Eur,
                cost_str: "100.00".to_string(),
                channel: "test".to_string(),
                seller_info: String::new(),
                multiple_items: true,
                status: PurchaseStatus::Bought,
            },
        )
        .unwrap();

        for name in &["Deck A", "Deck B"] {
            inventory::insert(
                &conn,
                &InventoryItemDraft {
                    name: name.to_string(),
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
        }

        let value = compute_inventory_asset_value(&conn).unwrap();
        assert_eq!(value, dec!(100.00)); // 50 + 50 = 100
    }

    #[test]
    fn inventory_asset_value_excludes_donated_items() {
        let conn = open_full_db();
        let cat_id = any_category_id(&conn);

        let purchase_id = purchases_qry::insert(
            &conn,
            &PurchaseDraft {
                date: "2025-01-01".to_string(),
                currency: Currency::Eur,
                cost_str: "80.00".to_string(),
                channel: "test".to_string(),
                seller_info: String::new(),
                multiple_items: false,
                status: PurchaseStatus::Bought,
            },
        )
        .unwrap();

        // Available item: counts toward assets
        inventory::insert(
            &conn,
            &InventoryItemDraft {
                name: "Available item".to_string(),
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

        // A separate purchase whose item is already donated: must not count
        let purchase2_id = purchases_qry::insert(
            &conn,
            &PurchaseDraft {
                date: "2025-02-01".to_string(),
                currency: Currency::Eur,
                cost_str: "60.00".to_string(),
                channel: "test".to_string(),
                seller_info: String::new(),
                multiple_items: false,
                status: PurchaseStatus::Bought,
            },
        )
        .unwrap();
        let donated_id = inventory::insert(
            &conn,
            &InventoryItemDraft {
                name: "Donated item".to_string(),
                category_id: Some(cat_id),
                source_type: SourceType::Purchase,
                source_donation_id: None,
                source_purchase_id: Some(purchase2_id),
                location: Location::Brazil,
                status: ItemStatus::Available,
                notes: String::new(),
            },
        )
        .unwrap();
        conn.execute(
            "UPDATE inventory_item SET status = 'donated' WHERE id = ?1",
            [donated_id],
        )
        .unwrap();

        let value = compute_inventory_asset_value(&conn).unwrap();
        assert_eq!(value, dec!(80.00)); // only the available item
    }

    #[test]
    fn einnahmen_ausgaben_includes_transfer_to_brl_in_project_expenses() {
        use crate::model::transfer::TransferDraft;
        let conn = open_full_db();
        transfers_qry::insert(
            &conn,
            &TransferDraft {
                date: "2025-05-01".to_string(),
                eur_amount_sent_str: "400.00".to_string(),
                exchange_rate_str: "5.00".to_string(),
                notes: String::new(),
            },
        )
        .unwrap();
        let rows = eur_qry::list(&conn).unwrap();
        let ea = compute_einnahmen_ausgaben(&rows, 2025);
        assert_eq!(ea.project_expenses, dec!(400.00));
        assert_eq!(ea.expense_total, dec!(400.00));
    }

    #[test]
    fn load_assembles_all_fields_correctly() {
        use crate::db::queries::settings as settings_qry;
        let conn = open_full_db();
        settings_qry::set(&conn, "org_name", "Skateboard für alle").unwrap();
        settings_qry::set(&conn, "org_address", "Jahnstr. 2/1, 75397 Simmozheim").unwrap();
        eur_qry::insert(
            &conn,
            &EurTxDraft {
                date: "2025-03-01".to_string(),
                tx_type: ManualEurTxType::DonationIn,
                amount_str: "1056.00".to_string(),
                donor_id: None,
                note: String::new(),
            },
        )
        .unwrap();

        let data = load(&conn, 2025).unwrap();
        assert_eq!(data.year, 2025);
        assert_eq!(data.org_name, "Skateboard für alle");
        assert_eq!(data.einnahmen_ausgaben.income_total, dec!(1056.00));
        assert_eq!(data.draft.year, 2025);
        assert_eq!(data.vermoegen.bank_balance, dec!(1056.00));
    }
}
