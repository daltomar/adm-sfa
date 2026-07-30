use axum::extract::{Path, Query, State};
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

    // `qry::list` returns newest-first (shared with desktop, which relies on
    // that same order — reversed here, web-presentation-only, per the
    // user's request that the web list show oldest on top).
    let view_rows = rows
        .iter()
        .rev()
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

/// Prefills the create form after a round trip through "Create new donor"
/// (`/donors/new?return_to=...`) — `donors::create` appends `donor_id` to
/// whatever `return_to` URL was captured client-side, which for this page
/// also carries `date`/`amount_str`/`note` so the in-progress entry the
/// user was typing isn't lost. All fields are optional so a plain
/// `GET /eur-ledger/new` (no query string) behaves exactly as before.
#[derive(Deserialize)]
struct NewEntryQuery {
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    amount_str: Option<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    donor_id: Option<i64>,
}

async fn new_form(
    State(state): State<AppState>,
    Query(query): Query<NewEntryQuery>,
) -> impl IntoResponse {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    // Validate the id actually resolves before trusting it: a stale link or
    // a hand-edited query string with a nonexistent donor_id would otherwise
    // still force show_donor/donation_checked on, and most browsers default
    // an unmatched <select> to its first real option — silently attributing
    // the entry to the wrong donor if the user doesn't notice.
    let donor_id = query
        .donor_id
        .filter(|id| donors_qry::get(&conn, *id).ok().flatten().is_some());
    let show_donor = donor_id.is_some();
    let donors = donor_options(&conn, donor_id);
    HtmlTemplate(EurTxFormTemplate {
        id: None,
        date: query
            .date
            .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string()),
        type_label: None,
        show_donor,
        donation_checked: show_donor,
        self_funding_checked: false,
        amount_str: query.amount_str.unwrap_or_default(),
        donors,
        note: query.note.unwrap_or_default(),
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
        donation_checked: false,
        self_funding_checked: false,
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
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);

    let manual_type = match form.tx_type.as_str() {
        "donation_in" => ManualEurTxType::DonationIn,
        "self_funding_in" => ManualEurTxType::SelfFundingIn,
        _ => {
            let donors = donor_options(&conn, None);
            return HtmlTemplate(EurTxFormTemplate {
                id: None,
                date: form.date,
                type_label: None,
                show_donor: false,
                donation_checked: false,
                self_funding_checked: false,
                amount_str: form.amount_str,
                donors,
                note: form.note,
                error: Some(
                    rust_i18n::t!("web.eur_ledger.error.type_required", locale = &locale)
                        .to_string(),
                ),
                locale,
            })
            .into_response();
        }
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
    match qry::insert(&conn, &draft) {
        Ok(_) => Redirect::to("/eur-ledger").into_response(),
        Err(e) => {
            let donors = donor_options(&conn, draft.donor_id);
            HtmlTemplate(EurTxFormTemplate {
                id: None,
                date: draft.date,
                type_label: None,
                show_donor: manual_type == ManualEurTxType::DonationIn,
                donation_checked: manual_type == ManualEurTxType::DonationIn,
                self_funding_checked: manual_type == ManualEurTxType::SelfFundingIn,
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
                donation_checked: false,
                self_funding_checked: false,
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

#[cfg(test)]
mod tests {
    use crate::test_support;
    use adm_sfa_core::db::queries::{donors as donors_qry, eur_ledger as qry};
    use adm_sfa_core::model::donor::DonorDraft;
    use adm_sfa_core::model::transaction::EurTxType;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};

    /// A valid `tx_type` should still redirect and insert a row — guards
    /// the happy path through `create()`'s restructured `match` (previously
    /// an `if/else`) now that an early-return error arm exists alongside it.
    #[tokio::test]
    async fn create_with_a_valid_tx_type_redirects_and_inserts_a_row() {
        let (state, dir) = test_support::test_app("eur-ledger-valid-tx-type");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body = "date=2026-01-01&tx_type=donation_in&amount_str=10.00&note=";
        let req = Request::builder()
            .method("POST")
            .uri("/eur-ledger")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let rows = qry::list(&state.conn()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].tx_type, EurTxType::DonationIn);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A `donor_id` submitted alongside `tx_type=self_funding_in` must never
    /// leak into the persisted row — `create()` forces `donor_id` to `None`
    /// unless `manual_type == DonationIn`, keyed off the authoritative
    /// parsed type rather than the raw form value. The donor `<select>` is
    /// always present in the DOM now (hidden via inline style, not removed
    /// via Askama) so the JS toggle works, which is exactly why this guard
    /// matters: a hidden-but-present field still submits its value.
    #[tokio::test]
    async fn create_ignores_a_submitted_donor_id_for_self_funding() {
        let (state, dir) = test_support::test_app("eur-ledger-self-funding-donor-leak");
        let donor_id = donors_qry::insert(
            &state.conn(),
            &DonorDraft {
                name: "Alex".to_string(),
                contact_info: String::new(),
                notes: String::new(),
            },
        )
        .unwrap();
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body = format!(
            "date=2026-01-01&tx_type=self_funding_in&amount_str=10.00&donor_id={donor_id}&note="
        );
        let req = Request::builder()
            .method("POST")
            .uri("/eur-ledger")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let rows = qry::list(&state.conn()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].donor_id, None);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Regression coverage for making the "Typ" radio actually mandatory:
    /// a missing `tx_type` used to silently default to Donation server-side
    /// even though the HTML `required` attribute is trivially bypassable.
    #[tokio::test]
    async fn create_rejects_a_missing_tx_type_and_inserts_nothing() {
        let (state, dir) = test_support::test_app("eur-ledger-missing-tx-type");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body = "date=2026-01-01&amount_str=10.00&note=";
        let req = Request::builder()
            .method("POST")
            .uri("/eur-ledger")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body_text = test_support::body_text(res).await;
        assert!(body_text.contains("Please select a type."));
        assert!(qry::list(&state.conn()).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Same guard, but for a non-empty value that isn't one of the two
    /// allowed `tx_type`s — confirms the catch-all arm rejects garbage too,
    /// not just the empty-string default case above.
    #[tokio::test]
    async fn create_rejects_an_invalid_tx_type_and_inserts_nothing() {
        let (state, dir) = test_support::test_app("eur-ledger-invalid-tx-type");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body = "date=2026-01-01&tx_type=nonsense-value&amount_str=10.00&note=";
        let req = Request::builder()
            .method("POST")
            .uri("/eur-ledger")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body_text = test_support::body_text(res).await;
        assert!(body_text.contains("Please select a type."));
        assert!(qry::list(&state.conn()).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The other half of the "Create new donor" round trip (donors.rs's
    /// `create()` appends `donor_id`, tested there): this page's own
    /// `?donor_id=` prefill should pre-select the donor, show the donor
    /// field, and mark Donation as checked, without the caller needing to
    /// re-pick anything.
    #[tokio::test]
    async fn new_form_with_a_donor_id_query_param_preselects_and_shows_donor() {
        let (state, dir) = test_support::test_app("eur-ledger-new-form-donor-prefill");
        let donor_id = donors_qry::insert(
            &state.conn(),
            &DonorDraft {
                name: "Alex".to_string(),
                contact_info: String::new(),
                notes: String::new(),
            },
        )
        .unwrap();
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("GET")
            .uri(format!(
                "/eur-ledger/new?date=2026-02-02&amount_str=15.00&note=hi&donor_id={donor_id}"
            ))
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body_text = test_support::body_text(res).await;
        assert!(body_text.contains(r#"value="2026-02-02""#));
        assert!(body_text.contains(r#"value="15.00""#));
        assert!(body_text.contains(">hi<"));
        assert!(body_text.contains(&format!(r#"value="{donor_id}" selected"#)));
        assert!(body_text.contains(r#"value="donation_in" required checked"#));
        assert!(!body_text.contains(r#"id="donor_field" style="display:none""#));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A `donor_id` that doesn't resolve to a real donor (stale link, or a
    /// hand-edited query string) must not force the donor field open —
    /// otherwise an unmatched `<select>` would default to its first real
    /// option in most browsers, silently attributing the entry to the wrong
    /// donor.
    #[tokio::test]
    async fn new_form_with_a_nonexistent_donor_id_does_not_show_the_donor_field() {
        let (state, dir) = test_support::test_app("eur-ledger-new-form-bad-donor-id");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("GET")
            .uri("/eur-ledger/new?donor_id=999999")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body_text = test_support::body_text(res).await;
        assert!(body_text.contains(r#"id="donor_field" style="display:none""#));
        assert!(!body_text.contains(r#"value="donation_in" required checked"#));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The list page should show the oldest entry first (per the user's
    /// request), reversing `core::db::queries::eur_ledger::list()`'s
    /// newest-first SQL order in the web presentation layer only — desktop
    /// shares the same query and is intentionally left unaffected.
    #[tokio::test]
    async fn list_shows_the_oldest_entry_first() {
        use adm_sfa_core::model::transaction::{EurTxDraft, ManualEurTxType};

        let (state, dir) = test_support::test_app("eur-ledger-list-oldest-first");
        let conn = state.conn();
        qry::insert(
            &conn,
            &EurTxDraft {
                date: "2026-01-01".to_string(),
                tx_type: ManualEurTxType::SelfFundingIn,
                amount_str: "10.00".to_string(),
                donor_id: None,
                note: "older".to_string(),
            },
        )
        .unwrap();
        qry::insert(
            &conn,
            &EurTxDraft {
                date: "2026-06-01".to_string(),
                tx_type: ManualEurTxType::SelfFundingIn,
                amount_str: "20.00".to_string(),
                donor_id: None,
                note: "newer".to_string(),
            },
        )
        .unwrap();
        drop(conn);
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("GET")
            .uri("/eur-ledger")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;
        let body_text = test_support::body_text(res).await;

        let older_pos = body_text.find("older").unwrap();
        let newer_pos = body_text.find("newer").unwrap();
        assert!(older_pos < newer_pos);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A self-funding entry with no note has nothing to show in the
    /// Description column — the edit link used to wrap that (possibly
    /// empty) text, making the row unclickable. It should fall back to a
    /// visible, translated "Edit" label instead.
    #[tokio::test]
    async fn list_shows_an_edit_link_for_a_self_funding_entry_with_no_note() {
        use adm_sfa_core::model::transaction::{EurTxDraft, ManualEurTxType};

        let (state, dir) = test_support::test_app("eur-ledger-list-no-note-edit-link");
        let conn = state.conn();
        qry::insert(
            &conn,
            &EurTxDraft {
                date: "2026-01-01".to_string(),
                tx_type: ManualEurTxType::SelfFundingIn,
                amount_str: "10.00".to_string(),
                donor_id: None,
                note: String::new(),
            },
        )
        .unwrap();
        drop(conn);
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("GET")
            .uri("/eur-ledger")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;
        let body_text = test_support::body_text(res).await;

        assert!(body_text.contains(r#"<a href="/eur-ledger/1/edit">Edit</a>"#));
        std::fs::remove_dir_all(&dir).ok();
    }
}
