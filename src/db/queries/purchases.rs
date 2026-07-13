use crate::model::purchase::{Currency, Purchase, PurchaseDraft, PurchaseStatus};
use rusqlite::{params, Connection, Result};
use rust_decimal::Decimal;

pub fn list(conn: &Connection) -> Result<Vec<Purchase>> {
    let mut stmt = conn.prepare(
        "SELECT id, date, currency, cost, channel, seller_info, multiple_items, status
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
                row.get::<_, i32>(6)?,
                row.get::<_, String>(7)?,
            ))
        })?
        .collect::<Result<Vec<_>>>()?;

    let mut purchases = Vec::with_capacity(rows.len());
    for (id, date, currency_str, cost_str, channel, seller_info, multiple_items, status_str) in rows
    {
        let currency =
            Currency::from_str(&currency_str).ok_or_else(|| invalid_enum(2, &currency_str))?;
        let cost = parse_decimal(3, &cost_str)?;
        let status =
            PurchaseStatus::from_str(&status_str).ok_or_else(|| invalid_enum(7, &status_str))?;
        purchases.push(Purchase {
            id,
            date,
            currency,
            cost,
            channel,
            seller_info,
            multiple_items: multiple_items != 0,
            status,
        });
    }
    Ok(purchases)
}

pub fn insert(conn: &Connection, draft: &PurchaseDraft) -> Result<i64> {
    let cost = parse_amount(&draft.cost_str)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO purchase (date, currency, cost, channel, seller_info, multiple_items, status)
              VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            draft.date.trim(),
            draft.currency.as_str(),
            cost.to_string(),
            draft.channel.trim(),
            super::opt(&draft.seller_info),
            draft.multiple_items as i32,
            draft.status.as_str(),
        ],
    )?;
    let purchase_id = tx.last_insert_rowid();
    // A negotiating purchase writes no ledger row until it transitions to bought.
    if draft.status == PurchaseStatus::Bought {
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
    }
    tx.commit()?;
    Ok(purchase_id)
}

pub fn update(conn: &Connection, id: i64, draft: &PurchaseDraft) -> Result<()> {
    let cost = parse_amount(&draft.cost_str)?;
    let tx = conn.unchecked_transaction()?;

    let current_status_str: String =
        tx.query_row("SELECT status FROM purchase WHERE id = ?1", [id], |row| {
            row.get(0)
        })?;
    let current_status = PurchaseStatus::from_str(&current_status_str)
        .ok_or_else(|| invalid_enum(0, &current_status_str))?;
    // Terminal guard: bought is terminal, so a bought purchase can never
    // revert to negotiating even if a stale draft somehow requests it.
    let target_status = if current_status == PurchaseStatus::Bought {
        PurchaseStatus::Bought
    } else {
        draft.status
    };

    tx.execute(
        "UPDATE purchase
            SET date = ?1, currency = ?2, cost = ?3, channel = ?4, seller_info = ?5,
                multiple_items = ?6, status = ?7
          WHERE id = ?8",
        params![
            draft.date.trim(),
            draft.currency.as_str(),
            cost.to_string(),
            draft.channel.trim(),
            super::opt(&draft.seller_info),
            draft.multiple_items as i32,
            target_status.as_str(),
            id,
        ],
    )?;

    // Only touch the ledger once the purchase is bought. Delete-then-insert
    // handles both an ordinary edit of an already-bought purchase (currency
    // changes, etc. — the delete is a real cleanup) and the first-ever
    // negotiating->bought transition (the delete is a harmless no-op since
    // no ledger row exists yet) with the same code path.
    if target_status == PurchaseStatus::Bought {
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
    }
    tx.commit()?;
    Ok(())
}

/// Hard-deletes a purchase row. Only succeeds while `status = 'negotiating'`
/// — a negotiating purchase has never written a ledger row or been linked
/// to an inventory item, so there is no auditable state to lose. This is
/// the only record-level hard-delete in the codebase; callers must
/// soft-delete any documents already attached to the purchase *before*
/// calling this (see `documents::list_for_record` / `documents::soft_delete`).
pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    let changed = conn.execute(
        "DELETE FROM purchase WHERE id = ?1 AND status = 'negotiating'",
        [id],
    )?;
    if changed == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    Ok(())
}

/// Count of inventory items whose source is this purchase.
/// Used to validate that single-item purchases aren't linked to more than one item.
pub fn linked_item_count(conn: &Connection, purchase_id: i64) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM inventory_item WHERE source_purchase_id = ?1",
        [purchase_id],
        |row| row.get(0),
    )
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

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../../../schema.sql"))
            .unwrap();
        conn
    }

    fn draft(status: PurchaseStatus, currency: Currency) -> PurchaseDraft {
        PurchaseDraft {
            date: "2026-01-01".to_string(),
            currency,
            cost_str: "50.00".to_string(),
            channel: "Kleinanzeigen".to_string(),
            seller_info: String::new(),
            multiple_items: false,
            status,
        }
    }

    fn eur_out_count(conn: &Connection, purchase_id: i64) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM eur_transaction
              WHERE linked_purchase_id = ?1 AND type = 'purchase_out'",
            [purchase_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn purchase_status(conn: &Connection, purchase_id: i64) -> String {
        conn.query_row(
            "SELECT status FROM purchase WHERE id = ?1",
            [purchase_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn negotiating_insert_writes_no_ledger_row() {
        let conn = test_db();
        let id = insert(&conn, &draft(PurchaseStatus::Negotiating, Currency::Eur)).unwrap();
        assert_eq!(eur_out_count(&conn, id), 0);
        assert_eq!(purchase_status(&conn, id), "negotiating");
    }

    #[test]
    fn bought_insert_writes_ledger_row() {
        let conn = test_db();
        let id = insert(&conn, &draft(PurchaseStatus::Bought, Currency::Eur)).unwrap();
        assert_eq!(eur_out_count(&conn, id), 1);
        assert_eq!(purchase_status(&conn, id), "bought");
    }

    #[test]
    fn negotiating_update_stays_negotiating_no_ledger() {
        let conn = test_db();
        let id = insert(&conn, &draft(PurchaseStatus::Negotiating, Currency::Eur)).unwrap();
        let mut d = draft(PurchaseStatus::Negotiating, Currency::Eur);
        d.channel = "Updated channel".to_string();
        update(&conn, id, &d).unwrap();
        assert_eq!(eur_out_count(&conn, id), 0);
        assert_eq!(purchase_status(&conn, id), "negotiating");
    }

    #[test]
    fn negotiating_to_bought_transition_writes_ledger() {
        let conn = test_db();
        let id = insert(&conn, &draft(PurchaseStatus::Negotiating, Currency::Eur)).unwrap();
        update(&conn, id, &draft(PurchaseStatus::Bought, Currency::Eur)).unwrap();
        assert_eq!(eur_out_count(&conn, id), 1);
        assert_eq!(purchase_status(&conn, id), "bought");
    }

    #[test]
    fn bought_to_bought_edit_recreates_exactly_one_ledger_row() {
        let conn = test_db();
        let id = insert(&conn, &draft(PurchaseStatus::Bought, Currency::Eur)).unwrap();
        let mut d = draft(PurchaseStatus::Bought, Currency::Eur);
        d.cost_str = "75.00".to_string();
        update(&conn, id, &d).unwrap();
        assert_eq!(eur_out_count(&conn, id), 1);
        let amount: String = conn
            .query_row(
                "SELECT amount FROM eur_transaction WHERE linked_purchase_id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(amount, "75.00");
    }

    #[test]
    fn bought_purchase_cannot_revert_to_negotiating() {
        let conn = test_db();
        let id = insert(&conn, &draft(PurchaseStatus::Bought, Currency::Eur)).unwrap();
        // A stale/malicious draft claiming negotiating must not undo the
        // terminal bought state or touch the existing ledger row.
        update(
            &conn,
            id,
            &draft(PurchaseStatus::Negotiating, Currency::Eur),
        )
        .unwrap();
        assert_eq!(purchase_status(&conn, id), "bought");
        assert_eq!(eur_out_count(&conn, id), 1);
    }

    #[test]
    fn delete_succeeds_for_negotiating() {
        let conn = test_db();
        let id = insert(&conn, &draft(PurchaseStatus::Negotiating, Currency::Eur)).unwrap();
        delete(&conn, id).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM purchase WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn delete_fails_for_bought() {
        let conn = test_db();
        let id = insert(&conn, &draft(PurchaseStatus::Bought, Currency::Eur)).unwrap();
        assert!(delete(&conn, id).is_err());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM purchase WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }
}
