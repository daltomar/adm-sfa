pub struct Document {
    pub id: i64,
    pub filename: String,
    pub record_type: String,
    pub record_id: i64,
    pub label: String,
    pub deleted: bool,
}

pub struct DocumentLabel {
    pub id: i64,
    pub name: String,
}
