use eframe::egui;
use rusqlite::Connection;
use rust_decimal::Decimal;

use crate::db::queries::{donors as donors_qry, eur_ledger as qry};
use crate::model::transaction::{EurTxDraft, EurTxRow, EurTxType, ManualEurTxType};

enum Mode {
    List,
    Adding,
    Editing(i64),
    ViewingLinked(i64),
}

pub struct EurLedgerView {
    rows: Vec<EurTxRow>,
    balance: Decimal,
    mode: Mode,
    draft: EurTxDraft,
    error: Option<String>,
    needs_reload: bool,
    donors: Vec<(i64, String)>,
    donors_loaded: bool,
}

impl Default for EurLedgerView {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            balance: Decimal::ZERO,
            mode: Mode::List,
            draft: EurTxDraft::default(),
            error: None,
            needs_reload: true,
            donors: Vec::new(),
            donors_loaded: false,
        }
    }
}

impl EurLedgerView {
    pub fn show(&mut self, ui: &mut egui::Ui, db: &Connection) {
        if self.needs_reload {
            match qry::list(db) {
                Ok(rows) => {
                    self.balance = compute_balance(&rows);
                    self.rows = rows;
                    self.needs_reload = false;
                    self.donors_loaded = false;
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }

        if !self.donors_loaded {
            match donors_qry::list(db) {
                Ok(list) => {
                    self.donors = list.into_iter().map(|d| (d.id, d.name)).collect();
                    self.donors_loaded = true;
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }

        egui::Panel::left("eur_ledger_list_panel")
            .resizable(true)
            .default_size(340.0)
            .show(ui, |ui| self.show_list(ui));

        egui::ScrollArea::vertical()
            .id_salt("eur_ledger_detail_scroll")
            .show(ui, |ui| match self.mode {
                Mode::List => {
                    ui.add_space(16.0);
                    ui.weak("Add an entry, or select a manual entry to edit.");
                }
                Mode::Adding | Mode::Editing(_) => self.show_form(ui, db),
                Mode::ViewingLinked(id) => self.show_linked_info(ui, id),
            });
    }

    fn show_list(&mut self, ui: &mut egui::Ui) {
        ui.heading("EUR Ledger");
        ui.add_space(4.0);

        let bal_color = if self.balance >= Decimal::ZERO {
            egui::Color32::from_rgb(80, 190, 80)
        } else {
            egui::Color32::from_rgb(220, 60, 60)
        };
        ui.label(
            egui::RichText::new(format!("Balance: € {:.2}", self.balance))
                .strong()
                .color(bal_color),
        );
        ui.add_space(4.0);

        if ui.button("+ Add entry").clicked() {
            self.draft = EurTxDraft::default();
            self.mode = Mode::Adding;
            self.error = None;
            self.donors_loaded = false;
        }

        ui.separator();

        if let Some(err) = &self.error {
            ui.colored_label(egui::Color32::RED, err);
            ui.separator();
        }

        egui::ScrollArea::vertical()
            .id_salt("eur_ledger_list_scroll")
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
                        format!("{}  {}  {}€{:.2}", date, tx_type.label(), sign, amount)
                    } else {
                        format!(
                            "{}  {}  {}€{:.2}  {}",
                            date,
                            tx_type.label(),
                            sign,
                            amount,
                            desc
                        )
                    };

                    let selected = matches!(self.mode, Mode::Editing(eid) if eid == id)
                        || matches!(self.mode, Mode::ViewingLinked(vid) if vid == id);

                    if ui.selectable_label(selected, &row_label).clicked() {
                        if tx_type.is_manual() {
                            let date_c = self.rows[i].date.clone();
                            let amount_str = self.rows[i].amount.to_string();
                            let donor_id = self.rows[i].donor_id;
                            let note = self.rows[i].note.clone().unwrap_or_default();
                            let manual_type = if tx_type == EurTxType::DonationIn {
                                ManualEurTxType::DonationIn
                            } else {
                                ManualEurTxType::SelfFundingIn
                            };
                            self.draft = EurTxDraft {
                                date: date_c,
                                tx_type: manual_type,
                                amount_str,
                                donor_id,
                                note,
                            };
                            self.mode = Mode::Editing(id);
                            self.error = None;
                        } else {
                            self.mode = Mode::ViewingLinked(id);
                        }
                    }
                }
            });
    }

    fn show_form(&mut self, ui: &mut egui::Ui, db: &Connection) {
        let is_adding = matches!(self.mode, Mode::Adding);
        let edit_id: Option<i64> = if let Mode::Editing(id) = self.mode {
            Some(id)
        } else {
            None
        };

        ui.heading(if is_adding { "New Entry" } else { "Edit Entry" });
        ui.add_space(8.0);

        // Type selector — shown only when adding; read-only label when editing.
        if is_adding {
            ui.horizontal(|ui| {
                ui.label("Type:");
                ui.radio_value(
                    &mut self.draft.tx_type,
                    ManualEurTxType::DonationIn,
                    "Donation",
                );
                ui.radio_value(
                    &mut self.draft.tx_type,
                    ManualEurTxType::SelfFundingIn,
                    "Self-funding",
                );
            });
            ui.add_space(4.0);
        } else {
            ui.horizontal(|ui| {
                ui.label("Type:");
                ui.label(egui::RichText::new(self.draft.tx_type.as_eur_tx_type().label()).strong());
            });
            ui.add_space(4.0);
        }

        // Clone donors before entering closures to avoid split-borrow on self.
        let donors = self.donors.clone();
        let show_donor = self.draft.tx_type == ManualEurTxType::DonationIn;

        egui::Grid::new("eur_tx_form_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .min_col_width(80.0)
            .show(ui, |ui| {
                ui.label("Date *");
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.date)
                        .hint_text("YYYY-MM-DD")
                        .desired_width(140.0),
                );
                ui.end_row();

                ui.label("Amount (€) *");
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.amount_str)
                        .hint_text("0.00")
                        .desired_width(140.0),
                );
                ui.end_row();

                if show_donor {
                    ui.label("Donor");
                    let selected_name = self
                        .draft
                        .donor_id
                        .and_then(|id| donors.iter().find(|(did, _)| *did == id))
                        .map(|(_, n)| n.clone())
                        .unwrap_or_else(|| "(none)".to_string());
                    egui::ComboBox::from_id_salt("eur_donor_combo")
                        .selected_text(&selected_name)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.draft.donor_id, None, "(none)");
                            for (did, name) in &donors {
                                ui.selectable_value(&mut self.draft.donor_id, Some(*did), name);
                            }
                        });
                    ui.end_row();
                }

                ui.label("Note");
                ui.add(
                    egui::TextEdit::multiline(&mut self.draft.note)
                        .desired_width(280.0)
                        .desired_rows(3),
                );
                ui.end_row();
            });

        if let Some(err) = &self.error {
            ui.add_space(4.0);
            ui.colored_label(egui::Color32::RED, err);
        }

        let amount_ok = self
            .draft
            .amount_str
            .trim()
            .parse::<Decimal>()
            .map(|d| d > Decimal::ZERO)
            .unwrap_or(false);
        let form_ok = !self.draft.date.trim().is_empty() && amount_ok;

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(form_ok, egui::Button::new("Save")).clicked() {
                if is_adding {
                    match qry::insert(db, &self.draft) {
                        Ok(_) => {
                            self.mode = Mode::List;
                            self.needs_reload = true;
                            self.error = None;
                        }
                        Err(e) => self.error = Some(e.to_string()),
                    }
                } else if let Some(id) = edit_id {
                    match qry::update(db, id, &self.draft) {
                        Ok(()) => {
                            self.needs_reload = true;
                            self.error = None;
                        }
                        Err(e) => self.error = Some(e.to_string()),
                    }
                }
            }

            if ui.button("Cancel").clicked() {
                self.mode = Mode::List;
                self.error = None;
            }
        });
    }

    fn show_linked_info(&self, ui: &mut egui::Ui, id: i64) {
        let Some(row) = self.rows.iter().find(|r| r.id == id) else {
            return;
        };
        match row.tx_type {
            EurTxType::PurchaseOut => {
                ui.heading("Purchase (read-only)");
                ui.add_space(8.0);
                ui.label(format!("Date: {}", row.date));
                ui.label(format!("Amount: € {:.2}", row.amount));
                if let Some(ch) = &row.purchase_channel {
                    ui.label(format!("Channel: {ch}"));
                }
                if let Some(n) = &row.note {
                    ui.label(format!("Note: {n}"));
                }
                ui.add_space(8.0);
                ui.weak("Created automatically by the Purchases section. Edit or delete the purchase there.");
            }
            EurTxType::TransferToBrlOut => {
                ui.heading("EUR → BRL Transfer (read-only)");
                ui.add_space(8.0);
                ui.label(format!("Date: {}", row.date));
                ui.label(format!("Amount sent: € {:.2}", row.amount));
                if let Some(n) = &row.note {
                    ui.label(format!("Note: {n}"));
                }
                ui.add_space(8.0);
                ui.weak("Created automatically by the Transfers section.");
            }
            _ => {}
        }
    }
}

fn compute_balance(rows: &[EurTxRow]) -> Decimal {
    rows.iter().fold(Decimal::ZERO, |acc, r| {
        if r.tx_type.is_inflow() {
            acc + r.amount
        } else {
            acc - r.amount
        }
    })
}

fn row_desc(row: &EurTxRow) -> String {
    match row.tx_type {
        EurTxType::DonationIn => row.donor_name.clone().unwrap_or_default(),
        EurTxType::SelfFundingIn => row.note.clone().unwrap_or_default(),
        EurTxType::PurchaseOut => row.purchase_channel.clone().unwrap_or_default(),
        EurTxType::TransferToBrlOut => row.note.clone().unwrap_or_else(|| "EUR→BRL".to_string()),
    }
}
