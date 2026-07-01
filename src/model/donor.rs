pub struct Donor {
    pub id: i64,
    pub name: String,
    pub contact_info: Option<String>,
    pub notes: Option<String>,
}

pub struct PhysicalDonation {
    pub id: i64,
    pub donor_id: Option<i64>,
    pub date_received: String,
    pub notes: Option<String>,
}

#[derive(Default, Clone)]
pub struct DonorDraft {
    pub name: String,
    pub contact_info: String,
    pub notes: String,
}
