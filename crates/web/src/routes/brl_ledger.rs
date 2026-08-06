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
    let balance_display = format::amount_in(balance, &locale);

    // `qry::list` returns newest-first (shared with desktop, which relies on
    // that same order — reversed here, web-presentation-only, matching the
    // precedent set for the EUR Ledger list page).
    let view_rows = rows
        .iter()
        .rev()
        .map(|r| BrlLedgerRow {
            date_display: format::date(&r.date),
            type_label: type_label(r.tx_type, &locale),
            sign: if r.tx_type.is_inflow() { "+" } else { "-" },
            amount_display: format::amount_in(r.amount, &locale),
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

#[cfg(test)]
mod tests {
    use crate::test_support;
    use adm_sfa_core::db::queries::{purchases as purchases_qry, settings as settings_qry};
    use adm_sfa_core::model::purchase::{Currency, PurchaseDraft, PurchaseStatus};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};

    /// Regression coverage for CLAUDE.md's "Sorting" backlog item, mirroring
    /// `eur_ledger.rs`'s `list_shows_the_oldest_entry_first`. `brl_transaction`
    /// rows have no direct `insert` — they're only ever created as a side
    /// effect of a BRL-currency purchase, so that's what seeds this test.
    #[tokio::test]
    async fn list_shows_the_oldest_entry_first() {
        let (state, dir) = test_support::test_app("brl-ledger-list-oldest-first");
        let conn = state.conn();
        purchases_qry::insert(
            &conn,
            &PurchaseDraft {
                date: "2026-01-01".to_string(),
                currency: Currency::Brl,
                cost_str: "111.11".to_string(),
                channel: "older".to_string(),
                seller_info: String::new(),
                multiple_items: false,
                status: PurchaseStatus::Bought,
            },
        )
        .unwrap();
        purchases_qry::insert(
            &conn,
            &PurchaseDraft {
                date: "2026-06-01".to_string(),
                currency: Currency::Brl,
                cost_str: "222.22".to_string(),
                channel: "newer".to_string(),
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
            .uri("/brl-ledger")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        let older_pos = body.find("older").expect("older channel not found");
        let newer_pos = body.find("newer").expect("newer channel not found");
        assert!(
            older_pos < newer_pos,
            "expected the older transaction to render before the newer one"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Regression test: `list()` used to call `format::amount()` (the
    /// *ambient* `rust_i18n::locale()`, which `web` never sets — see
    /// `core::format::amount()`'s doc comment) instead of
    /// `format::amount_in(value, &locale)` (the already-resolved
    /// `ui_locale`). With `ui_locale` set to German, both the row amount and
    /// the balance line must render with a comma decimal separator
    /// (`1.234,56`), not the English default (`1,234.56`).
    #[tokio::test]
    async fn list_formats_amounts_using_the_resolved_ui_locale() {
        let (state, dir) = test_support::test_app("brl-ledger-locale-amount-format");
        let conn = state.conn();
        settings_qry::set(&conn, "ui_locale", "de").unwrap();
        purchases_qry::insert(
            &conn,
            &PurchaseDraft {
                date: "2026-01-01".to_string(),
                currency: Currency::Brl,
                cost_str: "1234.56".to_string(),
                channel: "TestChannel".to_string(),
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
            .uri("/brl-ledger")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        assert!(
            body.contains("1.234,56"),
            "expected the German-formatted amount in the body: {body}"
        );
        assert!(
            !body.contains("1,234.56"),
            "amount rendered in English format despite ui_locale=de: {body}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
