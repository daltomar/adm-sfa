pub enum Location {
    Germany,
    Brazil,
}

pub enum ItemStatus {
    Available,
    Reserved,
    Donated,
}

pub enum SourceType {
    Donation,
    Purchase,
}

pub struct InventoryItem {
    pub id: i64,
    pub name: String,
    pub category_id: i64,
    pub source_type: SourceType,
    pub source_donation_id: Option<i64>,
    pub source_purchase_id: Option<i64>,
    pub location: Location,
    pub status: ItemStatus,
    pub notes: Option<String>,
}
