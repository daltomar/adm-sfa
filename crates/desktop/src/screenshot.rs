use std::path::PathBuf;
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug)]
pub enum CaptureOutcome {
    /// Capture succeeded; the PNG is at this temp path, not yet filed.
    Success(PathBuf),
    /// The OS tool ran but produced nothing — most region-select tools
    /// (maim, grim+slurp, screencapture -i) signal an Escape/cancel this
    /// way, via a non-zero exit rather than a distinct "cancelled" code.
    /// Not an error: no document, no orphan record, nothing to show but a
    /// neutral status.
    Cancelled,
}

/// Runs `command_template` (with `{path}` substituted for a fresh temp file
/// path) through the platform shell, and interprets the result.
///
/// `command_template` must contain a literal `{path}` placeholder — that's
/// how the app tells the OS tool where to write the capture, and how tools
/// needing shell features (e.g. `grim -g "$(slurp)" {path}`) still work,
/// since the whole string is handed to `sh -c` / `cmd /C` rather than
/// exec'd directly.
pub fn capture(command_template: &str) -> Result<CaptureOutcome, String> {
    if !command_template.contains("{path}") {
        return Err(
            "Screenshot command must contain a {path} placeholder — check Settings.".to_string(),
        );
    }

    // PID + millisecond timestamp alone can collide (e.g. concurrent test
    // threads share a PID and can land in the same millisecond) — an
    // atomic counter guarantees uniqueness within this process regardless.
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_path = std::env::temp_dir().join(format!(
        "adm-sfa-capture-{}-{}-{n}.png",
        std::process::id(),
        chrono::Local::now().format("%Y%m%d%H%M%S%3f"),
    ));
    // Quoted so a temp dir containing a space (routine on Windows, e.g.
    // `C:\Users\John Doe\AppData\Local\Temp`) doesn't split into multiple
    // shell arguments — command templates aren't expected to pre-quote the
    // placeholder themselves.
    let quoted_path = format!("\"{}\"", temp_path.to_string_lossy());
    let cmd = command_template.replace("{path}", &quoted_path);

    let status = run_shell(&cmd).map_err(|e| format!("Failed to run screenshot command: {e}"))?;

    // Shell convention: exit 127 means the shell itself couldn't find the
    // command (distinct from the *tool* declining/cancelling, which exits
    // with its own non-zero code). Not guaranteed on Windows' cmd.exe, but
    // harmless there — it just falls through to the Cancelled bucket below.
    if status.code() == Some(127) {
        return Err("Screenshot tool not found — check the command in Settings.".to_string());
    }

    if !status.success() || !temp_path.is_file() {
        let _ = std::fs::remove_file(&temp_path);
        return Ok(CaptureOutcome::Cancelled);
    }

    Ok(CaptureOutcome::Success(temp_path))
}

#[cfg(unix)]
fn run_shell(cmd: &str) -> std::io::Result<ExitStatus> {
    Command::new("sh").arg("-c").arg(cmd).status()
}

#[cfg(windows)]
fn run_shell(cmd: &str) -> std::io::Result<ExitStatus> {
    Command::new("cmd").arg("/C").arg(cmd).status()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_command_with_no_path_placeholder() {
        let err = capture("printf x").unwrap_err();
        assert!(err.contains("{path}"));
    }

    #[test]
    fn success_when_the_command_writes_the_path_and_exits_zero() {
        match capture("printf x > {path}").unwrap() {
            CaptureOutcome::Success(path) => {
                assert!(path.is_file());
                std::fs::remove_file(path).unwrap();
            }
            CaptureOutcome::Cancelled => panic!("expected Success, got Cancelled"),
        }
    }

    #[test]
    fn non_zero_exit_with_no_file_is_treated_as_cancelled_not_an_error() {
        // `false` always exits 1 and writes nothing — the common shape of a
        // user pressing Escape in a region-select tool.
        assert!(matches!(
            capture("false {path}").unwrap(),
            CaptureOutcome::Cancelled
        ));
    }

    #[test]
    fn command_not_found_is_a_distinct_error_not_a_cancel() {
        let err = capture("adm-sfa-definitely-not-a-real-command {path}").unwrap_err();
        assert!(err.contains("not found"));
    }
}
