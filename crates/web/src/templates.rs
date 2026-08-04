use askama::Template;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

/// Askama custom filters, reachable from every template in this module as
/// `{{ "some.key"|t(locale) }}` — Askama resolves a custom filter `foo` to
/// a plain function `filters::foo` in the same module as the `#[derive(
/// Template)]` struct, so this one `mod` covers all of them.
pub(crate) mod filters {
    /// Thin wrapper over `rust_i18n::t!` with an *explicit* locale — never
    /// touches the ambient global locale. See `main.rs`'s doc comment on
    /// the `rust_i18n::i18n!` invocation for why `web` can't use the
    /// simpler ambient-locale form desktop uses. Static strings only (no
    /// `%{}` interpolation) — anything needing interpolated arguments is
    /// formatted in the route handler instead, same as this crate already
    /// does for money/date formatting (see `PurchaseRow`'s doc comment
    /// below).
    pub fn t(key: &str, locale: &str) -> askama::Result<String> {
        Ok(rust_i18n::t!(key, locale = locale).to_string())
    }

    /// Same as `t`, but additionally JS-string-escapes the result — for the
    /// handful of translated strings interpolated into a single-quoted JS
    /// string literal inside an inline `onsubmit="return confirm('...')"`
    /// (Askama's normal auto-escaping only protects the surrounding HTML
    /// *attribute*, not the JS string nested inside it: a literal `'` in
    /// the translated text would close the JS string early, at best
    /// throwing a syntax error that silently breaks the confirm dialog).
    /// Escaping backslash before quote matters — otherwise a backslash this
    /// function inserts for an escaped quote would itself need escaping.
    pub fn tjs(key: &str, locale: &str) -> askama::Result<String> {
        let translated = rust_i18n::t!(key, locale = locale);
        Ok(translated
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace(['\n', '\r'], "\\n"))
    }
}

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
    pub locale: String,
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
    pub status_label: String,
    pub multiple_items: bool,
}

#[derive(Template)]
#[template(path = "purchases/list.html")]
pub struct PurchasesListTemplate {
    pub purchases: Vec<PurchaseRow>,
    pub locale: String,
}

/// One line of the per-file status list shown after a create-with-
/// documents submission where the purchase saved but not every staged
/// document attached — see `PurchaseFormTemplate::attach_results`.
/// `message` is pre-formatted (interpolated `%{name}`/`%{label}`/
/// `%{error}`) in the route handler, same reasoning as `PurchaseRow`'s doc
/// comment: the Askama `t` filter is static-string-only.
pub struct AttachResult {
    pub ok: bool,
    pub message: String,
}

#[derive(Template)]
#[template(path = "purchases/form.html")]
pub struct PurchaseFormTemplate {
    pub id: Option<i64>,
    pub draft: adm_sfa_core::model::purchase::PurchaseDraft,
    pub error: Option<String>,
    pub documents: Vec<adm_sfa_core::model::document::Document>,
    pub is_negotiating: bool,
    /// Document labels for the document picker(s) — sourced from
    /// `document_label`, same allow-list `core::service::attach_document`
    /// enforces authoritatively, so the web form can't submit a value that
    /// gets rejected. Populated on both the create form (the "Documents?"
    /// section's repeatable label pickers) and the edit form (the
    /// "Attach a document" mini-form).
    pub labels: Vec<String>,
    /// Per-document outcome of a create-with-documents submission where the
    /// purchase saved but at least one staged document failed to attach.
    /// Empty on every other render path (a fresh new-purchase form, a plain
    /// edit-page load, a fully-successful create that redirected away).
    pub attach_results: Vec<AttachResult>,
    pub locale: String,
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
    pub locale: String,
}

#[derive(Template)]
#[template(path = "donors/form.html")]
pub struct DonorFormTemplate {
    pub id: Option<i64>,
    pub draft: adm_sfa_core::model::donor::DonorDraft,
    pub error: Option<String>,
    /// Round-tripped from `?return_to=` on `GET /donors/new` through a
    /// hidden form field, so `create()` can redirect back to the page that
    /// linked here (e.g. `/eur-ledger/new`) instead of always landing on
    /// this donor's own edit page. `None` on the edit form and on the
    /// normal Donors-page "+ New donor" flow (no caller to return to).
    pub return_to: Option<String>,
    pub locale: String,
}

pub struct EurLedgerRow {
    pub date_display: String,
    pub type_label: String,
    pub sign: &'static str,
    pub amount_display: String,
    pub desc: String,
    /// Where the description cell links to, pre-formatted in the route
    /// handler: a manual entry (Donation/Self-funding) links to its own
    /// `/eur-ledger/{id}/edit` — it's not directly editable here since
    /// they're auto-created, but the underlying record still is, so this
    /// links to *that* record's own edit page instead of nothing. `None`
    /// only if a purchase-/transfer-linked row's `linked_purchase_id`/
    /// `linked_transfer_id` is somehow unset — a data-integrity gap the
    /// `NOT NULL` FK should prevent, not the common case.
    pub link_href: Option<String>,
}

#[derive(Template)]
#[template(path = "eur_ledger/list.html")]
pub struct EurLedgerListTemplate {
    pub rows: Vec<EurLedgerRow>,
    pub balance_positive: bool,
    /// Full "Balance: € 123.45" sentence, pre-formatted in the route
    /// handler via `t!("common.balance", locale = .., symbol = .., amount
    /// = ..)` — word order around an interpolated amount can differ per
    /// language, so this isn't built by concatenating static template text
    /// with `balance_display`.
    pub balance_label: String,
    pub locale: String,
}

#[derive(Template)]
#[template(path = "eur_ledger/form.html")]
pub struct EurTxFormTemplate {
    pub id: Option<i64>,
    pub date: String,
    /// `None` when adding (radios shown, both choices possible); `Some(label)`
    /// when editing an existing manual entry (type is fixed after creation).
    pub type_label: Option<String>,
    /// Whether the donor field should be visible on render. When adding:
    /// `false` until a type is chosen, then reflects whichever type was
    /// actually chosen/submitted (`donation_checked`) — also toggled live by
    /// the template's own JS as the user clicks a radio. When editing:
    /// fixed based on the existing entry's immutable type, never toggled.
    pub show_donor: bool,
    /// Create-form only: whether the "Donation" / "Self-funding" radio
    /// should render `checked`. Both `false` on a fresh `/eur-ledger/new`
    /// load (no default) and on the type-validation-error path; reflect the
    /// actually-parsed type after a date/amount validation error. Unused
    /// (left `false`) when `type_label` is `Some` — that branch doesn't
    /// render radios at all.
    pub donation_checked: bool,
    pub self_funding_checked: bool,
    pub amount_str: String,
    /// (donor id, donor name, whether this donor is the currently selected one).
    pub donors: Vec<(i64, String, bool)>,
    pub note: String,
    pub error: Option<String>,
    pub locale: String,
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
    pub balance_positive: bool,
    /// See `EurLedgerListTemplate::balance_label`.
    pub balance_label: String,
    pub locale: String,
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
    pub locale: String,
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
    /// See `PurchaseFormTemplate::attach_results`.
    pub attach_results: Vec<AttachResult>,
    pub locale: String,
}

pub struct InventoryRow {
    pub id: i64,
    pub name: String,
    pub category_name: String,
    pub location_label: String,
    pub status_label: String,
    pub source_desc: String,
}

#[derive(Template)]
#[template(path = "inventory/list.html")]
pub struct InventoryListTemplate {
    pub items: Vec<InventoryRow>,
    pub locale: String,
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
    /// See `PurchaseFormTemplate::attach_results`.
    pub attach_results: Vec<AttachResult>,
    pub locale: String,
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
    /// Round-tripped from `?return_to=` on `GET /inventory/donations`
    /// through a hidden form field, so `create_donation()` can redirect
    /// back to the page that linked here (`/inventory/new`) instead of
    /// always landing on this donations list. `None` when this page was
    /// reached any other way (e.g. its own nav entry). Mirrors
    /// `DonorFormTemplate::return_to`.
    pub return_to: Option<String>,
    pub locale: String,
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
    pub locale: String,
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
    pub locale: String,
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
    pub return_to: Option<String>,
    pub locale: String,
}

pub struct TabLink {
    pub label: String,
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
    pub locale: String,
}

#[derive(Template)]
#[template(path = "settings/index.html")]
pub struct SettingsTemplate {
    pub categories: Vec<(i64, String)>,
    pub labels: Vec<(i64, String)>,
    pub error: Option<String>,
    pub locale: String,
}
