use std::path::{Path, PathBuf};

/// Default data root used when no explicit override is given: the OS's
/// local-data directory joined with the app name, e.g.
/// `~/.local/share/adm-sfa` on Linux. Both front-ends resolve their own
/// CLI/env override first and fall back to this, so they agree on the same
/// default when neither is given one explicitly.
pub fn default_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("adm-sfa")
}

/// Creates the data directory tree (including `documents/_deleted`) if it
/// doesn't already exist. Both front-ends must call this before opening the
/// DB or filing any document.
pub fn ensure_dirs(data_dir: &Path) {
    std::fs::create_dir_all(data_dir.join("documents/_deleted"))
        .expect("failed to create data directories");
}
