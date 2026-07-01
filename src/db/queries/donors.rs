use crate::model::donor::{Donor, DonorDraft};
use rusqlite::{params, Connection, Result};

pub fn list(conn: &Connection) -> Result<Vec<Donor>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, contact_info, notes
           FROM donor
          ORDER BY name COLLATE NOCASE",
    )?;
    let donors = stmt.query_map([], |row| {
        Ok(Donor {
            id: row.get(0)?,
            name: row.get(1)?,
            contact_info: row.get(2)?,
            notes: row.get(3)?,
        })
    })?
    .collect::<Result<Vec<_>>>()?;
    Ok(donors)
}

pub fn insert(conn: &Connection, draft: &DonorDraft) -> Result<i64> {
    conn.execute(
        "INSERT INTO donor (name, contact_info, notes) VALUES (?1, ?2, ?3)",
        params![draft.name.trim(), opt(&draft.contact_info), opt(&draft.notes)],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update(conn: &Connection, id: i64, draft: &DonorDraft) -> Result<()> {
    conn.execute(
        "UPDATE donor SET name = ?1, contact_info = ?2, notes = ?3 WHERE id = ?4",
        params![draft.name.trim(), opt(&draft.contact_info), opt(&draft.notes), id],
    )?;
    Ok(())
}

fn opt(s: &str) -> Option<&str> {
    let t = s.trim();
    if t.is_empty() { None } else { Some(t) }
}
