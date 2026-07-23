use crate::model::transaction::{EurTxDraft, EurTxRow, EurTxType};
use rusqlite::{params, Connection, Result};
use rust_decimal::Decimal;

pub fn list(conn: &Connection) -> Result<Vec<EurTxRow>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.date, t.type, t.amount,
                t.donor_id, t.note, t.linked_purchase_id, t.linked_transfer_id,
                d.name, p.channel
           FROM eur_transaction t
           LEFT JOIN donor   d ON d.id = t.donor_id
           LEFT JOIN purchase p ON p.id = t.linked_purchase_id
          ORDER BY t.date DESC, t.id DESC",
    )?;
    let raw = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
            ))
        })?
        .collect::<Result<Vec<_>>>()?;

    let mut rows = Vec::with_capacity(raw.len());
    for (
        id,
        date,
        type_str,
        amount_str,
        donor_id,
        note,
        linked_purchase_id,
        linked_transfer_id,
        donor_name,
        purchase_channel,
    ) in raw
    {
        let tx_type = EurTxType::from_str(&type_str).ok_or_else(|| invalid_enum(2, &type_str))?;
        let amount = parse_decimal(3, &amount_str)?;
        rows.push(EurTxRow {
            id,
            date,
            tx_type,
            amount,
            donor_id,
            donor_name,
            purchase_channel,
            note,
            linked_purchase_id,
            linked_transfer_id,
        });
    }
    Ok(rows)
}

pub fn insert(conn: &Connection, draft: &EurTxDraft) -> Result<i64> {
    let date = parse_date(&draft.date)?;
    let amount = parse_amount(&draft.amount_str)?;
    conn.execute(
        "INSERT INTO eur_transaction (date, type, amount, donor_id, note)
              VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            date,
            draft.tx_type.as_eur_tx_type().as_str(),
            amount.to_string(),
            draft.donor_id,
            super::opt(&draft.note),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update(conn: &Connection, id: i64, draft: &EurTxDraft) -> Result<()> {
    let date = parse_date(&draft.date)?;
    let amount = parse_amount(&draft.amount_str)?;
    conn.execute(
        "UPDATE eur_transaction
            SET date = ?1, amount = ?2, donor_id = ?3, note = ?4
          WHERE id = ?5",
        params![
            date,
            amount.to_string(),
            draft.donor_id,
            super::opt(&draft.note),
            id
        ],
    )?;
    Ok(())
}

fn parse_decimal(col: usize, s: &str) -> rusqlite::Result<Decimal> {
    s.parse::<Decimal>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(col, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn parse_amount(s: &str) -> rusqlite::Result<Decimal> {
    crate::money::parse_amount_input(s).ok_or_else(|| {
        rusqlite::Error::ToSqlConversionFailure(format!("invalid amount: {s:?}").into())
    })
}

fn parse_date(s: &str) -> rusqlite::Result<String> {
    crate::date::parse_date_input(s)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .ok_or_else(|| {
            rusqlite::Error::ToSqlConversionFailure(format!("invalid date: {s:?}").into())
        })
}

fn invalid_enum(col: usize, val: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        col,
        rusqlite::types::Type::Text,
        format!("unknown discriminant: {val:?}").into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::transaction::ManualEurTxType;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../../../schema.sql"))
            .unwrap();
        conn
    }

    fn draft() -> EurTxDraft {
        EurTxDraft {
            date: "2026-01-01".to_string(),
            tx_type: ManualEurTxType::SelfFundingIn,
            amount_str: "100.00".to_string(),
            donor_id: None,
            note: String::new(),
        }
    }

    #[test]
    fn dotted_date_input_round_trips_to_stored_iso() {
        let conn = test_db();
        let mut d = draft();
        d.date = "16.07.2026".to_string();
        let id = insert(&conn, &d).unwrap();
        let stored: String = conn
            .query_row(
                "SELECT date FROM eur_transaction WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, "2026-07-16");
    }

    #[test]
    fn invalid_date_is_rejected() {
        let conn = test_db();
        let mut d = draft();
        d.date = "31.02.2026".to_string();
        assert!(insert(&conn, &d).is_err());
    }
}
