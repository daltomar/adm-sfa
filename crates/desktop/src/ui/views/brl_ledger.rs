use eframe::egui;
use rusqlite::Connection;
use rust_decimal::Decimal;
use rust_i18n::t;

use adm_sfa_core::db::queries::brl_ledger as qry;
use adm_sfa_core::format;
use adm_sfa_core::model::transaction::{BrlTxRow, BrlTxType};

enum Mode {
    List,
    Viewing(i64),
}

pub struct BrlLedgerView {
    rows: Vec<BrlTxRow>,
    balance: Decimal,
    mode: Mode,
    error: Option<String>,
    needs_reload: bool,
}

impl Default for BrlLedgerView {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            balance: Decimal::ZERO,
            mode: Mode::List,
            error: None,
            needs_reload: true,
        }
    }
}

impl BrlLedgerView {
    pub fn invalidate(&mut self) {
        self.needs_reload = true;
    }

    pub fn show(&mut self, ui: &mut egui::Ui, db: &Connection) {
        if self.needs_reload {
            match qry::list(db) {
                Ok(rows) => {
                    self.balance = compute_balance(&rows);
                    self.rows = rows;
                    self.needs_reload = false;
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }

        egui::Panel::left("brl_ledger_list_panel")
            .resizable(true)
            .default_size(340.0)
            .show(ui, |ui| self.show_list(ui));

        egui::ScrollArea::vertical()
            .id_salt("brl_ledger_detail_scroll")
            .show(ui, |ui| match self.mode {
                Mode::List => {
                    ui.add_space(16.0);
                    ui.weak(t!("brl_ledger.hint.auto_created").as_ref());
                }
                Mode::Viewing(id) => self.show_detail(ui, id),
            });
    }

    fn show_list(&mut self, ui: &mut egui::Ui) {
        ui.heading(t!("brl_ledger.heading").as_ref());
        ui.add_space(4.0);

        let bal_color = if self.balance >= Decimal::ZERO {
            egui::Color32::from_rgb(80, 190, 80)
        } else {
            egui::Color32::from_rgb(220, 60, 60)
        };
        ui.label(
            egui::RichText::new(
                t!(
                    "common.balance",
                    symbol = "R$",
                    amount = format::amount(self.balance)
                )
                .into_owned(),
            )
            .strong()
            .color(bal_color),
        );
        ui.add_space(4.0);

        ui.weak(t!("brl_ledger.hint.read_only").as_ref());
        ui.separator();

        if let Some(err) = &self.error {
            ui.colored_label(egui::Color32::RED, err);
            ui.separator();
        }

        egui::ScrollArea::vertical()
            .id_salt("brl_ledger_list_scroll")
            .show(ui, |ui| {
                if self.rows.is_empty() {
                    ui.weak(t!("common.no_entries").as_ref());
                    return;
                }
                for i in 0..self.rows.len() {
                    let id = self.rows[i].id;
                    let tx_type = self.rows[i].tx_type;
                    let sign = if tx_type.is_inflow() { "+" } else { "-" };
                    let amount = self.rows[i].amount;
                    let date = format::date(&self.rows[i].date);
                    let desc = row_desc(&self.rows[i]);

                    let row_label = if desc.is_empty() {
                        t!(
                            "common.ledger_row.no_desc",
                            date = date,
                            kind = tx_type.label(),
                            sign = sign,
                            symbol = "R$",
                            amount = format::amount(amount)
                        )
                        .into_owned()
                    } else {
                        t!(
                            "common.ledger_row.with_desc",
                            date = date,
                            kind = tx_type.label(),
                            sign = sign,
                            symbol = "R$",
                            amount = format::amount(amount),
                            desc = desc
                        )
                        .into_owned()
                    };

                    let selected = matches!(self.mode, Mode::Viewing(vid) if vid == id);
                    if ui.selectable_label(selected, &row_label).clicked() {
                        self.mode = Mode::Viewing(id);
                    }
                }
            });
    }

    fn show_detail(&self, ui: &mut egui::Ui, id: i64) {
        let Some(row) = self.rows.iter().find(|r| r.id == id) else {
            return;
        };

        match row.tx_type {
            BrlTxType::TransferIn => {
                ui.heading(t!("brl_ledger.detail.transfer_in.heading").as_ref());
                ui.add_space(8.0);
                ui.label(t!("common.detail.date", date = format::date(&row.date)).into_owned());
                ui.label(
                    t!(
                        "brl_ledger.detail.amount_received",
                        amount = format::amount(row.amount)
                    )
                    .into_owned(),
                );
                if let Some(n) = &row.note {
                    if !n.is_empty() {
                        ui.label(t!("common.detail.note", note = n).into_owned());
                    }
                }
                ui.add_space(8.0);
                ui.weak(
                    t!(
                        "brl_ledger.detail.created_by",
                        section = t!("sidebar.transfers")
                    )
                    .into_owned(),
                );
            }
            BrlTxType::BrazilPurchaseOut => {
                ui.heading(t!("brl_ledger.detail.purchase.heading").as_ref());
                ui.add_space(8.0);
                ui.label(t!("common.detail.date", date = format::date(&row.date)).into_owned());
                ui.label(
                    t!(
                        "common.detail.amount",
                        symbol = "R$",
                        amount = format::amount(row.amount)
                    )
                    .into_owned(),
                );
                if let Some(ch) = &row.purchase_channel {
                    ui.label(t!("common.detail.channel", channel = ch).into_owned());
                }
                if let Some(n) = &row.note {
                    if !n.is_empty() {
                        ui.label(t!("common.detail.note", note = n).into_owned());
                    }
                }
                ui.add_space(8.0);
                ui.weak(
                    t!(
                        "brl_ledger.detail.created_by",
                        section = t!("sidebar.purchases")
                    )
                    .into_owned(),
                );
            }
            BrlTxType::CashGiftOut => {
                ui.heading(t!("brl_ledger.detail.cash_gift.heading").as_ref());
                ui.add_space(8.0);
                ui.label(t!("common.detail.date", date = format::date(&row.date)).into_owned());
                ui.label(
                    t!(
                        "common.detail.amount",
                        symbol = "R$",
                        amount = format::amount(row.amount)
                    )
                    .into_owned(),
                );
                if let Some(rp) = &row.recipient_name {
                    ui.label(t!("brl_ledger.detail.recipient", recipient = rp).into_owned());
                }
                if let Some(n) = &row.note {
                    if !n.is_empty() {
                        ui.label(t!("common.detail.note", note = n).into_owned());
                    }
                }
                ui.add_space(8.0);
                ui.weak(
                    t!(
                        "brl_ledger.detail.created_by",
                        section = t!("sidebar.outbound")
                    )
                    .into_owned(),
                );
            }
        }
    }
}

fn compute_balance(rows: &[BrlTxRow]) -> Decimal {
    rows.iter().fold(Decimal::ZERO, |acc, r| {
        if r.tx_type.is_inflow() {
            acc + r.amount
        } else {
            acc - r.amount
        }
    })
}

fn row_desc(row: &BrlTxRow) -> String {
    match row.tx_type {
        BrlTxType::TransferIn => String::new(),
        BrlTxType::BrazilPurchaseOut => row.purchase_channel.clone().unwrap_or_default(),
        BrlTxType::CashGiftOut => row.recipient_name.clone().unwrap_or_default(),
    }
}
