use rust_decimal::Decimal;

pub struct RecipientProject {
    pub id: i64,
    pub name: String,
    // contact_info/location stored in DB; displayed when Settings is implemented
    #[allow(dead_code)]
    pub contact_info: Option<String>,
    #[allow(dead_code)]
    pub location: Option<String>,
    pub active: bool,
}

#[derive(Clone)]
pub struct RecipientProjectDraft {
    pub name: String,
    pub contact_info: String,
    pub location: String,
    pub active: bool,
}

impl Default for RecipientProjectDraft {
    fn default() -> Self {
        Self {
            name: String::new(),
            contact_info: String::new(),
            location: String::new(),
            active: true,
        }
    }
}

// Bare row type; UI uses OutboundEventRow (with joined fields) instead.
#[allow(dead_code)]
pub struct OutboundEvent {
    pub id: i64,
    pub date: String,
    pub recipient_project_id: i64,
    pub cash_amount_brl: Option<Decimal>,
    pub notes: Option<String>,
}

/// Row returned by the list query — includes the joined recipient name and
/// the count of inventory items linked via outbound_event_item.
pub struct OutboundEventRow {
    pub id: i64,
    pub date: String,
    pub recipient_project_id: i64,
    pub recipient_name: String,
    pub cash_amount_brl: Option<Decimal>,
    pub notes: Option<String>,
    pub item_count: i64,
}

#[derive(Clone)]
pub struct OutboundEventDraft {
    pub date: String,
    pub recipient_project_id: Option<i64>,
    pub cash_amount_brl_str: String,
    pub notes: String,
}

impl Default for OutboundEventDraft {
    fn default() -> Self {
        Self {
            date: chrono::Local::now().format("%Y-%m-%d").to_string(),
            recipient_project_id: None,
            cash_amount_brl_str: String::new(),
            notes: String::new(),
        }
    }
}

// Mirrors the outbound_event_item join table; not directly constructed in UI code.
#[allow(dead_code)]
pub struct OutboundEventItem {
    pub outbound_event_id: i64,
    pub inventory_item_id: i64,
}
