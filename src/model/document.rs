#[derive(Clone)]
pub struct Document {
    pub id: i64,
    pub filename: String,
    // record_type/record_id/deleted are read by db layer, not yet by UI
    #[allow(dead_code)]
    pub record_type: String,
    #[allow(dead_code)]
    pub record_id: i64,
    pub label: String,
    #[allow(dead_code)]
    pub deleted: bool,
}

// Used by the Settings view (not yet implemented).
#[allow(dead_code)]
pub struct DocumentLabel {
    pub id: i64,
    pub name: String,
}
