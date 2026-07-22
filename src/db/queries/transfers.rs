use crate::model::transfer::{AnnualTransfer, TransferDraft};
use rusqlite::{params, Connection, Result};
use rust_decimal::Decimal;

pub fn list(conn: &Connection) -> Result<Vec<AnnualTransfer>> {
    let mut stmt = conn.prepare(
        "SELECT id, date, eur_amount_sent, exchange_rate, brl_amount_received, notes
           FROM annual_transfer
          ORDER BY date DESC, id DESC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?
        .collect::<Result<Vec<_>>>()?;

    let mut transfers = Vec::with_capacity(rows.len());
    for (id, date, eur_str, rate_str, brl_str, notes) in rows {
        transfers.push(AnnualTransfer {
            id,
            date,
            eur_amount_sent: parse_decimal(2, &eur_str)?,
            exchange_rate: parse_decimal(3, &rate_str)?,
            brl_amount_received: parse_decimal(4, &brl_str)?,
            notes,
        });
    }
    Ok(transfers)
}

pub fn insert(conn: &Connection, draft: &TransferDraft) -> Result<i64> {
    let date = parse_date(&draft.date)?;
    let eur_amount = parse_amount(&draft.eur_amount_sent_str)?;
    let rate = parse_amount(&draft.exchange_rate_str)?;
    let brl_amount = eur_amount * rate;

    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO annual_transfer
                (date, eur_amount_sent, exchange_rate, brl_amount_received, notes)
              VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            date,
            eur_amount.to_string(),
            rate.to_string(),
            brl_amount.to_string(),
            super::opt(&draft.notes),
        ],
    )?;
    let transfer_id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO eur_transaction (date, type, amount, linked_transfer_id)
              VALUES (?1, 'transfer_to_brl_out', ?2, ?3)",
        params![date, eur_amount.to_string(), transfer_id],
    )?;
    tx.execute(
        "INSERT INTO brl_transaction (date, type, amount, linked_transfer_id)
              VALUES (?1, 'transfer_in', ?2, ?3)",
        params![date, brl_amount.to_string(), transfer_id],
    )?;
    tx.commit()?;
    Ok(transfer_id)
}

pub fn update(conn: &Connection, id: i64, draft: &TransferDraft) -> Result<()> {
    let date = parse_date(&draft.date)?;
    let eur_amount = parse_amount(&draft.eur_amount_sent_str)?;
    let rate = parse_amount(&draft.exchange_rate_str)?;
    let brl_amount = eur_amount * rate;

    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE annual_transfer
            SET date = ?1, eur_amount_sent = ?2, exchange_rate = ?3,
                brl_amount_received = ?4, notes = ?5
          WHERE id = ?6",
        params![
            date,
            eur_amount.to_string(),
            rate.to_string(),
            brl_amount.to_string(),
            super::opt(&draft.notes),
            id,
        ],
    )?;
    // Delete-and-recreate the linked ledger entries so date/amount changes propagate.
    tx.execute(
        "DELETE FROM eur_transaction WHERE linked_transfer_id = ?1 AND type = 'transfer_to_brl_out'",
        [id],
    )?;
    tx.execute(
        "DELETE FROM brl_transaction WHERE linked_transfer_id = ?1 AND type = 'transfer_in'",
        [id],
    )?;
    tx.execute(
        "INSERT INTO eur_transaction (date, type, amount, linked_transfer_id)
              VALUES (?1, 'transfer_to_brl_out', ?2, ?3)",
        params![date, eur_amount.to_string(), id],
    )?;
    tx.execute(
        "INSERT INTO brl_transaction (date, type, amount, linked_transfer_id)
              VALUES (?1, 'transfer_in', ?2, ?3)",
        params![date, brl_amount.to_string(), id],
    )?;
    tx.commit()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../../../schema.sql"))
            .unwrap();
        conn
    }

    fn draft() -> TransferDraft {
        TransferDraft {
            date: "2026-01-01".to_string(),
            eur_amount_sent_str: "1000.00".to_string(),
            exchange_rate_str: "5.5".to_string(),
            notes: String::new(),
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
                "SELECT date FROM annual_transfer WHERE id = ?1",
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
