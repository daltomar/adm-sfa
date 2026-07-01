use rust_decimal::Decimal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Currency {
    Eur,
    Brl,
}

impl Currency {
    pub fn as_str(self) -> &'static str {
        match self {
            Currency::Eur => "EUR",
            Currency::Brl => "BRL",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "EUR" => Some(Currency::Eur),
            "BRL" => Some(Currency::Brl),
            _ => None,
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Currency::Eur => "€",
            Currency::Brl => "R$",
        }
    }
}

pub struct Purchase {
    pub id: i64,
    pub date: String,
    pub currency: Currency,
    pub cost: Decimal,
    pub channel: String,
    pub seller_info: Option<String>,
}

#[derive(Clone)]
pub struct PurchaseDraft {
    pub date: String,
    pub currency: Currency,
    pub cost_str: String,
    pub channel: String,
    pub seller_info: String,
}

impl Default for PurchaseDraft {
    fn default() -> Self {
        Self {
            date: chrono::Local::now().format("%Y-%m-%d").to_string(),
            currency: Currency::Eur,
            cost_str: String::new(),
            channel: String::new(),
            seller_info: String::new(),
        }
    }
}
