//! Per-request locale resolution for `web`'s UI chrome.
//!
//! `web` must never call `rust_i18n::set_locale` — see the doc comment on
//! the `rust_i18n::i18n!` invocation in `main.rs` for why (it's a single
//! process-wide `LazyLock`, not request-scoped, so mutating it from one
//! axum request while another renders concurrently would be a real race).
//! Every `t!()` call in this crate must instead receive its locale
//! explicitly, and this module is the one place that decides what that
//! locale is.

use rusqlite::Connection;

/// The installation's UI locale, shared with `desktop` via the `ui_locale`
/// `app_setting` row (same DB, same key `desktop`'s Settings panel writes
/// to). Read fresh on every request — never cached, never pushed into
/// `rust_i18n::set_locale` — so a language change made in desktop takes
/// effect on `web`'s very next request with no restart needed. Falls back
/// to `"en"` if unset, matching `rust_i18n::i18n!`'s own fallback.
pub fn resolve_locale(conn: &Connection) -> String {
    adm_sfa_core::db::queries::settings::get(conn, "ui_locale")
        .ok()
        .flatten()
        .unwrap_or_else(|| "en".to_string())
}
