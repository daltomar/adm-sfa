use crate::model::inventory::{
    InventoryItemDraft, InventoryItemRow, ItemStatus, Location, SourceType,
};
use rusqlite::{params, Connection, Result};

pub fn list(conn: &Connection) -> Result<Vec<InventoryItemRow>> {
    let mut stmt = conn.prepare(
        "SELECT i.id, i.name, i.category_id, c.name,
                i.source_type, i.source_donation_id, i.source_purchase_id,
                dnr.name, pd.date_received, pu.channel,
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
                row.get::<_, String>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, Option<String>>(12)?,
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
        location_str,
        status_str,
        notes,
    ) in raw
    {
        let source_type = SourceType::from_str(&source_type_str)
            .ok_or_else(|| invalid_enum(4, &source_type_str))?;
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
