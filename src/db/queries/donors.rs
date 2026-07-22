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
    let date_received = parse_date(&draft.date_received)?;
    conn.execute(
        "INSERT INTO physical_donation (donor_id, date_received, notes)
              VALUES (?1, ?2, ?3)",
        params![draft.donor_id, date_received, super::opt(&draft.notes)],
    )?;
    Ok(conn.last_insert_rowid())
}

fn parse_date(s: &str) -> rusqlite::Result<String> {
    crate::date::parse_date_input(s)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .ok_or_else(|| {
            rusqlite::Error::ToSqlConversionFailure(format!("invalid date: {s:?}").into())
        })
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
    fn dotted_date_input_round_trips_to_stored_iso() {
        let conn = test_db();
        let draft = PhysicalDonationDraft {
            donor_id: None,
            date_received: "16.07.2026".to_string(),
            notes: String::new(),
        };
        let id = insert_donation(&conn, &draft).unwrap();
        let stored: String = conn
            .query_row(
                "SELECT date_received FROM physical_donation WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, "2026-07-16");
    }

    #[test]
    fn invalid_date_is_rejected() {
        let conn = test_db();
        let draft = PhysicalDonationDraft {
            donor_id: None,
            date_received: "31.02.2026".to_string(),
            notes: String::new(),
        };
        assert!(insert_donation(&conn, &draft).is_err());
    }
}
