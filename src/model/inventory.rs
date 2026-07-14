#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    Germany,
    Brazil,
}

impl Location {
    pub fn as_str(self) -> &'static str {
        match self {
            Location::Germany => "germany",
            Location::Brazil => "brazil",
        }
    }

    // Inherent method returning Option rather than impl std::str::FromStr,
    // consistent with the other domain enums in this codebase.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "germany" => Some(Location::Germany),
            "brazil" => Some(Location::Brazil),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Location::Germany => "Germany",
            Location::Brazil => "Brazil",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus {
    Available,
    Reserved,
    Donated,
}

impl ItemStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ItemStatus::Available => "available",
            ItemStatus::Reserved => "reserved",
            ItemStatus::Donated => "donated",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "available" => Some(ItemStatus::Available),
            "reserved" => Some(ItemStatus::Reserved),
            "donated" => Some(ItemStatus::Donated),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ItemStatus::Available => "Available",
            ItemStatus::Reserved => "Reserved",
            ItemStatus::Donated => "Donated",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceType {
    Donation,
    Purchase,
}

impl SourceType {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceType::Donation => "donation",
            SourceType::Purchase => "purchase",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "donation" => Some(SourceType::Donation),
            "purchase" => Some(SourceType::Purchase),
            _ => None,
        }
    }
}

/// Inventory item joined with its category name and a human-readable source
/// description (donor name / "Anonymous" for donations, channel for purchases),
/// for list display.
pub struct InventoryItemRow {
    pub id: i64,
    pub name: String,
    pub category_id: i64,
    pub category_name: String,
    pub source_type: SourceType,
    pub source_donation_id: Option<i64>,
    pub source_purchase_id: Option<i64>,
    pub source_desc: String,
    /// Date the item was acquired: `physical_donation.date_received` for
    /// donations, `purchase.date` for purchases. `None` only if the source
    /// row itself is missing (shouldn't happen given the FK, but the join
    /// is LEFT so it's not guaranteed at the type level).
    pub acquired_date: Option<String>,
    pub location: Location,
    pub status: ItemStatus,
    pub notes: Option<String>,
}

#[derive(Clone)]
pub struct InventoryItemDraft {
    pub name: String,
    pub category_id: Option<i64>,
    pub source_type: SourceType,
    pub source_donation_id: Option<i64>,
    pub source_purchase_id: Option<i64>,
    pub location: Location,
    pub status: ItemStatus,
    pub notes: String,
}

impl Default for InventoryItemDraft {
    fn default() -> Self {
        Self {
            name: String::new(),
            category_id: None,
            source_type: SourceType::Donation,
            source_donation_id: None,
            source_purchase_id: None,
            location: Location::Germany,
            status: ItemStatus::Available,
            notes: String::new(),
        }
    }
}
