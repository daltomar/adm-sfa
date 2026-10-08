use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use axum_extra::extract::SignedCookieJar;

use adm_sfa_core::dashboard::DashboardData;
use adm_sfa_core::format;

use crate::flash;
use crate::state::AppState;
use crate::templates::{DashboardTemplate, HtmlTemplate};

pub fn router() -> Router<AppState> {
    Router::new().route("/dashboard", get(index))
}

async fn index(State(state): State<AppState>, jar: SignedCookieJar) -> impl IntoResponse {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let data = adm_sfa_core::dashboard::load(&conn).unwrap_or_else(|e| {
        eprintln!("dashboard: failed to load data: {e}");
        DashboardData::default()
    });
    drop(conn);

    let negotiations_label = if data.negotiating_count == 0 {
        rust_i18n::t!("dashboard.negotiations.none", locale = &locale).to_string()
    } else if data.negotiating_count == 1 {
        rust_i18n::t!("dashboard.negotiations.count.one", locale = &locale).to_string()
    } else {
        rust_i18n::t!(
            "dashboard.negotiations.count.other",
            locale = &locale,
            count = data.negotiating_count
        )
        .to_string()
    };

    let (jar, kind) = flash::take_flash(jar);
    let flash = flash::flash_for_template(kind, &locale);

    (
        jar,
        HtmlTemplate(DashboardTemplate {
            eur_balance: format::amount_in(data.eur_balance, &locale),
            brl_balance: format::amount_in(data.brl_balance, &locale),
            inventory_available: data.inventory_available,
            inventory_reserved: data.inventory_reserved,
            inventory_donated: data.inventory_donated,
            outbound_items_this_year: data.outbound_items_this_year,
            outbound_cash_this_year: format::amount_in(data.outbound_cash_this_year, &locale),
            negotiations_label,
            flash,
            locale,
        }),
    )
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};

    use crate::test_support;

    #[tokio::test]
    async fn index_renders_for_an_authenticated_user() {
        let (state, dir) = test_support::test_app("dashboard-index-auth");
        let app = crate::build_app(state);
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("GET")
            .uri("/dashboard")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        assert!(
            body.contains("EUR Balance")
                || body.contains("EUR-Kontostand")
                || body.contains("Saldo EUR"),
            "expected a balance heading in the body: {body}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn index_without_a_session_redirects_to_login() {
        let (state, dir) = test_support::test_app("dashboard-index-unauth");
        let app = crate::build_app(state);

        let req = Request::builder()
            .method("GET")
            .uri("/dashboard")
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            res.headers().get("location").unwrap().to_str().unwrap(),
            "/login"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
