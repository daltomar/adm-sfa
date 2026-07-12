use crate::model::outbound::{
    OutboundEventDraft, OutboundEventRow, RecipientProject, RecipientProjectDraft,
};
use rusqlite::{params, Connection, Result};
use rust_decimal::Decimal;
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

pub fn insert(conn: &Connection, draft: &OutboundEventDraft, item_ids: &[i64]) -> Result<i64> {
    let cash_amount = parse_cash(&draft.cash_amount_brl_str);
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO outbound_event (date, recipient_project_id, cash_amount_brl, notes)
              VALUES (?1, ?2, ?3, ?4)",
        params![
            draft.date.trim(),
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
            params![draft.date.trim(), amount.to_string(), event_id],
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
    let cash_amount = parse_cash(&draft.cash_amount_brl_str);
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE outbound_event
            SET date = ?1, recipient_project_id = ?2, cash_amount_brl = ?3, notes = ?4
          WHERE id = ?5",
        params![
            draft.date.trim(),
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
            params![draft.date.trim(), amount.to_string(), id],
        )?;
    }
    tx.commit()?;
    Ok(())
}

fn link_items(conn: &Connection, event_id: i64, item_ids: &[i64]) -> Result<()> {
    for item_id in item_ids {
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
