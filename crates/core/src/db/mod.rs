pub mod queries;

use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};
use std::path::Path;

pub fn open_db(data_dir: &Path) -> Result<Connection, Box<dyn std::error::Error>> {
    let db_path = data_dir.join("adm-sfa.db");
    let mut conn = Connection::open(&db_path)?;
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
    run_migrations(&mut conn, data_dir)?;
    seed_default_settings(&conn)?;
    Ok(conn)
}

fn migrations_list() -> Vec<M<'static>> {
    vec![
        M::up(include_str!("../../migrations/001_initial.sql")),
        M::up(include_str!(
            "../../migrations/002_purchase_multiple_items.sql"
        )),
        M::up(include_str!(
            "../../migrations/003_purchase_negotiation_status.sql"
        )),
        M::up(include_str!("../../migrations/004_app_setting.sql")),
        M::up(include_str!("../../migrations/005_annual_report_draft.sql")),
    ]
}

fn run_migrations(
    conn: &mut Connection,
    data_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let migrations = migrations_list();
    let target_ver = migrations.len();

    let current_ver: usize = {
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        v as usize
    };

    // S0.3: clear error when a newer binary has already migrated this DB.
    if current_ver > target_ver {
        return Err(format!(
            "database schema is at v{current_ver} but this binary only knows \
             v{target_ver} — rebuild and redeploy both adm-sfa-desktop and \
             adm-sfa-web together"
        )
        .into());
    }

    // S0.2: take a consistent pre-migration snapshot before touching a
    // database that has previously-applied migrations (current_ver > 0) and
    // has pending ones (current_ver < target_ver). Using current_ver rather
    // than a filesystem existence check avoids a TOCTOU race and correctly
    // identifies "this DB has real data" regardless of when the file was
    // created.
    if current_ver > 0 && current_ver < target_ver {
        std::fs::create_dir_all(data_dir.join("backups"))?;
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let snapshot = data_dir.join("backups").join(format!(
            "pre-migration-v{current_ver}-to-v{target_ver}-{timestamp}.db"
        ));
        crate::backup::vacuum_into(conn, &snapshot)?;
    }

    Migrations::new(migrations).to_latest(conn)?;
    Ok(())
}

/// Seeds settings that need a value to be usable but can't be seeded from
/// static migration SQL because the default depends on the running OS
/// (`cfg!(target_os)`). Only inserts if missing, so it never clobbers a
/// value the user has already edited in Settings.
fn seed_default_settings(conn: &Connection) -> rusqlite::Result<()> {
    if queries::settings::get(conn, "screenshot_command")?.is_none() {
        queries::settings::set(conn, "screenshot_command", default_screenshot_command())?;
    }
    // ui_locale (SPEC.md §6.2): "en" | "de" | "pt-BR", validated in Rust only
    // (settings.rs), not via a DB CHECK constraint — app_setting is a
    // generic key/value table with no per-key constraint mechanism, see
    // NewFeature-Translation-Review.md §1.1.
    if queries::settings::get(conn, "ui_locale")?.is_none() {
        queries::settings::set(conn, "ui_locale", "en")?;
    }
    // Organization info for statutory reports — seeded blank so the app
    // starts cleanly; the user fills these in via Settings.
    for key in &["org_name", "org_address", "org_tax_number"] {
        if queries::settings::get(conn, key)?.is_none() {
            queries::settings::set(conn, key, "")?;
        }
    }
    Ok(())
}

fn default_screenshot_command() -> &'static str {
    if cfg!(target_os = "linux") {
        "maim -s {path}"
    } else if cfg!(target_os = "macos") {
        "screencapture -i -s {path}"
    } else {
        // No reliable built-in Windows CLI region-capture-to-file command to
        // seed — left blank; the Settings panel prompts the user to enter
        // their own {path}-templated command.
        ""
    }
}

#[cfg(test)]
pub(crate) mod mod_tests {
    use super::*;

    /// Opens a fully-migrated in-memory DB for tests in other modules.
    pub(crate) fn open_full_db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        Migrations::new(migrations_list())
            .to_latest(&mut conn)
            .unwrap();
        seed_default_settings(&conn).unwrap();
        conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "adm-sfa-db-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }

    /// Opens an in-memory connection with the first `n` migrations applied.
    /// Used by S0.4 populated-migration tests and S0.2 snapshot tests.
    fn open_at_migration_level(n: usize) -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        Migrations::new(migrations_list().into_iter().take(n).collect())
            .to_latest(&mut conn)
            .unwrap();
        conn
    }

    // ── Existing tests ──────────────────────────────────────────────────────

    #[test]
    fn migrations_apply_cleanly_in_order() {
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations(&mut conn, Path::new("")).unwrap();
        let status_default: String = conn
            .query_row(
                "SELECT dflt_value FROM pragma_table_info('purchase') WHERE name = 'status'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status_default, "'bought'");
    }

    #[test]
    fn seed_default_settings_populates_screenshot_command_once() {
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations(&mut conn, Path::new("")).unwrap();

        seed_default_settings(&conn).unwrap();
        let seeded = queries::settings::get(&conn, "screenshot_command")
            .unwrap()
            .unwrap();
        assert_eq!(seeded, default_screenshot_command());

        // A user edit must survive a second seeding pass (e.g. next app start).
        queries::settings::set(&conn, "screenshot_command", "my custom command {path}").unwrap();
        seed_default_settings(&conn).unwrap();
        assert_eq!(
            queries::settings::get(&conn, "screenshot_command").unwrap(),
            Some("my custom command {path}".to_string())
        );
    }

    #[test]
    fn seed_default_settings_populates_ui_locale_once() {
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations(&mut conn, Path::new("")).unwrap();

        seed_default_settings(&conn).unwrap();
        assert_eq!(
            queries::settings::get(&conn, "ui_locale").unwrap(),
            Some("en".to_string())
        );

        // A user's chosen locale must survive a second seeding pass.
        queries::settings::set(&conn, "ui_locale", "de").unwrap();
        seed_default_settings(&conn).unwrap();
        assert_eq!(
            queries::settings::get(&conn, "ui_locale").unwrap(),
            Some("de".to_string())
        );
    }

    // ── S0.3: DB-ahead-of-binary guard ──────────────────────────────────────

    #[test]
    fn run_migrations_rejects_a_db_ahead_of_binary() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA user_version = 99").unwrap();
        let err = run_migrations(&mut conn, Path::new(""))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("rebuild and redeploy"),
            "expected a helpful message, got: {err}"
        );
    }

    // ── S0.2: pre-migration snapshot ────────────────────────────────────────

    #[test]
    fn open_db_on_fresh_path_writes_no_snapshot() {
        let tmp = test_dir("fresh");
        std::fs::create_dir_all(&tmp).unwrap();
        open_db(&tmp).unwrap();
        assert!(
            !tmp.join("backups").exists(),
            "fresh DB must not create a backups/ directory"
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn open_db_with_pending_migrations_writes_snapshot() {
        let tmp = test_dir("pending");
        std::fs::create_dir_all(&tmp).unwrap();

        // Create a DB at migration level 3 (one behind current v4) by
        // opening it directly, bypassing open_db.
        {
            let mut conn = Connection::open(tmp.join("adm-sfa.db")).unwrap();
            conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
            Migrations::new(migrations_list().into_iter().take(3).collect())
                .to_latest(&mut conn)
                .unwrap();
        }

        // open_db should detect the pending migration and write a snapshot.
        open_db(&tmp).unwrap();

        let backups_dir = tmp.join("backups");
        assert!(backups_dir.exists(), "backups/ must have been created");
        let snapshots: Vec<_> = std::fs::read_dir(&backups_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(
            snapshots.len(),
            1,
            "expected exactly one pre-migration snapshot"
        );

        // The snapshot must be a valid, openable SQLite database.
        let snap_path = snapshots[0].path();
        let conn = Connection::open(&snap_path).unwrap();
        let integrity: String = conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn open_db_already_up_to_date_writes_no_snapshot() {
        let tmp = test_dir("uptodate");
        std::fs::create_dir_all(&tmp).unwrap();

        open_db(&tmp).unwrap(); // first open: fresh DB, no snapshot
        assert!(!tmp.join("backups").exists());

        open_db(&tmp).unwrap(); // second open: already at latest, no snapshot
        assert!(!tmp.join("backups").exists());

        std::fs::remove_dir_all(&tmp).ok();
    }

    // ── S0.4: populated-migration tests ─────────────────────────────────────

    #[test]
    fn migration_002_preserves_existing_purchases() {
        // Build DB at v1 (001_initial only), insert a purchase using only the
        // columns that exist before migration 002.
        let mut conn = open_at_migration_level(1);
        conn.execute(
            "INSERT INTO purchase (date, currency, cost, channel) \
             VALUES ('2026-01-01', 'EUR', '10.00', 'test')",
            [],
        )
        .unwrap();

        // Apply migrations up to v2.
        Migrations::new(migrations_list().into_iter().take(2).collect())
            .to_latest(&mut conn)
            .unwrap();

        // The row must still be present and multiple_items must be 0 (the
        // migration's DEFAULT — no inventory items were linked to this
        // purchase so the backfill UPDATE leaves it at 0).
        let (count, mi): (i64, i64) = conn
            .query_row("SELECT COUNT(*), multiple_items FROM purchase", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(mi, 0);
    }

    #[test]
    fn migration_003_preserves_existing_purchases() {
        // Build DB at v2, insert a purchase (multiple_items is now present).
        let mut conn = open_at_migration_level(2);
        conn.execute(
            "INSERT INTO purchase (date, currency, cost, channel, multiple_items) \
             VALUES ('2026-01-01', 'EUR', '10.00', 'test', 0)",
            [],
        )
        .unwrap();

        // Apply migrations up to v3.
        Migrations::new(migrations_list().into_iter().take(3).collect())
            .to_latest(&mut conn)
            .unwrap();

        // The row must still be present and status must be 'bought' (the
        // migration's DEFAULT).
        let (count, status): (i64, String) = conn
            .query_row("SELECT COUNT(*), status FROM purchase", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(status, "bought");
    }
}
