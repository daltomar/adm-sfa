use askama::Template;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

/// Wraps any Askama `Template` so route handlers can just `return
/// HtmlTemplate(SomeTemplate { .. })` and get a working `IntoResponse` —
/// Askama itself doesn't implement `IntoResponse` (it's framework-agnostic
/// by design), so every web framework using it needs this same few-line
/// adapter.
pub struct HtmlTemplate<T>(pub T);

impl<T: Template> IntoResponse for HtmlTemplate<T> {
    fn into_response(self) -> Response {
        match self.0.render() {
            Ok(html) => Html(html).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("template render error: {e}"),
            )
                .into_response(),
        }
    }
}

#[derive(Template)]
#[template(path = "login.html")]
pub struct LoginTemplate {
    pub error: Option<String>,
}

/// Pre-formatted for display (locale-aware via `adm_sfa_core::format`,
/// computed in the route handler, not the template) — templates just
/// interpolate strings, they don't call into formatting logic themselves.
pub struct PurchaseRow {
    pub id: i64,
    pub date_display: String,
    pub channel: String,
    pub cost_display: String,
    pub currency_symbol: &'static str,
    pub status_label: &'static str,
    pub multiple_items: bool,
}

#[derive(Template)]
#[template(path = "purchases/list.html")]
pub struct PurchasesListTemplate {
    pub purchases: Vec<PurchaseRow>,
}

#[derive(Template)]
#[template(path = "purchases/form.html")]
pub struct PurchaseFormTemplate {
    pub id: Option<i64>,
    pub draft: adm_sfa_core::model::purchase::PurchaseDraft,
    pub error: Option<String>,
    pub documents: Vec<adm_sfa_core::model::document::Document>,
    pub is_negotiating: bool,
}

pub struct DonorRow {
    pub id: i64,
    pub name: String,
    pub contact_info: String,
}

#[derive(Template)]
#[template(path = "donors/list.html")]
pub struct DonorsListTemplate {
    pub donors: Vec<DonorRow>,
}

#[derive(Template)]
#[template(path = "donors/form.html")]
pub struct DonorFormTemplate {
    pub id: Option<i64>,
    pub draft: adm_sfa_core::model::donor::DonorDraft,
    pub error: Option<String>,
}

pub struct EurLedgerRow {
    pub id: i64,
    pub date_display: String,
    pub type_label: String,
    pub sign: &'static str,
    pub amount_display: String,
    pub desc: String,
    /// Only manual entries (Donation/Self-funding) link to an edit form —
    /// purchase- and transfer-linked entries are auto-created and read-only
    /// here, same as desktop.
    pub editable: bool,
}

#[derive(Template)]
#[template(path = "eur_ledger/list.html")]
pub struct EurLedgerListTemplate {
    pub rows: Vec<EurLedgerRow>,
    pub balance_display: String,
    pub balance_positive: bool,
}

#[derive(Template)]
#[template(path = "eur_ledger/form.html")]
pub struct EurTxFormTemplate {
    pub id: Option<i64>,
    pub date: String,
    /// `None` when adding (radios shown, both choices possible); `Some(label)`
    /// when editing an existing manual entry (type is fixed after creation).
    pub type_label: Option<String>,
    /// Whether the donor field should be shown: always true when adding
    /// (the chosen radio isn't known until submit, so both fields render and
    /// the server ignores donor_id if self-funding was submitted); only true
    /// when editing a donation entry.
    pub show_donor: bool,
    pub amount_str: String,
    /// (donor id, donor name, whether this donor is the currently selected one).
    pub donors: Vec<(i64, String, bool)>,
    pub note: String,
    pub error: Option<String>,
}

pub struct BrlLedgerRow {
    pub date_display: String,
    pub type_label: String,
    pub sign: &'static str,
    pub amount_display: String,
    pub desc: String,
}

#[derive(Template)]
#[template(path = "brl_ledger/list.html")]
pub struct BrlLedgerListTemplate {
    pub rows: Vec<BrlLedgerRow>,
    pub balance_display: String,
    pub balance_positive: bool,
}

pub struct TransferRow {
    pub id: i64,
    pub date_display: String,
    pub eur_display: String,
    pub brl_display: String,
    pub rate_display: String,
}

#[derive(Template)]
#[template(path = "transfers/list.html")]
pub struct TransfersListTemplate {
    pub transfers: Vec<TransferRow>,
}

#[derive(Template)]
#[template(path = "transfers/form.html")]
pub struct TransferFormTemplate {
    pub id: Option<i64>,
    pub date: String,
    pub eur_amount_sent_str: String,
    pub exchange_rate_str: String,
    pub notes: String,
    /// Live EUR*rate preview, shown when both fields currently parse —
    /// mirrors desktop's own non-authoritative preview label
    /// (`ui/views/transfers.rs`); the authoritative BRL amount is computed
    /// in `core::db::queries::transfers` at save time.
    pub brl_preview: Option<String>,
    pub error: Option<String>,
    pub documents: Vec<adm_sfa_core::model::document::Document>,
}
