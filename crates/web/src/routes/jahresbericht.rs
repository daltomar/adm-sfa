use axum::extract::{Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::{Form, Router};
use axum_extra::extract::SignedCookieJar;
use serde::Deserialize;
use std::sync::atomic::{AtomicU64, Ordering};

use adm_sfa_core::db::queries::annual_report_draft as draft_qry;
use adm_sfa_core::format;
use adm_sfa_core::statutory::{self, AnnualReportDraft};

use crate::flash::{self, FlashKind};
use crate::state::AppState;
use crate::templates::{HtmlTemplate, JahresberichtEventRow, JahresberichtTemplate};

static EXPORT_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/jahresbericht", get(index).post(save_draft))
        .route("/jahresbericht/export.pdf", get(export_pdf))
}

#[derive(Deserialize)]
struct YearQuery {
    #[serde(default)]
    year: Option<i32>,
}

async fn index(
    State(state): State<AppState>,
    Query(q): Query<YearQuery>,
    jar: SignedCookieJar,
) -> impl IntoResponse {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let year = q.year.unwrap_or_else(default_year);
    let (jar, kind) = flash::take_flash(jar);
    let flash = flash::flash_for_template(kind, &locale);
    let tmpl = build_template(&conn, year, &locale, None, flash);
    (jar, HtmlTemplate(tmpl))
}

#[derive(Deserialize)]
struct DraftForm {
    year: i32,
    member_count: String,
    meeting_date: String,
    meeting_time_from: String,
    meeting_time_to: String,
    tb_activities: String,
    tb_continuous: String,
    tb_outlook: String,
    pk_activities_next_year: String,
    pk_decisions: String,
}

async fn save_draft(
    State(state): State<AppState>,
    jar: SignedCookieJar,
    Form(form): Form<DraftForm>,
) -> Response {
    let conn = state.conn();
    let draft = AnnualReportDraft {
        year: form.year,
        member_count: form.member_count.trim().parse().unwrap_or(0),
        meeting_date: form.meeting_date.trim().to_string(),
        meeting_time_from: form.meeting_time_from.trim().to_string(),
        meeting_time_to: form.meeting_time_to.trim().to_string(),
        tb_activities: form.tb_activities.clone(),
        tb_continuous: form.tb_continuous.clone(),
        tb_outlook: form.tb_outlook.clone(),
        pk_activities_next_year: form.pk_activities_next_year.clone(),
        pk_decisions: form.pk_decisions.clone(),
    };
    match draft_qry::upsert(&conn, &draft) {
        Ok(()) => {
            let jar = flash::set_flash(jar, FlashKind::Success);
            (
                jar,
                Redirect::to(&format!("/jahresbericht?year={}", form.year)),
            )
                .into_response()
        }
        Err(e) => {
            let locale = crate::i18n::resolve_locale(&conn);
            let tmpl = build_template(&conn, form.year, &locale, Some(e.to_string()), None);
            HtmlTemplate(tmpl).into_response()
        }
    }
}

#[derive(Deserialize)]
struct ExportQuery {
    year: i32,
    #[serde(default = "default_report")]
    report: String,
}

fn default_report() -> String {
    "all".to_string()
}

async fn export_pdf(State(state): State<AppState>, Query(q): Query<ExportQuery>) -> Response {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);

    let data = match statutory::load(&conn, q.year) {
        Ok(d) => d,
        Err(e) => {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to load data: {e}"),
            )
                .into_response();
        }
    };
    drop(conn);

    let input = crate::routes::jahresbericht_input::build_input(&data, &locale);

    let tmp_path = std::env::temp_dir().join(format!(
        "adm-sfa-jahresbericht-{}-{}-{}.pdf",
        q.year,
        q.report,
        EXPORT_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    let result = match q.report.as_str() {
        "ea" => reports::statutory::export_einnahmen_ausgaben(&input, &tmp_path),
        "vermoegen" => reports::statutory::export_vermoegen(&input, &tmp_path),
        "tb" => reports::statutory::export_taetigkeitsbericht(&input, &tmp_path),
        "protokoll" => reports::statutory::export_protokoll(&input, &tmp_path),
        _ => reports::statutory::export_all(&input, &tmp_path),
    };

    if let Err(e) = result {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("PDF generation failed: {e}"),
        )
            .into_response();
    }

    let filename = format!("jahresbericht-{}-{}.pdf", q.year, q.report);
    let bytes = match std::fs::read(&tmp_path) {
        Ok(b) => b,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_path);
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read generated PDF: {e}"),
            )
                .into_response();
        }
    };
    let _ = std::fs::remove_file(&tmp_path);

    (
        [
            ("content-type", "application/pdf"),
            (
                "content-disposition",
                &format!("attachment; filename=\"{filename}\""),
            ),
        ],
        bytes,
    )
        .into_response()
}

// ── Template builder ─────────────────────────────────────────────────────────

fn build_template(
    conn: &rusqlite::Connection,
    year: i32,
    locale: &str,
    error: Option<String>,
    flash: Option<crate::templates::Flash>,
) -> JahresberichtTemplate {
    let data = statutory::load(conn, year).ok();

    let (org_name, org_address, org_tax_number) = data
        .as_ref()
        .map(|d| {
            (
                d.org_name.clone(),
                d.org_address.clone(),
                d.org_tax_number.clone(),
            )
        })
        .unwrap_or_default();

    let (income_total, expense_total, surplus, bank_balance, inventory_value) = data
        .as_ref()
        .map(|d| {
            let ea = &d.einnahmen_ausgaben;
            let v = &d.vermoegen;
            (
                format::amount_in(ea.income_total, locale),
                format::amount_in(ea.expense_total, locale),
                format::amount_in(ea.surplus, locale),
                format::amount_in(v.bank_balance, locale),
                format::amount_in(v.inventory_value, locale),
            )
        })
        .unwrap_or_default();

    let events: Vec<JahresberichtEventRow> = data
        .as_ref()
        .map(|d| {
            d.outbound_events
                .iter()
                .map(|e| JahresberichtEventRow {
                    date: format::date_in(&e.date, locale),
                    recipient: e.recipient_name.clone(),
                    item_count: e.item_count.to_string(),
                })
                .collect()
        })
        .unwrap_or_default();

    let draft = data
        .as_ref()
        .map(|d| &d.draft)
        .map(|dr| crate::templates::JahresberichtDraft {
            member_count: dr.member_count.to_string(),
            meeting_date: dr.meeting_date.clone(),
            meeting_time_from: dr.meeting_time_from.clone(),
            meeting_time_to: dr.meeting_time_to.clone(),
            tb_activities: dr.tb_activities.clone(),
            tb_continuous: dr.tb_continuous.clone(),
            tb_outlook: dr.tb_outlook.clone(),
            pk_activities_next_year: dr.pk_activities_next_year.clone(),
            pk_decisions: dr.pk_decisions.clone(),
        })
        .unwrap_or_default();

    JahresberichtTemplate {
        year,
        org_name,
        org_address,
        org_tax_number,
        income_total,
        expense_total,
        surplus,
        bank_balance,
        inventory_value,
        events,
        draft,
        error,
        flash,
        locale: locale.to_string(),
    }
}

fn default_year() -> i32 {
    chrono::Local::now()
        .format("%Y")
        .to_string()
        .parse()
        .unwrap_or(2025)
        - 1
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};

    use crate::test_support;

    #[tokio::test]
    async fn index_renders_for_authenticated_user() {
        let (state, dir) = test_support::test_app("jahresbericht-index");
        let app = crate::build_app(state);
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("GET")
            .uri("/jahresbericht?year=2025")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        assert!(body.contains("2025"), "expected year in body: {body}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn index_without_session_redirects_to_login() {
        let (state, dir) = test_support::test_app("jahresbericht-unauth");
        let app = crate::build_app(state);

        let req = Request::builder()
            .method("GET")
            .uri("/jahresbericht")
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get("location").unwrap(), "/login");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn save_draft_redirects_and_persists() {
        let (state, dir) = test_support::test_app("jahresbericht-save");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body = "year=2025&member_count=2&meeting_date=14.02.2026\
            &meeting_time_from=16%3A00&meeting_time_to=19%3A00\
            &tb_activities=Test+activities&tb_continuous=Ongoing\
            &tb_outlook=Future+plans&pk_activities_next_year=Next+year\
            &pk_decisions=None";
        let req = Request::builder()
            .method("POST")
            .uri("/jahresbericht")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let location = res.headers().get("location").unwrap().to_str().unwrap();
        assert!(
            location.contains("2025"),
            "expected redirect to year page: {location}"
        );

        // Verify it was persisted
        let conn = state.conn();
        let draft = adm_sfa_core::db::queries::annual_report_draft::get(&conn, 2025)
            .unwrap()
            .unwrap();
        assert_eq!(draft.member_count, 2);
        assert_eq!(draft.tb_activities, "Test activities");
        std::fs::remove_dir_all(&dir).ok();
    }
}
