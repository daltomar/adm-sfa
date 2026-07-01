use crate::model::document::Document;
use rusqlite::{params, Connection, Result};

pub fn list_for_record(conn: &Connection, record_type: &str, record_id: i64) -> Result<Vec<Document>> {
    let mut stmt = conn.prepare(
        "SELECT id, filename, record_type, record_id, label, deleted
           FROM document
          WHERE record_type = ?1 AND record_id = ?2 AND deleted = 0
          ORDER BY id",
    )?;
    let docs = stmt
        .query_map(params![record_type, record_id], |row| {
            Ok(Document {
                id: row.get(0)?,
                filename: row.get(1)?,
                record_type: row.get(2)?,
                record_id: row.get(3)?,
                label: row.get(4)?,
                deleted: row.get::<_, i32>(5)? != 0,
            })
        })?
        .collect::<Result<Vec<_>>>()?;
    Ok(docs)
}

pub fn labels(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT name FROM document_label ORDER BY id")?;
    let names = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<String>>>()?;
    Ok(names)
}

pub fn insert(
    conn: &Connection,
    record_type: &str,
    record_id: i64,
    filename: &str,
    label: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO document (filename, record_type, record_id, label) VALUES (?1, ?2, ?3, ?4)",
        params![filename, record_type, record_id, label],
    )?;
    Ok(())
}

pub fn soft_delete(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("UPDATE document SET deleted = 1 WHERE id = ?1", [id])?;
    Ok(())
}
