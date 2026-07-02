use crate::model::transfer::{AnnualTransfer, TransferDraft};
use rust_decimal::Decimal;
use rusqlite::{params, Connection, Result};
use std::str::FromStr;

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
            eur_amount_sent: Decimal::from_str(&eur_str).unwrap_or_default(),
            exchange_rate: Decimal::from_str(&rate_str).unwrap_or_default(),
            brl_amount_received: Decimal::from_str(&brl_str).unwrap_or_default(),
            notes,
        });
    }
    Ok(transfers)
}

pub fn insert(conn: &Connection, draft: &TransferDraft) -> Result<i64> {
    let eur_amount: Decimal = draft.eur_amount_sent_str.trim().parse().unwrap_or_default();
    let rate: Decimal = draft.exchange_rate_str.trim().parse().unwrap_or_default();
    let brl_amount = eur_amount * rate;

    conn.execute_batch("BEGIN")?;
    let result: Result<i64> = (|| {
        conn.execute(
            "INSERT INTO annual_transfer
                    (date, eur_amount_sent, exchange_rate, brl_amount_received, notes)
                  VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                draft.date.trim(),
                eur_amount.to_string(),
                rate.to_string(),
                brl_amount.to_string(),
                opt(&draft.notes),
            ],
        )?;
        let transfer_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO eur_transaction (date, type, amount, linked_transfer_id)
                  VALUES (?1, 'transfer_to_brl_out', ?2, ?3)",
            params![draft.date.trim(), eur_amount.to_string(), transfer_id],
        )?;
        conn.execute(
            "INSERT INTO brl_transaction (date, type, amount, linked_transfer_id)
                  VALUES (?1, 'transfer_in', ?2, ?3)",
            params![draft.date.trim(), brl_amount.to_string(), transfer_id],
        )?;

        Ok(transfer_id)
    })();

    match result {
        Ok(id) => {
            conn.execute_batch("COMMIT")?;
            Ok(id)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

pub fn update(conn: &Connection, id: i64, draft: &TransferDraft) -> Result<()> {
    let eur_amount: Decimal = draft.eur_amount_sent_str.trim().parse().unwrap_or_default();
    let rate: Decimal = draft.exchange_rate_str.trim().parse().unwrap_or_default();
    let brl_amount = eur_amount * rate;

    conn.execute_batch("BEGIN")?;
    let result: Result<()> = (|| {
        conn.execute(
            "UPDATE annual_transfer
                SET date = ?1, eur_amount_sent = ?2, exchange_rate = ?3,
                    brl_amount_received = ?4, notes = ?5
              WHERE id = ?6",
            params![
                draft.date.trim(),
                eur_amount.to_string(),
                rate.to_string(),
                brl_amount.to_string(),
                opt(&draft.notes),
                id,
            ],
        )?;

        // Delete-and-recreate the linked ledger entries so date/amount changes propagate.
        conn.execute(
            "DELETE FROM eur_transaction WHERE linked_transfer_id = ?1 AND type = 'transfer_to_brl_out'",
            [id],
        )?;
        conn.execute(
            "DELETE FROM brl_transaction WHERE linked_transfer_id = ?1 AND type = 'transfer_in'",
            [id],
        )?;

        conn.execute(
            "INSERT INTO eur_transaction (date, type, amount, linked_transfer_id)
                  VALUES (?1, 'transfer_to_brl_out', ?2, ?3)",
            params![draft.date.trim(), eur_amount.to_string(), id],
        )?;
        conn.execute(
            "INSERT INTO brl_transaction (date, type, amount, linked_transfer_id)
                  VALUES (?1, 'transfer_in', ?2, ?3)",
            params![draft.date.trim(), brl_amount.to_string(), id],
        )?;

        Ok(())
    })();

    match result {
        Ok(()) => {
            conn.execute_batch("COMMIT")?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

fn opt(s: &str) -> Option<&str> {
    let t = s.trim();
    if t.is_empty() { None } else { Some(t) }
}
