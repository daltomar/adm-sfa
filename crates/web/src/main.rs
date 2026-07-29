mod auth;
mod i18n;
mod routes;
mod state;
mod templates;
#[cfg(test)]
mod test_support;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// Compile-time locale catalogue (mirrors crates/desktop/src/main.rs and
// crates/core/src/lib.rs — each crate embeds its own copy of the same
// locales/*.yml content; see crates/core/src/lib.rs's doc comment on why
// that's per-crate rather than inherited). Unlike desktop, `web` must
// never call `rust_i18n::set_locale`: its runtime locale state
// (`rust_i18n::locale()`/`set_locale()`) is a single process-wide
// `LazyLock<AtomicStr>` (see the `rust-i18n` crate's source), not
// request-scoped, so one axum request mutating it while another renders
// concurrently would be a real race — two users on the LAN could get
// text baked in the other's locale. Every `t!()` call in this crate must
// carry an explicit `locale = ...` argument instead; `crate::i18n`
// resolves what that locale is per request.
rust_i18n::i18n!("../../locales", fallback = "en");

use axum::extract::DefaultBodyLimit;
use axum::response::Redirect;
use axum::routing::get;
use axum::Router;
use axum_extra::extract::cookie::Key;
use tower_http::services::ServeDir;

use state::AppState;

const MAX_UPLOAD_BYTES: usize = 25 * 1024 * 1024; // 25 MiB — photos/screenshots, not video.

#[tokio::main]
async fn main() {
    let data_dir = parse_data_dir();
    adm_sfa_core::config::ensure_dirs(&data_dir);
    let documents_dir = data_dir.join("documents");

    let db = adm_sfa_core::db::open_db(&data_dir).expect("failed to open database");

    let password = std::env::var("ADM_SFA_WEB_PASSWORD").unwrap_or_else(|_| {
        eprintln!(
            "ADM_SFA_WEB_PASSWORD is not set. Set it to the shared password before starting \
             the web server — there is no default."
        );
        std::process::exit(1);
    });

    let state = AppState {
        db: Arc::new(Mutex::new(db)),
        documents_dir: documents_dir.clone(),
        password,
        cookie_key: Key::generate(),
    };

    let app = build_app(state);

    let bind_addr = std::env::var("ADM_SFA_WEB_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {bind_addr}: {e}"));
    println!("adm-sfa-web listening on http://{bind_addr}");

    axum::serve(listener, app).await.expect("server error");
}

/// Assembles the full router — pulled out of `main` so tests can build the
/// same app (auth middleware, upload size limit, and all) without going
/// through `main`'s env-var/DB-opening/TCP-binding setup.
fn build_app(state: AppState) -> Router {
    let protected = Router::new()
        .route("/", get(|| async { Redirect::to("/purchases") }))
        .merge(routes::purchases::router())
        .merge(routes::donors::router())
        .merge(routes::eur_ledger::router())
        .merge(routes::brl_ledger::router())
        .merge(routes::transfers::router())
        .merge(routes::inventory::router())
        .merge(routes::outbound::router())
        .merge(routes::reports::router())
        .merge(routes::settings::router())
        .nest_service("/documents", ServeDir::new(&state.documents_dir))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ));

    Router::new()
        .merge(routes::login::router())
        .merge(protected)
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES))
        .with_state(state)
}

fn parse_data_dir() -> PathBuf {
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--data-dir" && i + 1 < args.len() {
            return PathBuf::from(&args[i + 1]);
        }
        i += 1;
    }
    if let Ok(dir) = std::env::var("ADM_SFA_DATA_DIR") {
        return PathBuf::from(dir);
    }
    adm_sfa_core::config::default_data_dir()
}
