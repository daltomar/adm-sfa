use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use axum_extra::extract::cookie::Key;
use rusqlite::Connection;

/// Shared server state. `db` is a single connection behind a plain
/// `std::sync::Mutex`, not a pool — matching CLAUDE.md's "two users, one
/// machine, no sync" design: this app expects rare overlapping use, not
/// real concurrent load, so a connection pool or async-aware lock would be
/// solving a problem this app doesn't have. WAL mode (set in
/// `core::db::open_db`) is what actually lets `desktop` and `web` share
/// the same SQLite file safely; this mutex only serializes `web`'s own
/// handlers against each other.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
    pub documents_dir: PathBuf,
    /// The single shared password (SPEC.md / CLAUDE.md: no per-user
    /// accounts). Read once from the `ADM_SFA_WEB_PASSWORD` environment
    /// variable at startup — see `main.rs`.
    pub password: String,
    /// Signs the session cookie so it can't be forged or edited client-side.
    /// Generated fresh per process start (not persisted): a restart simply
    /// requires logging in again, which is an acceptable tradeoff for a
    /// single-shared-login app and avoids needing a place to store a
    /// long-lived secret.
    pub cookie_key: Key,
}

impl AppState {
    /// Locks the shared connection, recovering from poisoning instead of
    /// panicking. A panic inside one handler while holding the lock would
    /// otherwise poison the mutex permanently, taking down every later
    /// request for the rest of the process's life; `Connection` has no
    /// invariant a panicking holder could have left broken, so recovering
    /// the guard is safe.
    pub fn conn(&self) -> MutexGuard<'_, Connection> {
        self.db
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl axum::extract::FromRef<AppState> for Key {
    fn from_ref(state: &AppState) -> Self {
        state.cookie_key.clone()
    }
}
