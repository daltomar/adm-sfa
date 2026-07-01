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
