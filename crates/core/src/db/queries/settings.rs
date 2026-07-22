use rusqlite::{params, Connection, OptionalExtension, Result};

pub fn get(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM app_setting WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
}

pub fn set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO app_setting (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../../../schema.sql"))
            .unwrap();
        conn
    }

    #[test]
    fn missing_key_returns_none() {
        let conn = test_db();
        assert_eq!(get(&conn, "nope").unwrap(), None);
    }

    #[test]
    fn set_then_get_round_trips() {
        let conn = test_db();
        set(&conn, "screenshot_command", "maim -s {path}").unwrap();
        assert_eq!(
            get(&conn, "screenshot_command").unwrap(),
            Some("maim -s {path}".to_string())
        );
    }

    #[test]
    fn set_twice_upserts_rather_than_erroring() {
        let conn = test_db();
        set(&conn, "screenshot_command", "first").unwrap();
        set(&conn, "screenshot_command", "second").unwrap();
        assert_eq!(
            get(&conn, "screenshot_command").unwrap(),
            Some("second".to_string())
        );
    }
}
