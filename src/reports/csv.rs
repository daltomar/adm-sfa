use std::path::Path;

/// CSV export stub — to be implemented alongside the reporting views.
pub fn export(_dest: &Path) -> Result<(), Box<dyn std::error::Error>> {
    Err("CSV export not yet implemented".into())
}
