// Model enums' `label()` methods (SPEC.md §6, CLAUDE.md "domain vocabulary")
// call `rust_i18n::t!()` directly, so this crate needs its own catalogue
// embedding — `t!()` is generated per-crate by this macro, not inherited
// from a dependent crate's own invocation. `desktop` (and later `web`)
// invoke it too, each embedding an independent copy of the same
// `locales/*.yml` content; the active-locale runtime state
// (`rust_i18n::locale()`/`set_locale()`) is shared globally regardless of
// which crate's `i18n!` produced it, so this stays in sync across crates.
rust_i18n::i18n!("../../locales", fallback = "en");

pub mod backup;
pub mod config;
pub mod date;
pub mod db;
pub mod docs_fs;
pub mod format;
pub mod model;
pub mod money;
pub mod reporting;
pub mod service;
