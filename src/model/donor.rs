pub struct Donor {
    pub id: i64,
    pub name: String,
    pub contact_info: Option<String>,
    pub notes: Option<String>,
}

pub struct PhysicalDonation {
    pub id: i64,
    pub donor_id: Option<i64>,
    pub donor_name: Option<String>,
    pub date_received: String,
    #[allow(dead_code)]
    pub notes: Option<String>,
}

#[derive(Default, Clone)]
pub struct DonorDraft {
    pub name: String,
    pub contact_info: String,
    pub notes: String,
}

#[derive(Clone)]
pub struct PhysicalDonationDraft {
    pub donor_id: Option<i64>,
    pub date_received: String,
    pub notes: String,
}

impl Default for PhysicalDonationDraft {
    fn default() -> Self {
        Self {
            donor_id: None,
            date_received: chrono::Local::now().format("%Y-%m-%d").to_string(),
            notes: String::new(),
        }
    }
}
