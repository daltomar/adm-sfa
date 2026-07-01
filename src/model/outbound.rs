use rust_decimal::Decimal;

pub struct RecipientProject {
    pub id: i64,
    pub name: String,
    pub contact_info: Option<String>,
    pub location: Option<String>,
    pub active: bool,
}

pub struct OutboundEvent {
    pub id: i64,
    pub date: String,
    pub recipient_project_id: i64,
    pub cash_amount_brl: Option<Decimal>,
    pub notes: Option<String>,
}

pub struct OutboundEventItem {
    pub outbound_event_id: i64,
    pub inventory_item_id: i64,
}
