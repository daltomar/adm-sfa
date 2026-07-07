use eframe::egui;
use rusqlite::Connection;
use rust_decimal::Decimal;

use crate::db::queries::brl_ledger as qry;
use crate::model::transaction::{BrlTxRow, BrlTxType};

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
                    ui.weak("BRL entries are created automatically by the Purchases, Transfers, and Outbound sections.");
                }
                Mode::Viewing(id) => self.show_detail(ui, id),
            });
    }

    fn show_list(&mut self, ui: &mut egui::Ui) {
        ui.heading("BRL Ledger");
        ui.add_space(4.0);

        let bal_color = if self.balance >= Decimal::ZERO {
            egui::Color32::from_rgb(80, 190, 80)
        } else {
            egui::Color32::from_rgb(220, 60, 60)
        };
        ui.label(
            egui::RichText::new(format!("Balance: R$ {:.2}", self.balance))
                .strong()
                .color(bal_color),
        );
        ui.add_space(4.0);

        ui.weak("Read-only — entries are created by other sections.");
        ui.separator();

        if let Some(err) = &self.error {
            ui.colored_label(egui::Color32::RED, err);
            ui.separator();
        }

        egui::ScrollArea::vertical()
            .id_salt("brl_ledger_list_scroll")
            .show(ui, |ui| {
                if self.rows.is_empty() {
                    ui.weak("No entries yet.");
                    return;
                }
                for i in 0..self.rows.len() {
                    let id = self.rows[i].id;
                    let tx_type = self.rows[i].tx_type;
                    let sign = if tx_type.is_inflow() { "+" } else { "-" };
                    let amount = self.rows[i].amount;
                    let date = self.rows[i].date.clone();
                    let desc = row_desc(&self.rows[i]);

                    let row_label = if desc.is_empty() {
                        format!("{}  {}  {}R${:.2}", date, tx_type.label(), sign, amount)
                    } else {
                        format!(
                            "{}  {}  {}R${:.2}  {}",
                            date,
                            tx_type.label(),
                            sign,
                            amount,
                            desc
                        )
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
                ui.heading("EUR → BRL Transfer");
                ui.add_space(8.0);
                ui.label(format!("Date: {}", row.date));
                ui.label(format!("Amount received: R$ {:.2}", row.amount));
                if let Some(n) = &row.note {
                    if !n.is_empty() {
                        ui.label(format!("Note: {n}"));
                    }
                }
                ui.add_space(8.0);
                ui.weak("Created by the Transfers section.");
            }
            BrlTxType::BrazilPurchaseOut => {
                ui.heading("Purchase (BRL)");
                ui.add_space(8.0);
                ui.label(format!("Date: {}", row.date));
                ui.label(format!("Amount: R$ {:.2}", row.amount));
                if let Some(ch) = &row.purchase_channel {
                    ui.label(format!("Channel: {ch}"));
                }
                if let Some(n) = &row.note {
                    if !n.is_empty() {
                        ui.label(format!("Note: {n}"));
                    }
                }
                ui.add_space(8.0);
                ui.weak("Created by the Purchases section.");
            }
            BrlTxType::CashGiftOut => {
                ui.heading("Cash Gift");
                ui.add_space(8.0);
                ui.label(format!("Date: {}", row.date));
                ui.label(format!("Amount: R$ {:.2}", row.amount));
                if let Some(rp) = &row.recipient_name {
                    ui.label(format!("Recipient: {rp}"));
                }
                if let Some(n) = &row.note {
                    if !n.is_empty() {
                        ui.label(format!("Note: {n}"));
                    }
                }
                ui.add_space(8.0);
                ui.weak("Created by the Outbound section.");
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
