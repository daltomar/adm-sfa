use rust_decimal::Decimal;

pub struct AnnualTransfer {
    pub id: i64,
    pub date: String,
    pub eur_amount_sent: Decimal,
    pub exchange_rate: Decimal,
    pub brl_amount_received: Decimal,
    pub notes: Option<String>,
}
