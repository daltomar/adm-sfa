use rust_decimal::Decimal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EurTxType {
    DonationIn,
    SelfFundingIn,
    PurchaseOut,
    TransferToBrlOut,
}

impl EurTxType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "donation_in" => Some(Self::DonationIn),
            "self_funding_in" => Some(Self::SelfFundingIn),
            "purchase_out" => Some(Self::PurchaseOut),
            "transfer_to_brl_out" => Some(Self::TransferToBrlOut),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::DonationIn => "donation_in",
            Self::SelfFundingIn => "self_funding_in",
            Self::PurchaseOut => "purchase_out",
            Self::TransferToBrlOut => "transfer_to_brl_out",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::DonationIn => "Donation",
            Self::SelfFundingIn => "Self-funding",
            Self::PurchaseOut => "Purchase",
            Self::TransferToBrlOut => "→ BRL",
        }
    }

    pub fn is_inflow(self) -> bool {
        matches!(self, Self::DonationIn | Self::SelfFundingIn)
    }

    pub fn is_manual(self) -> bool {
        matches!(self, Self::DonationIn | Self::SelfFundingIn)
    }
}

// Bare row type; UI uses EurTxRow (with joined fields) instead.
#[allow(dead_code)]
pub struct EurTransaction {
    pub id: i64,
    pub date: String,
    pub tx_type: EurTxType,
    pub amount: Decimal,
    pub donor_id: Option<i64>,
    pub note: Option<String>,
    pub linked_purchase_id: Option<i64>,
    pub linked_transfer_id: Option<i64>,
}

/// Row returned by the ledger list query — includes joined donor name and purchase channel.
pub struct EurTxRow {
    pub id: i64,
    pub date: String,
    pub tx_type: EurTxType,
    pub amount: Decimal,
    pub donor_id: Option<i64>,
    pub donor_name: Option<String>,
    pub purchase_channel: Option<String>,
    pub note: Option<String>,
    pub linked_purchase_id: Option<i64>,
    pub linked_transfer_id: Option<i64>,
}

/// Types the user can enter manually; purchase_out and transfer_to_brl_out are auto-created.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum ManualEurTxType {
    #[default]
    DonationIn,
    SelfFundingIn,
}

impl ManualEurTxType {
    pub fn as_eur_tx_type(self) -> EurTxType {
        match self {
            Self::DonationIn => EurTxType::DonationIn,
            Self::SelfFundingIn => EurTxType::SelfFundingIn,
        }
    }
}

#[derive(Clone)]
pub struct EurTxDraft {
    pub date: String,
    pub tx_type: ManualEurTxType,
    pub amount_str: String,
    pub donor_id: Option<i64>,
    pub note: String,
}

impl Default for EurTxDraft {
    fn default() -> Self {
        Self {
            date: chrono::Local::now().format("%Y-%m-%d").to_string(),
            tx_type: ManualEurTxType::default(),
            amount_str: String::new(),
            donor_id: None,
            note: String::new(),
        }
    }
}

// ── BRL side ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrlTxType {
    TransferIn,
    BrazilPurchaseOut,
    CashGiftOut,
}

impl BrlTxType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "transfer_in" => Some(Self::TransferIn),
            "brazil_purchase_out" => Some(Self::BrazilPurchaseOut),
            "cash_gift_out" => Some(Self::CashGiftOut),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::TransferIn => "EUR→BRL",
            Self::BrazilPurchaseOut => "Purchase",
            Self::CashGiftOut => "Cash gift",
        }
    }

    pub fn is_inflow(self) -> bool {
        matches!(self, Self::TransferIn)
    }
}

// Bare row type; UI uses BrlTxRow (with joined fields) instead.
#[allow(dead_code)]
pub struct BrlTransaction {
    pub id: i64,
    pub date: String,
    pub tx_type: BrlTxType,
    pub amount: Decimal,
    pub linked_transfer_id: Option<i64>,
    pub linked_purchase_id: Option<i64>,
    pub linked_outbound_event_id: Option<i64>,
    pub note: Option<String>,
}

/// Row returned by the BRL ledger list query — includes joined description fields.
pub struct BrlTxRow {
    pub id: i64,
    pub date: String,
    pub tx_type: BrlTxType,
    pub amount: Decimal,
    pub note: Option<String>,
    pub linked_transfer_id: Option<i64>,
    pub linked_purchase_id: Option<i64>,
    pub linked_outbound_event_id: Option<i64>,
    /// Populated for brazil_purchase_out via JOIN with purchase.
    pub purchase_channel: Option<String>,
    /// Populated for cash_gift_out via JOIN with outbound_event → recipient_project.
    pub recipient_name: Option<String>,
}
