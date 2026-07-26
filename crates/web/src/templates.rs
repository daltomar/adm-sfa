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
