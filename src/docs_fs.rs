use std::path::Path;

/// Generates a document filename following the spec pattern:
/// `{date}_{record_type}-{id}_{label}{-n}.{ext}`
pub fn generate_filename(
    date: &str,
    record_type: &str,
    record_id: i64,
    label: &str,
    existing: &[String],
    ext: &str,
) -> String {
    let base = format!("{date}_{record_type}-{record_id}_{label}");
    let candidate = format!("{base}.{ext}");
    if !existing.iter().any(|n| n == &candidate) {
        return candidate;
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("{base}-{n}.{ext}");
        if !existing.iter().any(|name| name == &candidate) {
            return candidate;
        }
        n = n
            .checked_add(1)
            .expect("generate_filename: collision counter overflowed u32");
    }
}

pub fn copy_to_documents(src: &Path, documents_dir: &Path, filename: &str) -> std::io::Result<()> {
    std::fs::copy(src, documents_dir.join(filename))?;
    Ok(())
}

/// Moves a document to `documents/_deleted/` (soft-delete; never erases).
pub fn soft_delete(documents_dir: &Path, filename: &str) -> std::io::Result<()> {
    std::fs::rename(
        documents_dir.join(filename),
        documents_dir.join("_deleted").join(filename),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_collision() {
        let name = generate_filename("2026-06-30", "purchase", 42, "ad", &[], "jpg");
        assert_eq!(name, "2026-06-30_purchase-42_ad.jpg");
    }

    #[test]
    fn collision_appends_counter() {
        let existing = vec![
            "2026-06-30_purchase-42_chat.jpg".to_string(),
            "2026-06-30_purchase-42_chat-2.jpg".to_string(),
        ];
        let name = generate_filename("2026-06-30", "purchase", 42, "chat", &existing, "jpg");
        assert_eq!(name, "2026-06-30_purchase-42_chat-3.jpg");
    }
}
