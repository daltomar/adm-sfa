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
    conn.execute("INSERT INTO category (name) VALUES (?1)", params![name.trim()])?;
    Ok(conn.last_insert_rowid())
}

pub fn update(conn: &Connection, id: i64, name: &str) -> Result<()> {
    let changed = conn.execute(
        "UPDATE category SET name = ?1 WHERE id = ?2",
        params![name.trim(), id],
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
