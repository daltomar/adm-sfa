pub mod queries;

use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};
use std::path::Path;

pub fn open_db(data_dir: &Path) -> rusqlite::Result<Connection> {
    let mut conn = Connection::open(data_dir.join("adm-sfa.db"))?;
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
    run_migrations(&mut conn).expect("database migration failed");
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
    ])
    .to_latest(conn)
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
}
