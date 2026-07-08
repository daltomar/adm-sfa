use crate::model::document::Document;
use rusqlite::{params, Connection, Result};
use std::collections::HashMap;

pub fn list_for_record(
    conn: &Connection,
    record_type: &str,
    record_id: i64,
) -> Result<Vec<Document>> {
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

/// Count of active (non-deleted) documents per (record_type, record_id), for
/// annotating report rows without a query per row.
pub fn counts_by_record(conn: &Connection) -> Result<HashMap<(String, i64), i64>> {
    let mut stmt = conn.prepare(
        "SELECT record_type, record_id, COUNT(*)
           FROM document
          WHERE deleted = 0
          GROUP BY record_type, record_id",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                (row.get::<_, String>(0)?, row.get::<_, i64>(1)?),
                row.get::<_, i64>(2)?,
            ))
        })?
        .collect::<Result<HashMap<_, _>>>()?;
    Ok(rows)
}

pub fn labels(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT name FROM document_label ORDER BY id")?;
    let names = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<String>>>()?;
    Ok(names)
}

pub fn list_labels(conn: &Connection) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare("SELECT id, name FROM document_label ORDER BY id")?;
    let rows = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn insert_label(conn: &Connection, name: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO document_label (name) VALUES (?1)",
        params![name.trim()],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_label(conn: &Connection, id: i64, name: &str) -> Result<()> {
    let changed = conn.execute(
        "UPDATE document_label SET name = ?1 WHERE id = ?2",
        params![name.trim(), id],
    )?;
    if changed == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    Ok(())
}

pub fn delete_label(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM document_label WHERE id = ?1", [id])?;
    Ok(())
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
