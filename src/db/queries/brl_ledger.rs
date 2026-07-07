use crate::model::transaction::{BrlTxRow, BrlTxType};
use rust_decimal::Decimal;
use rusqlite::{Connection, Result};

pub fn list(conn: &Connection) -> Result<Vec<BrlTxRow>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.date, t.type, t.amount, t.note,
                t.linked_transfer_id, t.linked_purchase_id, t.linked_outbound_event_id,
                p.channel,
                rp.name
           FROM brl_transaction t
           LEFT JOIN purchase          p  ON p.id  = t.linked_purchase_id
           LEFT JOIN outbound_event    oe ON oe.id = t.linked_outbound_event_id
           LEFT JOIN recipient_project rp ON rp.id = oe.recipient_project_id
          ORDER BY t.date DESC, t.id DESC",
    )?;
    let raw = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<i64>>(5)?,
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
        note,
        linked_transfer_id,
        linked_purchase_id,
        linked_outbound_event_id,
        purchase_channel,
        recipient_name,
    ) in raw
    {
        let tx_type = BrlTxType::from_str(&type_str)
            .ok_or_else(|| invalid_enum(2, &type_str))?;
        let amount = parse_decimal(3, &amount_str)?;
        rows.push(BrlTxRow {
            id,
            date,
            tx_type,
            amount,
            note,
            linked_transfer_id,
            linked_purchase_id,
            linked_outbound_event_id,
            purchase_channel,
            recipient_name,
        });
    }
    Ok(rows)
}

fn parse_decimal(col: usize, s: &str) -> rusqlite::Result<Decimal> {
    s.parse::<Decimal>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(col, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn invalid_enum(col: usize, val: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        col,
        rusqlite::types::Type::Text,
        format!("unknown discriminant: {val:?}").into(),
    )
}
