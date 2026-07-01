use rust_decimal::Decimal;

pub enum EurTxType {
    DonationIn,
    SelfFundingIn,
    PurchaseOut,
    TransferToBrlOut,
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
