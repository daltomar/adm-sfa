use std::path::Path;

/// PDF export via typst-as-lib.
/// Stub — spike typst-as-lib integration before implementing (see stack-plan.md risk note).
#[allow(dead_code)]
pub fn export(_template: &Path, _dest: &Path) -> Result<(), Box<dyn std::error::Error>> {
    Err("PDF export not yet implemented".into())
}
