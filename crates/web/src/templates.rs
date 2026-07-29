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
    /// Document labels for the "Attach a document" picker — sourced from
    /// `document_label`, same allow-list `core::service::attach_document`
    /// enforces authoritatively, so the web form can't submit a value that
    /// gets rejected. Empty (and unused, since the template only renders
    /// the picker when `id` is `Some`) on the create form.
    pub labels: Vec<String>,
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
    /// See `PurchaseFormTemplate::labels`.
    pub labels: Vec<String>,
}

pub struct InventoryRow {
    pub id: i64,
    pub name: String,
    pub category_name: String,
    pub location_label: &'static str,
    pub status_label: &'static str,
    pub source_desc: String,
}

#[derive(Template)]
#[template(path = "inventory/list.html")]
pub struct InventoryListTemplate {
    pub items: Vec<InventoryRow>,
}

pub struct CategoryOption {
    pub id: i64,
    pub name: String,
    pub selected: bool,
}

pub struct DonationOption {
    pub id: i64,
    pub label: String,
    pub selected: bool,
}

pub struct PurchaseOption {
    pub id: i64,
    pub label: String,
    pub selected: bool,
    /// A single-item purchase already linked to a *different* inventory
    /// item — mirrors desktop's `purchase_source_blocked` grey-out. Shown
    /// as a disabled `<option>` rather than omitted, so it's clear the
    /// purchase exists but can't be picked, same as desktop.
    pub blocked: bool,
}

#[derive(Template)]
#[template(path = "inventory/form.html")]
pub struct InventoryFormTemplate {
    pub id: Option<i64>,
    pub name: String,
    /// "germany" or "brazil" — compared directly in the template rather
    /// than via a bool-per-option, since Askama handles a plain string
    /// `==` in an expression fine (see `purchases/form.html`'s
    /// `draft.currency.as_str() == "EUR"`).
    pub location: String,
    pub status: String,
    /// "donation" or "purchase".
    pub source_type: String,
    /// Raw scalar counterparts of `categories`/`donations`/`purchases`'
    /// per-option `selected` flags — needed as hidden-input values when
    /// `locked` is true, since a `<select>`'s own selection doesn't survive
    /// being `disabled` (disabled form controls aren't submitted at all).
    pub category_id: Option<i64>,
    pub source_donation_id: Option<i64>,
    pub source_purchase_id: Option<i64>,
    pub categories: Vec<CategoryOption>,
    pub donations: Vec<DonationOption>,
    pub purchases: Vec<PurchaseOption>,
    pub notes: String,
    pub error: Option<String>,
    pub documents: Vec<adm_sfa_core::model::document::Document>,
    /// See `PurchaseFormTemplate::labels`.
    pub labels: Vec<String>,
    /// True once the item has been donated — every field except `notes` is
    /// locked (CLAUDE.md's donated-item field-locking backlog item).
    /// `core::db::queries::inventory::update` enforces this authoritatively;
    /// the template only needs to match it for the UI indication.
    pub locked: bool,
}

pub struct DonationRow {
    pub date_display: String,
    pub donor_display: String,
}

#[derive(Template)]
#[template(path = "inventory/donations.html")]
pub struct DonationsTemplate {
    pub donations: Vec<DonationRow>,
    pub date: String,
    pub donors: Vec<(i64, String)>,
    pub error: Option<String>,
}

pub struct OutboundRow {
    pub id: i64,
    pub date_display: String,
    pub recipient_name: String,
    pub summary: String,
}

#[derive(Template)]
#[template(path = "outbound/list.html")]
pub struct OutboundListTemplate {
    pub events: Vec<OutboundRow>,
}

pub struct RecipientOption {
    pub id: i64,
    pub name: String,
    pub selected: bool,
}

pub struct ItemOption {
    pub id: i64,
    pub label: String,
    pub selected: bool,
}

#[derive(Template)]
#[template(path = "outbound/form.html")]
pub struct OutboundFormTemplate {
    pub id: Option<i64>,
    pub date: String,
    pub recipients: Vec<RecipientOption>,
    pub cash_amount_brl_str: String,
    pub notes: String,
    pub items: Vec<ItemOption>,
    pub error: Option<String>,
}

pub struct RecipientRow {
    pub name: String,
    pub contact_info: String,
    pub location: String,
    pub active: bool,
}

#[derive(Template)]
#[template(path = "outbound/recipients.html")]
pub struct RecipientsTemplate {
    pub recipients: Vec<RecipientRow>,
    pub error: Option<String>,
}

pub struct TabLink {
    pub label: &'static str,
    pub href: String,
    pub active: bool,
}

/// A label/value pair for the small stats panel shown above the EUR/BRL
/// tabs' detail table — e.g. ("Donations (3)", "€ 450.00"). `None` for the
/// other four tabs (Donors, Inventory, Outbound, Audit Trail), which are
/// just a table with no separate aggregate panel.
pub struct SummaryLine {
    pub label: String,
    pub value: String,
}

#[derive(Template)]
#[template(path = "reports/index.html")]
pub struct ReportsTemplate {
    pub tabs: Vec<TabLink>,
    pub active_tab: &'static str,
    pub date_from: String,
    pub date_to: String,
    pub recipient_id: Option<i64>,
    /// (recipient id, name, whether this is the currently selected filter).
    pub recipients: Vec<(i64, String, bool)>,
    pub summary: Vec<SummaryLine>,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "settings/index.html")]
pub struct SettingsTemplate {
    pub categories: Vec<(i64, String)>,
    pub labels: Vec<(i64, String)>,
    pub error: Option<String>,
}
