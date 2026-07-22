use std::path::Path;

use rusqlite::Connection;

use crate::db::queries::documents as docs_qry;

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

/// Generates the filename, copies `src` into `documents_dir`, and inserts
/// the document row — the "file an already-on-disk path as a document"
/// sequence shared by drag-and-drop, browse-for-file, and screenshot
/// capture (previously duplicated inline in `purchases.rs`, `transfers.rs`,
/// and `inventory.rs`). Returns the generated filename on success.
pub fn file_document(
    conn: &Connection,
    documents_dir: &Path,
    src: &Path,
    date: &str,
    record: (&str, i64),
    label: &str,
    existing: &[String],
) -> Result<String, String> {
    let (record_type, record_id) = record;
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin")
        .to_lowercase();
    let filename = generate_filename(date, record_type, record_id, label, existing, &ext);
    copy_to_documents(src, documents_dir, &filename).map_err(|e| format!("Copy failed: {e}"))?;
    if let Err(e) = docs_qry::insert(conn, record_type, record_id, &filename, label) {
        // Don't leave an untracked copy behind: with no document row, the UI
        // never lists it for removal and it would collide with a retry under
        // the same generated filename.
        let _ = std::fs::remove_file(documents_dir.join(&filename));
        return Err(format!("DB insert failed: {e}"));
    }
    Ok(filename)
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

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../schema.sql")).unwrap();
        conn
    }

    #[test]
    fn file_document_copies_and_inserts_in_one_call() {
        let conn = test_db();
        let tmp =
            std::env::temp_dir().join(format!("adm-sfa-file-document-test-{}", std::process::id()));
        let documents_dir = tmp.join("documents");
        std::fs::create_dir_all(&documents_dir).unwrap();
        let src = tmp.join("source.png");
        std::fs::write(&src, b"fake png bytes").unwrap();

        let filename = file_document(
            &conn,
            &documents_dir,
            &src,
            "2026-06-30",
            ("purchase", 42),
            "ad",
            &[],
        )
        .unwrap();

        assert_eq!(filename, "2026-06-30_purchase-42_ad.png");
        assert!(documents_dir.join(&filename).is_file());
        let db_filename: String = conn
            .query_row(
                "SELECT filename FROM document WHERE record_type = 'purchase' AND record_id = 42",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(db_filename, filename);

        std::fs::remove_dir_all(&tmp).ok();
    }
}
