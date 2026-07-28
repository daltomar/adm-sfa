use crate::model::category::Category;
use rusqlite::{params, Connection, Result};

pub fn list(conn: &Connection) -> Result<Vec<Category>> {
    let mut stmt = conn.prepare("SELECT id, name FROM category ORDER BY name COLLATE NOCASE")?;
    let categories = stmt
        .query_map([], |row| {
            Ok(Category {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>>>()?;
    Ok(categories)
}

pub fn insert(conn: &Connection, name: &str) -> Result<i64> {
    let name = super::require_name("category name", name)?;
    conn.execute("INSERT INTO category (name) VALUES (?1)", params![name])?;
    Ok(conn.last_insert_rowid())
}

pub fn update(conn: &Connection, id: i64, name: &str) -> Result<()> {
    let name = super::require_name("category name", name)?;
    let changed = conn.execute(
        "UPDATE category SET name = ?1 WHERE id = ?2",
        params![name, id],
    )?;
    if changed == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    Ok(())
}

pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM category WHERE id = ?1", [id])?;
    Ok(())
}

pub fn in_use(conn: &Connection, id: i64) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM inventory_item WHERE category_id = ?1",
        [id],
        |row| row.get(0),
    )?;
    Ok(count > 0)
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
    fn blank_name_is_rejected_on_insert() {
        let conn = test_db();
        let before = list(&conn).unwrap().len();
        assert!(insert(&conn, "   ").is_err());
        assert_eq!(list(&conn).unwrap().len(), before);
    }

    #[test]
    fn blank_name_is_rejected_on_update() {
        let conn = test_db();
        let id = insert(&conn, "Decks").unwrap();
        assert!(update(&conn, id, "").is_err());
        let row = list(&conn)
            .unwrap()
            .into_iter()
            .find(|c| c.id == id)
            .unwrap();
        assert_eq!(row.name, "Decks");
    }
}
