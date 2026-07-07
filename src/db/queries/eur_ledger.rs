use crate::model::transaction::{EurTxDraft, EurTxRow, EurTxType};
use rust_decimal::Decimal;
use rusqlite::{params, Connection, Result};

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
    for (id, date, type_str, amount_str, donor_id, note, linked_purchase_id, linked_transfer_id, donor_name, purchase_channel) in raw
    {
        let tx_type = EurTxType::from_str(&type_str)
            .ok_or_else(|| invalid_enum(2, &type_str))?;
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
    let amount = parse_amount(&draft.amount_str)?;
    conn.execute(
        "INSERT INTO eur_transaction (date, type, amount, donor_id, note)
              VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            draft.date.trim(),
            draft.tx_type.as_eur_tx_type().as_str(),
            amount.to_string(),
            draft.donor_id,
            opt(&draft.note),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update(conn: &Connection, id: i64, draft: &EurTxDraft) -> Result<()> {
    let amount = parse_amount(&draft.amount_str)?;
    conn.execute(
        "UPDATE eur_transaction
            SET date = ?1, amount = ?2, donor_id = ?3, note = ?4
          WHERE id = ?5",
        params![draft.date.trim(), amount.to_string(), draft.donor_id, opt(&draft.note), id],
    )?;
    Ok(())
}

fn opt(s: &str) -> Option<&str> {
    let t = s.trim();
    if t.is_empty() { None } else { Some(t) }
}

fn parse_decimal(col: usize, s: &str) -> rusqlite::Result<Decimal> {
    s.parse::<Decimal>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(col, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn parse_amount(s: &str) -> rusqlite::Result<Decimal> {
    s.trim().parse::<Decimal>().map_err(|e| {
        rusqlite::Error::ToSqlConversionFailure(Box::new(e))
    })
}

fn invalid_enum(col: usize, val: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        col,
        rusqlite::types::Type::Text,
        format!("unknown discriminant: {val:?}").into(),
    )
}
