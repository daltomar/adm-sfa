use crate::model::purchase::{Currency, Purchase, PurchaseDraft};
use rusqlite::{params, Connection, Result};
use rust_decimal::Decimal;

pub fn list(conn: &Connection) -> Result<Vec<Purchase>> {
    let mut stmt = conn.prepare(
        "SELECT id, date, currency, cost, channel, seller_info
           FROM purchase
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

    let mut purchases = Vec::with_capacity(rows.len());
    for (id, date, currency_str, cost_str, channel, seller_info) in rows {
        let currency =
            Currency::from_str(&currency_str).ok_or_else(|| invalid_enum(2, &currency_str))?;
        let cost = parse_decimal(3, &cost_str)?;
        purchases.push(Purchase {
            id,
            date,
            currency,
            cost,
            channel,
            seller_info,
        });
    }
    Ok(purchases)
}

pub fn insert(conn: &Connection, draft: &PurchaseDraft) -> Result<i64> {
    let cost = parse_amount(&draft.cost_str)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO purchase (date, currency, cost, channel, seller_info)
              VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            draft.date.trim(),
            draft.currency.as_str(),
            cost.to_string(),
            draft.channel.trim(),
            super::opt(&draft.seller_info),
        ],
    )?;
    let purchase_id = tx.last_insert_rowid();
    match draft.currency {
        Currency::Eur => tx.execute(
            "INSERT INTO eur_transaction (date, type, amount, linked_purchase_id)
                  VALUES (?1, 'purchase_out', ?2, ?3)",
            params![draft.date.trim(), cost.to_string(), purchase_id],
        )?,
        Currency::Brl => tx.execute(
            "INSERT INTO brl_transaction (date, type, amount, linked_purchase_id)
                  VALUES (?1, 'brazil_purchase_out', ?2, ?3)",
            params![draft.date.trim(), cost.to_string(), purchase_id],
        )?,
    };
    tx.commit()?;
    Ok(purchase_id)
}

pub fn update(conn: &Connection, id: i64, draft: &PurchaseDraft) -> Result<()> {
    let cost = parse_amount(&draft.cost_str)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE purchase
            SET date = ?1, currency = ?2, cost = ?3, channel = ?4, seller_info = ?5
          WHERE id = ?6",
        params![
            draft.date.trim(),
            draft.currency.as_str(),
            cost.to_string(),
            draft.channel.trim(),
            super::opt(&draft.seller_info),
            id,
        ],
    )?;
    // Delete-and-recreate the ledger entry so currency changes are handled correctly.
    tx.execute(
        "DELETE FROM eur_transaction WHERE linked_purchase_id = ?1 AND type = 'purchase_out'",
        [id],
    )?;
    tx.execute(
        "DELETE FROM brl_transaction WHERE linked_purchase_id = ?1 AND type = 'brazil_purchase_out'",
        [id],
    )?;
    match draft.currency {
        Currency::Eur => tx.execute(
            "INSERT INTO eur_transaction (date, type, amount, linked_purchase_id)
                  VALUES (?1, 'purchase_out', ?2, ?3)",
            params![draft.date.trim(), cost.to_string(), id],
        )?,
        Currency::Brl => tx.execute(
            "INSERT INTO brl_transaction (date, type, amount, linked_purchase_id)
                  VALUES (?1, 'brazil_purchase_out', ?2, ?3)",
            params![draft.date.trim(), cost.to_string(), id],
        )?,
    };
    tx.commit()?;
    Ok(())
}

fn parse_decimal(col: usize, s: &str) -> rusqlite::Result<Decimal> {
    s.parse::<Decimal>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(col, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn parse_amount(s: &str) -> rusqlite::Result<Decimal> {
    s.trim()
        .parse::<Decimal>()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

fn invalid_enum(col: usize, val: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        col,
        rusqlite::types::Type::Text,
        format!("unknown discriminant: {val:?}").into(),
    )
}
