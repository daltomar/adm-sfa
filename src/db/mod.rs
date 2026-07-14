pub mod queries;

use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};
use std::path::Path;

pub fn open_db(data_dir: &Path) -> rusqlite::Result<Connection> {
    let mut conn = Connection::open(data_dir.join("adm-sfa.db"))?;
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
    run_migrations(&mut conn).expect("database migration failed");
    seed_default_settings(&conn).expect("failed to seed default settings");
    Ok(conn)
}

fn run_migrations(conn: &mut Connection) -> rusqlite_migration::Result<()> {
    Migrations::new(vec![
        M::up(include_str!("../../migrations/001_initial.sql")),
        M::up(include_str!(
            "../../migrations/002_purchase_multiple_items.sql"
        )),
        M::up(include_str!(
            "../../migrations/003_purchase_negotiation_status.sql"
        )),
        M::up(include_str!("../../migrations/004_app_setting.sql")),
    ])
    .to_latest(conn)
}

/// Seeds settings that need a value to be usable but can't be seeded from
/// static migration SQL because the default depends on the running OS
/// (`cfg!(target_os)`). Only inserts if missing, so it never clobbers a
/// value the user has already edited in Settings.
fn seed_default_settings(conn: &Connection) -> rusqlite::Result<()> {
    if queries::settings::get(conn, "screenshot_command")?.is_none() {
        queries::settings::set(conn, "screenshot_command", default_screenshot_command())?;
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
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_cleanly_in_order() {
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations(&mut conn).unwrap();
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
        run_migrations(&mut conn).unwrap();

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
}
