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

/// Resolves the data root from, in priority order: a `--data-dir <path>`
/// CLI argument, the `ADM_SFA_DATA_DIR` environment variable, or
/// [`default_data_dir`]. Shared by both front-ends so `--data-dir`/
/// `ADM_SFA_DATA_DIR` behave identically regardless of which binary is
/// running — this used to be duplicated per binary, and the two copies had
/// quietly drifted: `desktop`'s never checked `ADM_SFA_DATA_DIR` at all,
/// even though this crate's own docs already described the env var as
/// following "the same convention as desktop."
///
/// Takes `args` as an iterator rather than reading `std::env::args()`
/// itself, so a test can pass a synthetic `Vec<String>` — like
/// `std::env::args()`, the first element (the program name) is skipped.
pub fn parse_data_dir(args: impl Iterator<Item = String>) -> PathBuf {
    let args: Vec<String> = args.collect();
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
    default_data_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_data_dir_prefers_the_cli_flag_over_everything() {
        // Doesn't touch ADM_SFA_DATA_DIR — the CLI flag must win regardless
        // of whatever the env var happens to hold, so this is safe to run
        // in parallel with the env-var test below.
        let args = vec![
            "adm-sfa".to_string(),
            "--data-dir".to_string(),
            "/tmp/from-cli".to_string(),
        ];
        assert_eq!(
            parse_data_dir(args.into_iter()),
            PathBuf::from("/tmp/from-cli")
        );
    }

    #[test]
    fn parse_data_dir_falls_back_to_env_var_then_default() {
        // std::env::var is process-global, so both scenarios are exercised
        // sequentially in one test rather than split across two — nothing
        // else in this crate reads or writes ADM_SFA_DATA_DIR, so this is
        // the only place mutating it, and cargo test's parallelism across
        // *different* test functions can't interleave with it.
        let no_args = vec!["adm-sfa".to_string()];

        std::env::remove_var("ADM_SFA_DATA_DIR");
        assert_eq!(
            parse_data_dir(no_args.clone().into_iter()),
            default_data_dir()
        );

        std::env::set_var("ADM_SFA_DATA_DIR", "/tmp/from-env");
        assert_eq!(
            parse_data_dir(no_args.into_iter()),
            PathBuf::from("/tmp/from-env")
        );

        std::env::remove_var("ADM_SFA_DATA_DIR");
    }
}
