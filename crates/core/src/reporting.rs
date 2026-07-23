//! Report aggregation — the pure "what does this data add up to" logic
//! behind `desktop`'s Reports view, extracted per CLAUDE.md phase 3 so a
//! future `web` front-end gets the same figures without reimplementing
//! them. No egui, no rendering: every function here takes plain data in and
//! returns plain data out (structs, `Vec`s, `String`s) for a caller to
//! format however its own UI needs. CSV/PDF export-formatting glue
//! (locale/German-CSV branching, header translation) stays in `desktop`'s
//! `ui/views/reports.rs`, which calls into this module for the aggregation
//! step and does its own presentation on top.

use std::collections::HashMap;

use rust_decimal::Decimal;
use rust_i18n::t;

use crate::model::donor::{Donor, PhysicalDonation};
use crate::model::outbound::OutboundEventRow;
use crate::model::transaction::{BrlTxRow, BrlTxType, EurTxRow, EurTxType};

/// True if `date` falls within `[from, to]`, treating an empty bound as "no
/// limit" on that side.
pub fn in_range(date: &str, from: &str, to: &str) -> bool {
    (from.is_empty() || date >= from) && (to.is_empty() || date <= to)
}

/// Sums a ledger's inflows minus its outflows. The one shared implementation
/// behind what used to be three independent copies: `eur_ledger.rs` and
/// `brl_ledger.rs`'s full-history balance, and a third period-scoped
/// variant inline in `reports.rs`. Callers map their own row type into
/// `(is_inflow, amount)` pairs.
pub fn compute_balance(flows: impl Iterator<Item = (bool, Decimal)>) -> Decimal {
    flows.fold(Decimal::ZERO, |acc, (is_inflow, amount)| {
        if is_inflow {
            acc + amount
        } else {
            acc - amount
        }
    })
}

pub fn donor_or_anonymous(name: &Option<String>) -> String {
    name.clone()
        .unwrap_or_else(|| t!("common.anonymous").into_owned())
}

pub fn eur_tx_description(r: &EurTxRow) -> String {
    match r.tx_type {
        EurTxType::DonationIn => r
            .donor_name
            .clone()
            .unwrap_or_else(|| t!("common.anonymous").into_owned()),
        EurTxType::SelfFundingIn => r.note.clone().unwrap_or_default(),
        EurTxType::PurchaseOut => r.purchase_channel.clone().unwrap_or_default(),
        EurTxType::TransferToBrlOut => t!("reports.tx.eur_to_brl_transfer").into_owned(),
    }
}

pub fn brl_tx_description(r: &BrlTxRow) -> String {
    match r.tx_type {
        BrlTxType::TransferIn => t!("reports.tx.eur_to_brl_transfer").into_owned(),
        BrlTxType::BrazilPurchaseOut => r.purchase_channel.clone().unwrap_or_default(),
        BrlTxType::CashGiftOut => r.recipient_name.clone().unwrap_or_default(),
    }
}

pub struct DonorRow {
    pub name: String,
    pub cash_count: i64,
    pub cash_total: Decimal,
    pub item_count: i64,
}

/// Per-donor cash + physical-donation totals within `[date_from, date_to]`,
/// plus a trailing row for anonymous activity. Donors with zero activity in
/// range are omitted entirely, and the anonymous row only appears if there
/// was any anonymous activity.
pub fn build_donor_rows(
    donors: &[Donor],
    eur_rows: &[EurTxRow],
    donations: &[PhysicalDonation],
    date_from: &str,
    date_to: &str,
) -> Vec<DonorRow> {
    let mut rows: Vec<DonorRow> = Vec::new();
    for donor in donors {
        let cash: Vec<&EurTxRow> = eur_rows
            .iter()
            .filter(|r| {
                r.tx_type == EurTxType::DonationIn
                    && r.donor_id == Some(donor.id)
                    && in_range(&r.date, date_from, date_to)
            })
            .collect();
        let item_count = donations
            .iter()
            .filter(|d| {
                d.donor_id == Some(donor.id) && in_range(&d.date_received, date_from, date_to)
            })
            .count() as i64;
        if cash.is_empty() && item_count == 0 {
            continue;
        }
        let cash_total: Decimal = cash.iter().map(|r| r.amount).sum();
        rows.push(DonorRow {
            name: donor.name.clone(),
            cash_count: cash.len() as i64,
            cash_total,
            item_count,
        });
    }
    let anon_cash: Vec<&EurTxRow> = eur_rows
        .iter()
        .filter(|r| {
            r.tx_type == EurTxType::DonationIn
                && r.donor_id.is_none()
                && in_range(&r.date, date_from, date_to)
        })
        .collect();
    let anon_items = donations
        .iter()
        .filter(|d| d.donor_id.is_none() && in_range(&d.date_received, date_from, date_to))
        .count() as i64;
    if !anon_cash.is_empty() || anon_items > 0 {
        let cash_total: Decimal = anon_cash.iter().map(|r| r.amount).sum();
        rows.push(DonorRow {
            name: t!("common.anonymous").into_owned(),
            cash_count: anon_cash.len() as i64,
            cash_total,
            item_count: anon_items,
        });
    }
    rows
}

pub struct EurSummary {
    pub starting_balance: Decimal,
    pub donation_count: i64,
    pub donation_total: Decimal,
    pub self_funding_count: i64,
    pub self_funding_total: Decimal,
    pub purchase_count: i64,
    pub purchase_total: Decimal,
    pub transfer_count: i64,
    pub transfer_total: Decimal,
    pub net: Decimal,
    pub ending_balance: Decimal,
}

/// EUR ledger summary for `[date_from, date_to]`: starting balance (the
/// running balance immediately before `date_from`), per-type counts/totals
/// within the period, net change, and ending balance.
pub fn eur_summary(rows: &[EurTxRow], date_from: &str, date_to: &str) -> EurSummary {
    let starting_balance = compute_balance(
        rows.iter()
            .filter(|r| !date_from.is_empty() && r.date.as_str() < date_from)
            .map(|r| (r.tx_type.is_inflow(), r.amount)),
    );

    let period: Vec<&EurTxRow> = rows
        .iter()
        .filter(|r| in_range(&r.date, date_from, date_to))
        .collect();

    let sum_for = |t: EurTxType| -> (i64, Decimal) {
        let matching: Vec<&&EurTxRow> = period.iter().filter(|r| r.tx_type == t).collect();
        (
            matching.len() as i64,
            matching.iter().map(|r| r.amount).sum(),
        )
    };

    let (donation_count, donation_total) = sum_for(EurTxType::DonationIn);
    let (self_funding_count, self_funding_total) = sum_for(EurTxType::SelfFundingIn);
    let (purchase_count, purchase_total) = sum_for(EurTxType::PurchaseOut);
    let (transfer_count, transfer_total) = sum_for(EurTxType::TransferToBrlOut);

    let net = donation_total + self_funding_total - purchase_total - transfer_total;
    let ending_balance = starting_balance + net;

    EurSummary {
        starting_balance,
        donation_count,
        donation_total,
        self_funding_count,
        self_funding_total,
        purchase_count,
        purchase_total,
        transfer_count,
        transfer_total,
        net,
        ending_balance,
    }
}

pub struct BrlSummary {
    pub starting_balance: Decimal,
    pub transfer_in_count: i64,
    pub transfer_in_total: Decimal,
    pub purchase_count: i64,
    pub purchase_total: Decimal,
    pub gift_count: i64,
    pub gift_total: Decimal,
    pub net: Decimal,
    pub ending_balance: Decimal,
}

/// BRL ledger summary for `[date_from, date_to]` — same shape as
/// `eur_summary`, over `BrlTxType`'s three variants instead of EUR's four.
pub fn brl_summary(rows: &[BrlTxRow], date_from: &str, date_to: &str) -> BrlSummary {
    let starting_balance = compute_balance(
        rows.iter()
            .filter(|r| !date_from.is_empty() && r.date.as_str() < date_from)
            .map(|r| (r.tx_type.is_inflow(), r.amount)),
    );

    let period: Vec<&BrlTxRow> = rows
        .iter()
        .filter(|r| in_range(&r.date, date_from, date_to))
        .collect();

    let sum_for = |t: BrlTxType| -> (i64, Decimal) {
        let matching: Vec<&&BrlTxRow> = period.iter().filter(|r| r.tx_type == t).collect();
        (
            matching.len() as i64,
            matching.iter().map(|r| r.amount).sum(),
        )
    };

    let (transfer_in_count, transfer_in_total) = sum_for(BrlTxType::TransferIn);
    let (purchase_count, purchase_total) = sum_for(BrlTxType::BrazilPurchaseOut);
    let (gift_count, gift_total) = sum_for(BrlTxType::CashGiftOut);

    let net = transfer_in_total - purchase_total - gift_total;
    let ending_balance = starting_balance + net;

    BrlSummary {
        starting_balance,
        transfer_in_count,
        transfer_in_total,
        purchase_count,
        purchase_total,
        gift_count,
        gift_total,
        net,
        ending_balance,
    }
}

pub struct AuditEntry {
    pub date: String,
    /// EUR/BRL rows only: pre-resolved at the ambient UI locale (via
    /// `r.tx_type.label()` for `kind`; `ledger` is always the fixed
    /// currency code "EUR"/"BRL", never translated). A disclosed, minor
    /// limitation — unlike everything else in this struct, this doesn't
    /// follow an export's explicit chosen locale. `None` for outbound rows,
    /// which use `outbound` below instead: those needed a real fix (not
    /// just a disclosed limitation) since Ledger/Type/Description are full
    /// locale-sensitive prose there, not a single enum label.
    pub ledger_kind: Option<(String, String)>,
    /// EUR/BRL only (donor name / purchase channel / note — mostly DB user
    /// data, not translated prose, see `eur_tx_description`/
    /// `brl_tx_description`). Empty for outbound rows.
    pub description: String,
    /// Outbound rows only — raw pieces, not pre-formatted text, so each
    /// consumer (on-screen table vs CSV vs PDF) can build Ledger/Type/
    /// Description in its own target locale via `outbound_audit_text()`
    /// instead of a value baked once at whichever locale was ambient when
    /// `build_audit_entries` ran.
    pub outbound: Option<OutboundAuditInfo>,
    pub amount: Option<AuditAmount>,
    pub docs: i64,
}

pub struct OutboundAuditInfo {
    pub item_count: i64,
    pub recipient_name: String,
    pub cash: Option<Decimal>,
}

pub struct AuditAmount {
    pub sign: &'static str,
    pub symbol: &'static str,
    pub value: Decimal,
}

/// Unified audit trail across all three ledgers/events for
/// `[date_from, date_to]`, optionally scoped to one recipient project
/// (`recipient_filter`, which only affects BRL cash-gift rows and outbound
/// events — EUR/BRL rows unrelated to a recipient are unaffected). Sorted
/// newest-first.
#[allow(clippy::too_many_arguments)]
pub fn build_audit_entries(
    eur_rows: &[EurTxRow],
    brl_rows: &[BrlTxRow],
    outbound_rows: &[OutboundEventRow],
    doc_counts: &HashMap<(String, i64), i64>,
    date_from: &str,
    date_to: &str,
    recipient_filter: Option<i64>,
) -> Vec<AuditEntry> {
    let mut entries: Vec<AuditEntry> = Vec::new();

    for r in eur_rows {
        if !in_range(&r.date, date_from, date_to) {
            continue;
        }
        let docs = match r.tx_type {
            EurTxType::PurchaseOut => r
                .linked_purchase_id
                .and_then(|id| doc_counts.get(&("purchase".to_string(), id)).copied())
                .unwrap_or(0),
            EurTxType::TransferToBrlOut => r
                .linked_transfer_id
                .and_then(|id| doc_counts.get(&("transfer".to_string(), id)).copied())
                .unwrap_or(0),
            _ => 0,
        };
        let description = eur_tx_description(r);
        let sign = if r.tx_type.is_inflow() { "+" } else { "-" };
        entries.push(AuditEntry {
            date: r.date.clone(),
            ledger_kind: Some(("EUR".to_string(), r.tx_type.label())),
            description,
            outbound: None,
            amount: Some(AuditAmount {
                sign,
                symbol: "€",
                value: r.amount,
            }),
            docs,
        });
    }

    for r in brl_rows {
        if !in_range(&r.date, date_from, date_to) {
            continue;
        }
        if let (Some(rid), BrlTxType::CashGiftOut) = (recipient_filter, r.tx_type) {
            let matches = outbound_rows
                .iter()
                .find(|e| Some(e.id) == r.linked_outbound_event_id)
                .map(|e| e.recipient_project_id == rid)
                .unwrap_or(false);
            if !matches {
                continue;
            }
        }
        let docs = match r.tx_type {
            BrlTxType::BrazilPurchaseOut => r
                .linked_purchase_id
                .and_then(|id| doc_counts.get(&("purchase".to_string(), id)).copied())
                .unwrap_or(0),
            BrlTxType::TransferIn => r
                .linked_transfer_id
                .and_then(|id| doc_counts.get(&("transfer".to_string(), id)).copied())
                .unwrap_or(0),
            BrlTxType::CashGiftOut => 0,
        };
        let description = brl_tx_description(r);
        let sign = if r.tx_type.is_inflow() { "+" } else { "-" };
        entries.push(AuditEntry {
            date: r.date.clone(),
            ledger_kind: Some(("BRL".to_string(), r.tx_type.label())),
            description,
            outbound: None,
            amount: Some(AuditAmount {
                sign,
                symbol: "R$",
                value: r.amount,
            }),
            docs,
        });
    }

    for e in outbound_rows {
        if !in_range(&e.date, date_from, date_to) {
            continue;
        }
        if let Some(rid) = recipient_filter {
            if e.recipient_project_id != rid {
                continue;
            }
        }
        let outbound_cash = e.cash_amount_brl.filter(|&cash| cash > Decimal::ZERO);
        entries.push(AuditEntry {
            date: e.date.clone(),
            ledger_kind: None,
            description: String::new(),
            outbound: Some(OutboundAuditInfo {
                item_count: e.item_count,
                recipient_name: e.recipient_name.clone(),
                cash: outbound_cash,
            }),
            amount: None,
            docs: 0,
        });
    }

    entries.sort_by(|a, b| b.date.cmp(&a.date));
    entries
}

/// Builds an outbound audit row's Ledger/Type/Description text in an
/// explicit `locale` — fixes a bug where this used to be baked once inside
/// `build_audit_entries` at the ambient UI locale, so a CSV/PDF export
/// chosen in a different language still showed these three columns in
/// whatever language happened to be active in the UI at the time.
pub fn outbound_audit_text(
    info: &OutboundAuditInfo,
    locale: &str,
    fmt_cash: impl Fn(Decimal) -> String,
) -> (String, String, String) {
    let ledger = t!("sidebar.outbound", locale = locale).into_owned();
    let kind = t!("status.source_type.donation", locale = locale).into_owned();
    let mut description = if info.item_count == 1 {
        t!(
            "reports.audit.outbound_desc_one",
            locale = locale,
            recipient = info.recipient_name.as_str()
        )
        .into_owned()
    } else {
        t!(
            "reports.audit.outbound_desc_other",
            locale = locale,
            count = info.item_count,
            recipient = info.recipient_name.as_str()
        )
        .into_owned()
    };
    if let Some(cash) = info.cash {
        description.push_str(&t!(
            "reports.audit.outbound_cash_suffix",
            locale = locale,
            cash = fmt_cash(cash)
        ));
    }
    (ledger, kind, description)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn eur_row(
        id: i64,
        date: &str,
        tx_type: EurTxType,
        amount: Decimal,
        donor_id: Option<i64>,
    ) -> EurTxRow {
        EurTxRow {
            id,
            date: date.to_string(),
            tx_type,
            amount,
            donor_id,
            donor_name: None,
            purchase_channel: None,
            note: None,
            linked_purchase_id: None,
            linked_transfer_id: None,
        }
    }

    fn brl_row(id: i64, date: &str, tx_type: BrlTxType, amount: Decimal) -> BrlTxRow {
        BrlTxRow {
            id,
            date: date.to_string(),
            tx_type,
            amount,
            note: None,
            linked_transfer_id: None,
            linked_purchase_id: None,
            linked_outbound_event_id: None,
            purchase_channel: None,
            recipient_name: None,
        }
    }

    #[test]
    fn in_range_treats_empty_bound_as_unbounded() {
        assert!(in_range("2026-06-15", "", ""));
        assert!(in_range("2026-06-15", "2026-01-01", ""));
        assert!(in_range("2026-06-15", "", "2026-12-31"));
    }

    #[test]
    fn in_range_is_inclusive_on_both_boundaries() {
        assert!(in_range("2026-01-01", "2026-01-01", "2026-12-31"));
        assert!(in_range("2026-12-31", "2026-01-01", "2026-12-31"));
        assert!(!in_range("2025-12-31", "2026-01-01", "2026-12-31"));
        assert!(!in_range("2027-01-01", "2026-01-01", "2026-12-31"));
    }

    #[test]
    fn compute_balance_sums_inflows_minus_outflows() {
        let flows = vec![(true, dec!(100)), (false, dec!(30)), (true, dec!(5))];
        assert_eq!(compute_balance(flows.into_iter()), dec!(75));
    }

    #[test]
    fn donor_or_anonymous_falls_back_on_none() {
        assert_eq!(donor_or_anonymous(&Some("Ana".to_string())), "Ana");
        assert_eq!(donor_or_anonymous(&None), "Anonymous");
    }

    #[test]
    fn build_donor_rows_omits_donors_with_no_activity_in_range() {
        let donors = vec![
            Donor {
                id: 1,
                name: "Active".to_string(),
                contact_info: None,
                notes: None,
            },
            Donor {
                id: 2,
                name: "Inactive".to_string(),
                contact_info: None,
                notes: None,
            },
        ];
        let eur_rows = vec![eur_row(
            1,
            "2026-06-01",
            EurTxType::DonationIn,
            dec!(50),
            Some(1),
        )];
        let rows = build_donor_rows(&donors, &eur_rows, &[], "", "");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Active");
        assert_eq!(rows[0].cash_total, dec!(50));
    }

    #[test]
    fn build_donor_rows_buckets_anonymous_activity_separately() {
        let eur_rows = vec![eur_row(
            1,
            "2026-06-01",
            EurTxType::DonationIn,
            dec!(20),
            None,
        )];
        let rows = build_donor_rows(&[], &eur_rows, &[], "", "");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Anonymous");
        assert_eq!(rows[0].cash_total, dec!(20));
    }

    #[test]
    fn build_donor_rows_respects_date_range() {
        let donors = vec![Donor {
            id: 1,
            name: "Donor".to_string(),
            contact_info: None,
            notes: None,
        }];
        let eur_rows = vec![eur_row(
            1,
            "2026-01-01",
            EurTxType::DonationIn,
            dec!(50),
            Some(1),
        )];
        let rows = build_donor_rows(&donors, &eur_rows, &[], "2026-06-01", "2026-12-31");
        assert!(rows.is_empty(), "donation before the range must not count");
    }

    #[test]
    fn eur_summary_starting_balance_is_the_pre_range_running_total() {
        let rows = vec![
            eur_row(1, "2026-01-01", EurTxType::DonationIn, dec!(100), None),
            eur_row(2, "2026-06-01", EurTxType::PurchaseOut, dec!(30), None),
        ];
        let s = eur_summary(&rows, "2026-06-01", "2026-12-31");
        assert_eq!(
            s.starting_balance,
            dec!(100),
            "only the Jan row precedes the range"
        );
        assert_eq!(s.purchase_count, 1);
        assert_eq!(s.purchase_total, dec!(30));
        assert_eq!(s.net, dec!(-30));
        assert_eq!(s.ending_balance, dec!(70));
    }

    #[test]
    fn eur_summary_with_no_lower_bound_has_zero_starting_balance() {
        let rows = vec![eur_row(
            1,
            "2026-01-01",
            EurTxType::DonationIn,
            dec!(100),
            None,
        )];
        let s = eur_summary(&rows, "", "");
        assert_eq!(s.starting_balance, dec!(0));
        assert_eq!(s.donation_count, 1);
        assert_eq!(s.ending_balance, dec!(100));
    }

    #[test]
    fn brl_summary_computes_net_across_all_three_types() {
        let rows = vec![
            brl_row(1, "2026-01-01", BrlTxType::TransferIn, dec!(1000)),
            brl_row(2, "2026-01-02", BrlTxType::BrazilPurchaseOut, dec!(200)),
            brl_row(3, "2026-01-03", BrlTxType::CashGiftOut, dec!(50)),
        ];
        let s = brl_summary(&rows, "", "");
        assert_eq!(s.net, dec!(750));
        assert_eq!(s.ending_balance, dec!(750));
    }

    #[test]
    fn build_audit_entries_sorts_newest_first_and_looks_up_docs() {
        let eur_rows = vec![
            eur_row(1, "2026-01-01", EurTxType::PurchaseOut, dec!(30), None)
                .tap(|r| r.linked_purchase_id = Some(9)),
        ];
        let brl_rows = vec![brl_row(1, "2026-06-01", BrlTxType::TransferIn, dec!(100))];
        let doc_counts = HashMap::from([(("purchase".to_string(), 9i64), 3i64)]);

        let entries = build_audit_entries(&eur_rows, &brl_rows, &[], &doc_counts, "", "", None);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].date, "2026-06-01", "newest first");
        assert_eq!(entries[1].date, "2026-01-01");
        assert_eq!(
            entries[1].docs, 3,
            "doc count looked up via linked_purchase_id"
        );
    }

    #[test]
    fn build_audit_entries_recipient_filter_only_affects_cash_gift_rows() {
        let brl_rows = vec![
            brl_row(1, "2026-01-01", BrlTxType::CashGiftOut, dec!(50))
                .tap(|r| r.linked_outbound_event_id = Some(1)),
            brl_row(2, "2026-01-02", BrlTxType::TransferIn, dec!(1000)),
        ];
        let outbound_rows = vec![OutboundEventRow {
            id: 1,
            date: "2026-01-01".to_string(),
            recipient_project_id: 42,
            recipient_name: "Other Project".to_string(),
            cash_amount_brl: Some(dec!(50)),
            notes: None,
            item_count: 1,
        }];
        let doc_counts = HashMap::new();

        // Filtering by a different recipient (7) must drop the cash-gift row
        // (linked to project 42) but keep the unrelated TransferIn row.
        let entries =
            build_audit_entries(&[], &brl_rows, &outbound_rows, &doc_counts, "", "", Some(7));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].date, "2026-01-02");
    }

    #[test]
    fn outbound_audit_text_uses_singular_for_one_item() {
        let info = OutboundAuditInfo {
            item_count: 1,
            recipient_name: "Projeto Teste".to_string(),
            cash: None,
        };
        let (_, _, description) = outbound_audit_text(&info, "en", |v| v.to_string());
        assert!(description.contains("Projeto Teste"));
        assert!(!description.contains("cash gift"));
    }

    #[test]
    fn outbound_audit_text_appends_cash_suffix_when_present() {
        let info = OutboundAuditInfo {
            item_count: 2,
            recipient_name: "Projeto Teste".to_string(),
            cash: Some(dec!(5)),
        };
        let (_, _, description) = outbound_audit_text(&info, "en", |v| format!("{v:.2}"));
        assert!(description.contains("cash gift"), "{description}");
        assert!(description.contains("5.00"), "{description}");
    }

    /// Small helper for building a test row and then mutating one field
    /// inline, since these fixtures are otherwise all-positional struct
    /// literals with many fields most tests don't care about.
    trait Tap: Sized {
        fn tap(mut self, f: impl FnOnce(&mut Self)) -> Self {
            f(&mut self);
            self
        }
    }
    impl<T> Tap for T {}
}
