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

    #[allow(clippy::should_implement_trait)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurchaseStatus {
    Negotiating,
    Bought,
}

impl PurchaseStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            PurchaseStatus::Negotiating => "negotiating",
            PurchaseStatus::Bought => "bought",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "negotiating" => Some(PurchaseStatus::Negotiating),
            "bought" => Some(PurchaseStatus::Bought),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct Purchase {
    pub id: i64,
    pub date: String,
    pub currency: Currency,
    pub cost: Decimal,
    pub channel: String,
    pub seller_info: Option<String>,
    pub multiple_items: bool,
    pub status: PurchaseStatus,
}

#[derive(Clone)]
pub struct PurchaseDraft {
    pub date: String,
    pub currency: Currency,
    pub cost_str: String,
    pub channel: String,
    pub seller_info: String,
    pub multiple_items: bool,
    pub status: PurchaseStatus,
}

impl Default for PurchaseDraft {
    fn default() -> Self {
        Self {
            date: chrono::Local::now().format("%d.%m.%Y").to_string(),
            currency: Currency::Eur,
            cost_str: String::new(),
            channel: String::new(),
            seller_info: String::new(),
            multiple_items: false,
            status: PurchaseStatus::Bought,
        }
    }
}
