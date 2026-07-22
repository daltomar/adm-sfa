use crate::model::inventory::{
    InventoryItemDraft, InventoryItemRow, ItemStatus, Location, SourceType,
};
use rusqlite::{params, Connection, Result};

pub fn list(conn: &Connection) -> Result<Vec<InventoryItemRow>> {
    let mut stmt = conn.prepare(
        "SELECT i.id, i.name, i.category_id, c.name,
                i.source_type, i.source_donation_id, i.source_purchase_id,
                dnr.name, pd.date_received, pu.channel, pu.date,
                i.location, i.status, i.notes
           FROM inventory_item i
           JOIN category c            ON c.id = i.category_id
           LEFT JOIN physical_donation pd ON pd.id = i.source_donation_id
           LEFT JOIN donor dnr            ON dnr.id = pd.donor_id
           LEFT JOIN purchase pu          ON pu.id = i.source_purchase_id
          ORDER BY i.id DESC",
    )?;
    let raw = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, String>(12)?,
                row.get::<_, Option<String>>(13)?,
            ))
        })?
        .collect::<Result<Vec<_>>>()?;

    let mut items = Vec::with_capacity(raw.len());
    for (
        id,
        name,
        category_id,
        category_name,
        source_type_str,
        source_donation_id,
        source_purchase_id,
        donor_name,
        date_received,
        purchase_channel,
        purchase_date,
        location_str,
        status_str,
        notes,
    ) in raw
    {
        let source_type = SourceType::from_str(&source_type_str)
            .ok_or_else(|| invalid_enum(4, &source_type_str))?;
        let acquired_date = match source_type {
            SourceType::Donation => date_received.clone(),
            SourceType::Purchase => purchase_date,
        };
        let source_desc = match source_type {
            SourceType::Donation => match (donor_name, date_received) {
                (Some(n), _) => n,
                (None, Some(d)) => format!("Anonymous donation ({d})"),
                (None, None) => "Donation".to_string(),
            },
            SourceType::Purchase => purchase_channel.unwrap_or_else(|| "Purchase".to_string()),
        };
        items.push(InventoryItemRow {
            id,
            name,
            category_id,
            category_name,
            source_type,
            source_donation_id,
            source_purchase_id,
            source_desc,
            acquired_date,
            location: Location::from_str(&location_str)
                .ok_or_else(|| invalid_enum(10, &location_str))?,
            status: ItemStatus::from_str(&status_str)
                .ok_or_else(|| invalid_enum(11, &status_str))?,
            notes,
        });
    }
    Ok(items)
}

pub fn insert(conn: &Connection, draft: &InventoryItemDraft) -> Result<i64> {
    conn.execute(
        "INSERT INTO inventory_item
                (name, category_id, source_type, source_donation_id, source_purchase_id,
                 location, status, notes)
              VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            draft.name.trim(),
            draft.category_id,
            draft.source_type.as_str(),
            draft.source_donation_id,
            draft.source_purchase_id,
            draft.location.as_str(),
            draft.status.as_str(),
            super::opt(&draft.notes),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update(conn: &Connection, id: i64, draft: &InventoryItemDraft) -> Result<()> {
    conn.execute(
        "UPDATE inventory_item
            SET name = ?1, category_id = ?2, source_type = ?3,
                source_donation_id = ?4, source_purchase_id = ?5,
                location = ?6, status = ?7, notes = ?8
          WHERE id = ?9",
        params![
            draft.name.trim(),
            draft.category_id,
            draft.source_type.as_str(),
            draft.source_donation_id,
            draft.source_purchase_id,
            draft.location.as_str(),
            draft.status.as_str(),
            super::opt(&draft.notes),
            id,
        ],
    )?;
    Ok(())
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
    use crate::db::queries::{categories, donors, purchases};
    use crate::model::donor::PhysicalDonationDraft;
    use crate::model::purchase::{Currency, PurchaseDraft, PurchaseStatus};

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../../../schema.sql"))
            .unwrap();
        conn
    }

    #[test]
    fn acquired_date_comes_from_the_matching_source_table() {
        let conn = test_db();
        let cat_id = categories::insert(&conn, "Decks").unwrap();

        let donation_id = donors::insert_donation(
            &conn,
            &PhysicalDonationDraft {
                donor_id: None,
                date_received: "2026-01-05".to_string(),
                notes: String::new(),
            },
        )
        .unwrap();

        let purchase_id = purchases::insert(
            &conn,
            &PurchaseDraft {
                date: "2026-02-10".to_string(),
                currency: Currency::Eur,
                cost_str: "50.00".to_string(),
                channel: "Kleinanzeigen".to_string(),
                seller_info: String::new(),
                multiple_items: false,
                status: PurchaseStatus::Bought,
            },
        )
        .unwrap();

        let donated_id = insert(
            &conn,
            &InventoryItemDraft {
                name: "Donated deck".to_string(),
                category_id: Some(cat_id),
                source_type: SourceType::Donation,
                source_donation_id: Some(donation_id),
                source_purchase_id: None,
                location: Location::Germany,
                status: ItemStatus::Available,
                notes: String::new(),
            },
        )
        .unwrap();

        let bought_id = insert(
            &conn,
            &InventoryItemDraft {
                name: "Bought deck".to_string(),
                category_id: Some(cat_id),
                source_type: SourceType::Purchase,
                source_donation_id: None,
                source_purchase_id: Some(purchase_id),
                location: Location::Germany,
                status: ItemStatus::Available,
                notes: String::new(),
            },
        )
        .unwrap();

        let rows = list(&conn).unwrap();
        let donated = rows.iter().find(|r| r.id == donated_id).unwrap();
        let bought = rows.iter().find(|r| r.id == bought_id).unwrap();

        assert_eq!(donated.acquired_date.as_deref(), Some("2026-01-05"));
        assert_eq!(bought.acquired_date.as_deref(), Some("2026-02-10"));
    }
}
