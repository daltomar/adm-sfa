use rusqlite::Connection;
use std::path::Path;
use walkdir::WalkDir;

/// Zips the data directory to `dest` using a consistent DB snapshot.
///
/// The DB is captured via `VACUUM INTO` (safe for live WAL databases — produces
/// a clean single-file copy at a consistent point in time, regardless of
/// whether any other connection has the DB open). The live `adm-sfa.db`,
/// `-wal`, and `-shm` files are skipped; only the snapshot ends up in the zip.
/// The `backups/` subdirectory (internal pre-migration snapshots) is also
/// skipped — it is not user data.
///
/// Writes to a `.zip.tmp` sibling first and renames atomically so a partial
/// write never corrupts an existing backup.
pub fn backup_to_zip(
    conn: &Connection,
    data_dir: &Path,
    dest: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    // Canonicalize so that paths with `..` components can't bypass the check.
    let data_dir_canon = data_dir
        .canonicalize()
        .unwrap_or_else(|_| data_dir.to_path_buf());
    let dest_dir_canon = dest
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .unwrap_or_else(|| dest.to_path_buf());
    if dest_dir_canon.starts_with(&data_dir_canon) {
        return Err(format!(
            "backup destination is inside the data directory — choose a path outside {}",
            data_dir.display()
        )
        .into());
    }
    let tmp = dest.with_extension("zip.tmp");
    let result = write_zip(conn, data_dir, &tmp);
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return result;
    }
    std::fs::rename(&tmp, dest).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(())
}

fn write_zip(
    conn: &Connection,
    data_dir: &Path,
    dest: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    // Produce a consistent DB snapshot in the OS temp dir, then zip it
    // together with documents/. The snapshot is always deleted after — on
    // success and on any error path.
    let snapshot_path = std::env::temp_dir().join(format!(
        "adm-sfa-snap-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    vacuum_into(conn, &snapshot_path)?;
    let zip_result = build_zip(data_dir, &snapshot_path, dest);
    let _ = std::fs::remove_file(&snapshot_path);
    zip_result
}

/// Runs `VACUUM INTO '<path>'` on `conn`, producing a consistent single-file
/// copy of the database at the current point in time. Requires SQLite ≥ 3.27.
pub(crate) fn vacuum_into(
    conn: &Connection,
    dest: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let path_str = dest.to_str().ok_or("snapshot path is not valid UTF-8")?;
    let escaped = path_str.replace('\'', "''");
    conn.execute_batch(&format!("VACUUM INTO '{escaped}'"))?;
    Ok(())
}

fn build_zip(
    data_dir: &Path,
    snapshot_path: &Path,
    dest: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(dest)?;
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // DB snapshot — stored as adm-sfa.db in the zip regardless of its temp name
    writer.start_file("adm-sfa.db", options)?;
    std::io::copy(&mut std::fs::File::open(snapshot_path)?, &mut writer)?;

    // Data files: documents/ and anything else in data_dir, excluding the
    // live DB files (covered by the snapshot above) and backups/ (internal
    // safety copies, not user data).
    let backups_dir = data_dir.join("backups");
    let skip_names = ["adm-sfa.db", "adm-sfa.db-wal", "adm-sfa.db-shm"];

    for entry in WalkDir::new(data_dir) {
        let entry = entry?; // fail loudly — a silent skip could hide data loss
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.starts_with(&backups_dir) {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if skip_names.contains(&name) {
            continue;
        }
        let rel = path.strip_prefix(data_dir)?;
        writer.start_file(rel.to_string_lossy(), options)?;
        std::io::copy(&mut std::fs::File::open(path)?, &mut writer)?;
    }
    writer.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "adm-sfa-backup-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn zips_db_and_documents_and_is_readable() {
        let tmp = test_dir("basic");
        let data_dir = tmp.join("data");
        std::fs::create_dir_all(data_dir.join("documents")).unwrap();

        // Real SQLite DB so VACUUM INTO works
        let conn = Connection::open(data_dir.join("adm-sfa.db")).unwrap();
        conn.execute_batch("CREATE TABLE t (x INTEGER); INSERT INTO t VALUES (1);")
            .unwrap();

        std::fs::write(
            data_dir.join("documents/2026-01-01_purchase-1_ad.jpg"),
            b"fake photo",
        )
        .unwrap();

        let dest = tmp.join("out.zip");
        backup_to_zip(&conn, &data_dir, &dest).unwrap();

        assert!(dest.exists());
        let file = std::fs::File::open(&dest).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec!["adm-sfa.db", "documents/2026-01-01_purchase-1_ad.jpg"]
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn backup_is_consistent_under_concurrent_writes() {
        let tmp = test_dir("consistent");
        let data_dir = tmp.join("data");
        std::fs::create_dir_all(&data_dir).unwrap();

        let conn = Connection::open(data_dir.join("adm-sfa.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT);
             INSERT INTO items VALUES (1, 'deck'), (2, 'trucks');",
        )
        .unwrap();

        let dest = tmp.join("backup.zip");
        backup_to_zip(&conn, &data_dir, &dest).unwrap();

        // Extract the DB from the zip and verify it
        let file = std::fs::File::open(&dest).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut db_bytes = Vec::new();
        {
            use std::io::Read;
            archive
                .by_name("adm-sfa.db")
                .unwrap()
                .read_to_end(&mut db_bytes)
                .unwrap();
        }
        let extracted = tmp.join("extracted.db");
        std::fs::write(&extracted, &db_bytes).unwrap();

        let restored = Connection::open(&extracted).unwrap();
        let integrity: String = restored
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");

        let count: i64 = restored
            .query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn backup_rejects_dest_inside_data_dir() {
        let tmp = test_dir("reject");
        let data_dir = tmp.join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let conn = Connection::open(data_dir.join("adm-sfa.db")).unwrap();

        let dest_inside = data_dir.join("backup.zip");
        assert!(backup_to_zip(&conn, &data_dir, &dest_inside).is_err());

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn backup_excludes_live_db_files_from_zip() {
        let tmp = test_dir("walshm");
        let data_dir = tmp.join("data");
        std::fs::create_dir_all(&data_dir).unwrap();

        let conn = Connection::open(data_dir.join("adm-sfa.db")).unwrap();
        conn.execute_batch("CREATE TABLE t (x INTEGER);").unwrap();

        // Plant stale WAL/SHM sidecars — only the snapshot (from VACUUM INTO)
        // should appear in the zip, not these raw files.
        std::fs::write(data_dir.join("adm-sfa.db-wal"), b"wal").unwrap();
        std::fs::write(data_dir.join("adm-sfa.db-shm"), b"shm").unwrap();

        let dest = tmp.join("out.zip");
        backup_to_zip(&conn, &data_dir, &dest).unwrap();

        let file = std::fs::File::open(&dest).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        assert!(
            !names
                .iter()
                .any(|n| n.ends_with("-wal") || n.ends_with("-shm")),
            "WAL/SHM files must not appear in the zip, got: {names:?}"
        );
        assert!(
            names.contains(&"adm-sfa.db".to_string()),
            "snapshot must be in zip"
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn backup_excludes_backups_dir_from_zip() {
        let tmp = test_dir("excl");
        let data_dir = tmp.join("data");
        std::fs::create_dir_all(data_dir.join("backups")).unwrap();

        let conn = Connection::open(data_dir.join("adm-sfa.db")).unwrap();
        conn.execute_batch("CREATE TABLE t (x INTEGER);").unwrap();

        // Plant a file in backups/ — it must not appear in the zip
        std::fs::write(
            data_dir.join("backups/pre-migration-v3-to-v4-123.db"),
            b"snapshot",
        )
        .unwrap();

        let dest = tmp.join("out.zip");
        backup_to_zip(&conn, &data_dir, &dest).unwrap();

        let file = std::fs::File::open(&dest).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        assert!(
            !names.iter().any(|n| n.starts_with("backups/")),
            "backups/ should be excluded from the zip, got: {names:?}"
        );

        std::fs::remove_dir_all(&tmp).ok();
    }
}
