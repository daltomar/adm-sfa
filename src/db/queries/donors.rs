use crate::model::donor::{Donor, DonorDraft, PhysicalDonation, PhysicalDonationDraft};
use rusqlite::{params, Connection, Result};

pub fn list(conn: &Connection) -> Result<Vec<Donor>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, contact_info, notes
           FROM donor
          ORDER BY name COLLATE NOCASE",
    )?;
    let donors = stmt
        .query_map([], |row| {
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
        params![
            draft.name.trim(),
            super::opt(&draft.contact_info),
            super::opt(&draft.notes)
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update(conn: &Connection, id: i64, draft: &DonorDraft) -> Result<()> {
    conn.execute(
        "UPDATE donor SET name = ?1, contact_info = ?2, notes = ?3 WHERE id = ?4",
        params![
            draft.name.trim(),
            super::opt(&draft.contact_info),
            super::opt(&draft.notes),
            id
        ],
    )?;
    Ok(())
}

pub fn list_donations(conn: &Connection) -> Result<Vec<PhysicalDonation>> {
    let mut stmt = conn.prepare(
        "SELECT pd.id, pd.donor_id, d.name, pd.date_received, pd.notes
           FROM physical_donation pd
           LEFT JOIN donor d ON d.id = pd.donor_id
          ORDER BY pd.date_received DESC, pd.id DESC",
    )?;
    let donations = stmt
        .query_map([], |row| {
            Ok(PhysicalDonation {
                id: row.get(0)?,
                donor_id: row.get(1)?,
                donor_name: row.get(2)?,
                date_received: row.get(3)?,
                notes: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>>>()?;
    Ok(donations)
}

pub fn insert_donation(conn: &Connection, draft: &PhysicalDonationDraft) -> Result<i64> {
    conn.execute(
        "INSERT INTO physical_donation (donor_id, date_received, notes)
              VALUES (?1, ?2, ?3)",
        params![
            draft.donor_id,
            draft.date_received.trim(),
            super::opt(&draft.notes)
        ],
    )?;
    Ok(conn.last_insert_rowid())
}
