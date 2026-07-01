use rust_decimal::Decimal;

pub enum Currency {
    Eur,
    Brl,
}

pub struct Purchase {
    pub id: i64,
    pub date: String,
    pub currency: Currency,
    pub cost: Decimal,
    pub channel: String,
    pub seller_info: Option<String>,
}
