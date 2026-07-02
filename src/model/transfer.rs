use rust_decimal::Decimal;

pub struct AnnualTransfer {
    pub id: i64,
    pub date: String,
    pub eur_amount_sent: Decimal,
    pub exchange_rate: Decimal,
    pub brl_amount_received: Decimal,
    pub notes: Option<String>,
}

#[derive(Clone)]
pub struct TransferDraft {
    pub date: String,
    pub eur_amount_sent_str: String,
    pub exchange_rate_str: String,
    pub notes: String,
}

impl Default for TransferDraft {
    fn default() -> Self {
        Self {
            date: chrono::Local::now().format("%Y-%m-%d").to_string(),
            eur_amount_sent_str: String::new(),
            exchange_rate_str: String::new(),
            notes: String::new(),
        }
    }
}
