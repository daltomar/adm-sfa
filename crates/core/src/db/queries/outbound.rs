use crate::model::outbound::{
    OutboundEventDraft, OutboundEventRow, RecipientProject, RecipientProjectDraft,
};
use rusqlite::{params, Connection, Result};
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::str::FromStr;

pub fn list_recipient_projects(conn: &Connection) -> Result<Vec<RecipientProject>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, contact_info, location, active
           FROM recipient_project
          ORDER BY active DESC, name COLLATE NOCASE",
    )?;
    let projects = stmt
        .query_map([], |row| {
            Ok(RecipientProject {
                id: row.get(0)?,
                name: row.get(1)?,
                contact_info: row.get(2)?,
                location: row.get(3)?,
                active: row.get::<_, i64>(4)? != 0,
            })
        })?
        .collect::<Result<Vec<_>>>()?;
    Ok(projects)
}

pub fn insert_recipient_project(conn: &Connection, draft: &RecipientProjectDraft) -> Result<i64> {
    conn.execute(
        "INSERT INTO recipient_project (name, contact_info, location, active)
              VALUES (?1, ?2, ?3, ?4)",
        params![
            draft.name.trim(),
            super::opt(&draft.contact_info),
            super::opt(&draft.location),
            draft.active as i64,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn list(conn: &Connection) -> Result<Vec<OutboundEventRow>> {
    let mut stmt = conn.prepare(
        "SELECT oe.id, oe.date, oe.recipient_project_id, rp.name,
                oe.cash_amount_brl, oe.notes,
                (SELECT COUNT(*) FROM outbound_event_item oei
                  WHERE oei.outbound_event_id = oe.id)
           FROM outbound_event oe
           JOIN recipient_project rp ON rp.id = oe.recipient_project_id
          ORDER BY oe.date DESC, oe.id DESC",
    )?;
    let raw = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })?
        .collect::<Result<Vec<_>>>()?;

    let mut events = Vec::with_capacity(raw.len());
    for (id, date, recipient_project_id, recipient_name, cash_str, notes, item_count) in raw {
        let cash_amount_brl = cash_str.and_then(|s| Decimal::from_str(&s).ok());
        events.push(OutboundEventRow {
            id,
            date,
            recipient_project_id,
            recipient_name,
            cash_amount_brl,
            notes,
            item_count,
        });
    }
    Ok(events)
}

pub fn item_ids_for_event(conn: &Connection, event_id: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT inventory_item_id FROM outbound_event_item WHERE outbound_event_id = ?1",
    )?;
    let ids = stmt
        .query_map([event_id], |row| row.get::<_, i64>(0))?
        .collect::<Result<Vec<_>>>()?;
    Ok(ids)
}

/// Item names given per outbound event, for reports — avoids an N+1 query
/// per event compared to calling `item_ids_for_event` in a loop.
pub fn item_names_by_event(conn: &Connection) -> Result<HashMap<i64, Vec<String>>> {
    let mut stmt = conn.prepare(
        "SELECT oei.outbound_event_id, i.name
           FROM outbound_event_item oei
           JOIN inventory_item i ON i.id = oei.inventory_item_id
          ORDER BY oei.outbound_event_id, i.name COLLATE NOCASE",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>>>()?;
    let mut map: HashMap<i64, Vec<String>> = HashMap::new();
    for (event_id, name) in rows {
        map.entry(event_id).or_default().push(name);
    }
    Ok(map)
}

pub fn insert(conn: &Connection, draft: &OutboundEventDraft, item_ids: &[i64]) -> Result<i64> {
    let date = parse_date(&draft.date)?;
    let cash_amount = parse_cash(&draft.cash_amount_brl_str);
    require_gift(item_ids, cash_amount)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO outbound_event (date, recipient_project_id, cash_amount_brl, notes)
              VALUES (?1, ?2, ?3, ?4)",
        params![
            date,
            draft.recipient_project_id,
            cash_amount.map(|d| d.to_string()),
            super::opt(&draft.notes),
        ],
    )?;
    let event_id = tx.last_insert_rowid();
    link_items(&tx, event_id, item_ids)?;
    if let Some(amount) = cash_amount.filter(|d| *d > Decimal::ZERO) {
        tx.execute(
            "INSERT INTO brl_transaction (date, type, amount, linked_outbound_event_id)
                  VALUES (?1, 'cash_gift_out', ?2, ?3)",
            params![date, amount.to_string(), event_id],
        )?;
    }
    tx.commit()?;
    Ok(event_id)
}

pub fn update(
    conn: &Connection,
    id: i64,
    draft: &OutboundEventDraft,
    item_ids: &[i64],
) -> Result<()> {
    let date = parse_date(&draft.date)?;
    let cash_amount = parse_cash(&draft.cash_amount_brl_str);
    require_gift(item_ids, cash_amount)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE outbound_event
            SET date = ?1, recipient_project_id = ?2, cash_amount_brl = ?3, notes = ?4
          WHERE id = ?5",
        params![
            date,
            draft.recipient_project_id,
            cash_amount.map(|d| d.to_string()),
            super::opt(&draft.notes),
            id,
        ],
    )?;
    // Release previously-linked items back to available before re-linking the
    // current selection, so items removed from the event aren't left "donated".
    let previous_ids: Vec<i64> = {
        let mut stmt = tx.prepare(
            "SELECT inventory_item_id FROM outbound_event_item WHERE outbound_event_id = ?1",
        )?;
        let ids = stmt
            .query_map([id], |row| row.get::<_, i64>(0))?
            .collect::<Result<Vec<i64>>>()?;
        ids
    };
    for prev_id in &previous_ids {
        tx.execute(
            "UPDATE inventory_item SET status = 'available' WHERE id = ?1",
            params![prev_id],
        )?;
    }
    tx.execute(
        "DELETE FROM outbound_event_item WHERE outbound_event_id = ?1",
        [id],
    )?;
    link_items(&tx, id, item_ids)?;
    // Delete-and-recreate the linked cash-gift ledger entry so amount changes propagate.
    tx.execute(
        "DELETE FROM brl_transaction
          WHERE linked_outbound_event_id = ?1 AND type = 'cash_gift_out'",
        [id],
    )?;
    if let Some(amount) = cash_amount.filter(|d| *d > Decimal::ZERO) {
        tx.execute(
            "INSERT INTO brl_transaction (date, type, amount, linked_outbound_event_id)
                  VALUES (?1, 'cash_gift_out', ?2, ?3)",
            params![date, amount.to_string(), id],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Links each item to the event and marks it donated — only if it's
/// currently `available`. This is the authoritative guard against
/// double-donating or hijacking a reserved item: the caller (view or,
/// eventually, an HTTP handler) may pre-filter its own item picker for UX,
/// but nothing stops a stale selection or an untrusted client from sending
/// an unavailable id here, so this check has to be the one that actually
/// blocks it. Runs inside the caller's transaction, so a rejection rolls
/// back the whole `insert`/`update` (the event row, any earlier releases,
/// any items already linked earlier in this same loop).
fn link_items(conn: &Connection, event_id: i64, item_ids: &[i64]) -> Result<()> {
    for item_id in item_ids {
        let status: String = conn.query_row(
            "SELECT status FROM inventory_item WHERE id = ?1",
            [item_id],
            |row| row.get(0),
        )?;
        if status != "available" {
            return Err(rusqlite::Error::ToSqlConversionFailure(
                format!("item {item_id} is not available (status: {status:?})").into(),
            ));
        }
        conn.execute(
            "INSERT INTO outbound_event_item (outbound_event_id, inventory_item_id)
                  VALUES (?1, ?2)",
            params![event_id, item_id],
        )?;
        conn.execute(
            "UPDATE inventory_item SET status = 'donated' WHERE id = ?1",
            params![item_id],
        )?;
    }
    Ok(())
}

fn parse_cash(s: &str) -> Option<Decimal> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        crate::money::parse_amount_input(t)
    }
}

/// Authoritative guard: an outbound event needs at least one item or a
/// positive cash amount, or it's a no-op donation to nobody. Previously
/// this was only checked client-side (`outbound.rs`'s `gift_ok`/`form_ok`
/// gate on the desktop Save button) — nothing stopped a caller that isn't
/// the desktop UI from creating an empty event.
fn require_gift(item_ids: &[i64], cash_amount: Option<Decimal>) -> rusqlite::Result<()> {
    let has_cash = cash_amount.map(|d| d > Decimal::ZERO).unwrap_or(false);
    if item_ids.is_empty() && !has_cash {
        return Err(rusqlite::Error::ToSqlConversionFailure(
            "an outbound event needs at least one item or a cash amount".into(),
        ));
    }
    Ok(())
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
    use crate::model::inventory::{InventoryItemDraft, ItemStatus, Location, SourceType};
    use crate::model::outbound::RecipientProjectDraft;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../../../schema.sql"))
            .unwrap();
        conn
    }

    fn test_item(conn: &Connection, status: ItemStatus) -> i64 {
        let cat_id = crate::db::queries::categories::insert(conn, "Decks").unwrap();
        crate::db::queries::inventory::insert(
            conn,
            &InventoryItemDraft {
                name: "Test deck".to_string(),
                category_id: Some(cat_id),
                source_type: SourceType::Donation,
                source_donation_id: None,
                source_purchase_id: None,
                location: Location::Germany,
                status,
                notes: String::new(),
            },
        )
        .unwrap()
    }

    fn item_status(conn: &Connection, item_id: i64) -> String {
        conn.query_row(
            "SELECT status FROM inventory_item WHERE id = ?1",
            [item_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn linking_an_available_item_marks_it_donated() {
        let conn = test_db();
        let rp = insert_recipient_project(
            &conn,
            &RecipientProjectDraft {
                name: "Test Project".to_string(),
                contact_info: String::new(),
                location: String::new(),
                active: true,
            },
        )
        .unwrap();
        let item_id = test_item(&conn, ItemStatus::Available);
        insert(&conn, &draft(rp), &[item_id]).unwrap();
        assert_eq!(item_status(&conn, item_id), "donated");
    }

    #[test]
    fn linking_an_already_donated_item_is_rejected_and_rolls_back() {
        let conn = test_db();
        let rp = insert_recipient_project(
            &conn,
            &RecipientProjectDraft {
                name: "Test Project".to_string(),
                contact_info: String::new(),
                location: String::new(),
                active: true,
            },
        )
        .unwrap();
        let already_donated = test_item(&conn, ItemStatus::Donated);
        assert!(insert(&conn, &draft(rp), &[already_donated]).is_err());
        // The whole insert (including the outbound_event row itself) must
        // have rolled back, not just the item link.
        let event_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM outbound_event", [], |row| row.get(0))
            .unwrap();
        assert_eq!(event_count, 0);
        assert_eq!(item_status(&conn, already_donated), "donated");
    }

    #[test]
    fn linking_a_reserved_item_is_rejected() {
        let conn = test_db();
        let rp = insert_recipient_project(
            &conn,
            &RecipientProjectDraft {
                name: "Test Project".to_string(),
                contact_info: String::new(),
                location: String::new(),
                active: true,
            },
        )
        .unwrap();
        let reserved = test_item(&conn, ItemStatus::Reserved);
        assert!(insert(&conn, &draft(rp), &[reserved]).is_err());
        assert_eq!(item_status(&conn, reserved), "reserved");
    }

    #[test]
    fn update_can_keep_the_same_previously_linked_item() {
        // The item gets released to 'available' before re-linking, so
        // keeping the same selection on an edit must not be rejected as
        // "not available" by the guard that now runs inside link_items.
        let conn = test_db();
        let rp = insert_recipient_project(
            &conn,
            &RecipientProjectDraft {
                name: "Test Project".to_string(),
                contact_info: String::new(),
                location: String::new(),
                active: true,
            },
        )
        .unwrap();
        let item_id = test_item(&conn, ItemStatus::Available);
        let event_id = insert(&conn, &draft(rp), &[item_id]).unwrap();
        update(&conn, event_id, &draft(rp), &[item_id]).unwrap();
        assert_eq!(item_status(&conn, item_id), "donated");
    }

    fn draft(recipient_project_id: i64) -> OutboundEventDraft {
        OutboundEventDraft {
            date: "2026-01-01".to_string(),
            recipient_project_id: Some(recipient_project_id),
            cash_amount_brl_str: String::new(),
            notes: String::new(),
        }
    }

    #[test]
    fn dotted_date_input_round_trips_to_stored_iso() {
        let conn = test_db();
        let rp = insert_recipient_project(
            &conn,
            &RecipientProjectDraft {
                name: "Test Project".to_string(),
                contact_info: String::new(),
                location: String::new(),
                active: true,
            },
        )
        .unwrap();
        let mut d = draft(rp);
        d.date = "16.07.2026".to_string();
        d.cash_amount_brl_str = "5.00".to_string();
        let id = insert(&conn, &d, &[]).unwrap();
        let stored: String = conn
            .query_row(
                "SELECT date FROM outbound_event WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, "2026-07-16");
    }

    #[test]
    fn invalid_date_is_rejected() {
        let conn = test_db();
        let rp = insert_recipient_project(
            &conn,
            &RecipientProjectDraft {
                name: "Test Project".to_string(),
                contact_info: String::new(),
                location: String::new(),
                active: true,
            },
        )
        .unwrap();
        let mut d = draft(rp);
        d.date = "31.02.2026".to_string();
        assert!(insert(&conn, &d, &[]).is_err());
    }

    #[test]
    fn an_event_with_no_items_and_no_cash_is_rejected() {
        let conn = test_db();
        let rp = insert_recipient_project(
            &conn,
            &RecipientProjectDraft {
                name: "Test Project".to_string(),
                contact_info: String::new(),
                location: String::new(),
                active: true,
            },
        )
        .unwrap();
        assert!(insert(&conn, &draft(rp), &[]).is_err());
    }

    #[test]
    fn a_cash_only_gift_with_no_items_is_allowed() {
        let conn = test_db();
        let rp = insert_recipient_project(
            &conn,
            &RecipientProjectDraft {
                name: "Test Project".to_string(),
                contact_info: String::new(),
                location: String::new(),
                active: true,
            },
        )
        .unwrap();
        let mut d = draft(rp);
        d.cash_amount_brl_str = "10.00".to_string();
        insert(&conn, &d, &[]).unwrap();
    }

    #[test]
    fn a_zero_cash_amount_with_no_items_is_still_rejected() {
        let conn = test_db();
        let rp = insert_recipient_project(
            &conn,
            &RecipientProjectDraft {
                name: "Test Project".to_string(),
                contact_info: String::new(),
                location: String::new(),
                active: true,
            },
        )
        .unwrap();
        let mut d = draft(rp);
        d.cash_amount_brl_str = "0.00".to_string();
        assert!(insert(&conn, &d, &[]).is_err());
    }
}
