use std::path::Path;

use rusqlite::Connection;
use rust_i18n::t;

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

/// Soft-deletes a document: moves its file to `documents/_deleted/`, *then*
/// marks the DB row deleted — file first, DB second, deliberately. Doing it
/// in this order (rather than DB-then-file, as every call site used to do
/// inline) means a failure partway through leaves the file still live but
/// the DB row still active too, which self-heals on retry. The reverse
/// order is worse: if the DB commit succeeds but the file move then fails,
/// the row is marked deleted (so `list_for_record` stops returning it —
/// nothing in the UI can reach it to retry) while the file is still sitting
/// live in `documents/`, unreachable *and* still occupying its generated
/// filename — a later document reusing that same generated name would
/// silently overwrite it.
///
/// Idempotent: if the file's already at the `_deleted/` path (e.g. this is
/// a retry after the DB step failed last time), the move is skipped rather
/// than erroring on a missing source file.
pub fn remove_document(
    conn: &Connection,
    documents_dir: &Path,
    doc_id: i64,
    filename: &str,
) -> Result<(), String> {
    let already_moved = documents_dir.join("_deleted").join(filename).is_file();
    if !already_moved {
        soft_delete(documents_dir, filename)
            .map_err(|e| t!("common.doc.error.file_move_failed", error = e).into_owned())?;
    }
    docs_qry::soft_delete(conn, doc_id)
        .map_err(|e| t!("common.doc.error.db_update_failed", error = e).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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

    fn setup_attached_doc(tag: &str) -> (Connection, PathBuf, String, i64) {
        let conn = test_db();
        let tmp = std::env::temp_dir().join(format!(
            "adm-sfa-remove-document-test-{tag}-{}",
            std::process::id()
        ));
        let documents_dir = tmp.join("documents");
        std::fs::create_dir_all(documents_dir.join("_deleted")).unwrap();
        let src = tmp.join("source.png");
        std::fs::write(&src, b"fake png bytes").unwrap();
        let filename = file_document(
            &conn,
            &documents_dir,
            &src,
            "2026-06-30",
            ("purchase", 1),
            "ad",
            &[],
        )
        .unwrap();
        let doc_id: i64 = conn
            .query_row(
                "SELECT id FROM document WHERE filename = ?1",
                [&filename],
                |row| row.get(0),
            )
            .unwrap();
        (conn, documents_dir, filename, doc_id)
    }

    fn is_deleted(conn: &Connection, doc_id: i64) -> bool {
        conn.query_row(
            "SELECT deleted FROM document WHERE id = ?1",
            [doc_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
            != 0
    }

    #[test]
    fn remove_document_moves_the_file_and_marks_it_deleted() {
        let (conn, documents_dir, filename, doc_id) = setup_attached_doc("basic");

        remove_document(&conn, &documents_dir, doc_id, &filename).unwrap();

        assert!(
            !documents_dir.join(&filename).is_file(),
            "must leave the live dir"
        );
        assert!(documents_dir.join("_deleted").join(&filename).is_file());
        assert!(is_deleted(&conn, doc_id));

        std::fs::remove_dir_all(documents_dir.parent().unwrap()).ok();
    }

    #[test]
    fn remove_document_is_idempotent_when_the_file_was_already_moved() {
        // Simulates a retry after a previous attempt's DB step failed: the
        // file is already sitting in `_deleted/`, but the row is still
        // active. Must not error trying to move an already-moved file.
        let (conn, documents_dir, filename, doc_id) = setup_attached_doc("retry");
        std::fs::rename(
            documents_dir.join(&filename),
            documents_dir.join("_deleted").join(&filename),
        )
        .unwrap();

        remove_document(&conn, &documents_dir, doc_id, &filename).unwrap();

        assert!(is_deleted(&conn, doc_id));

        std::fs::remove_dir_all(documents_dir.parent().unwrap()).ok();
    }
}
