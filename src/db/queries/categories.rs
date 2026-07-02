use crate::model::category::Category;
use rusqlite::{Connection, Result};

pub fn list(conn: &Connection) -> Result<Vec<Category>> {
    let mut stmt = conn.prepare("SELECT id, name FROM category ORDER BY name COLLATE NOCASE")?;
    let categories = stmt
        .query_map([], |row| Ok(Category { id: row.get(0)?, name: row.get(1)? }))?
        .collect::<Result<Vec<_>>>()?;
    Ok(categories)
}
