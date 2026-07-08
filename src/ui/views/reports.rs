use eframe::egui;
use egui_extras::{Column, TableBuilder};
use rusqlite::Connection;
use rust_decimal::Decimal;
use std::collections::{BTreeMap, HashMap};

use crate::db::queries::{
    brl_ledger as brl_qry, documents as documents_qry, donors as donors_qry, eur_ledger as eur_qry,
    inventory as inventory_qry, outbound as outbound_qry,
};
use crate::model::donor::{Donor, PhysicalDonation};
use crate::model::inventory::{InventoryItemRow, SourceType};
use crate::model::outbound::{OutboundEventRow, RecipientProject};
use crate::model::transaction::{BrlTxRow, BrlTxType, EurTxRow, EurTxType};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Donors,
    Eur,
    Brl,
    Inventory,
    Outbound,
    AuditTrail,
}

const TABS: &[(Tab, &str)] = &[
    (Tab::Donors, "Donor breakdown"),
    (Tab::Eur, "EUR ledger summary"),
    (Tab::Brl, "BRL ledger summary"),
    (Tab::Inventory, "Inventory summary"),
    (Tab::Outbound, "Outbound summary"),
    (Tab::AuditTrail, "Audit trail"),
];

pub struct ReportsView {
    tab: Tab,
    date_from: String,
    date_to: String,
    recipient_filter: Option<i64>,

    loaded: bool,
    last_refreshed: Option<String>,
    error: Option<String>,
    export_status: Option<Result<String, String>>,

    donors: Vec<Donor>,
    donations: Vec<PhysicalDonation>,
    eur_rows: Vec<EurTxRow>,
    brl_rows: Vec<BrlTxRow>,
    inventory_rows: Vec<InventoryItemRow>,
    outbound_rows: Vec<OutboundEventRow>,
    recipient_projects: Vec<RecipientProject>,
    doc_counts: HashMap<(String, i64), i64>,
}

impl Default for ReportsView {
    fn default() -> Self {
        Self {
            tab: Tab::Donors,
            date_from: String::new(),
            date_to: String::new(),
            recipient_filter: None,
            loaded: false,
            last_refreshed: None,
            error: None,
            export_status: None,
            donors: Vec::new(),
            donations: Vec::new(),
            eur_rows: Vec::new(),
            brl_rows: Vec::new(),
            inventory_rows: Vec::new(),
            outbound_rows: Vec::new(),
            recipient_projects: Vec::new(),
            doc_counts: HashMap::new(),
        }
    }
}

struct DonorRow {
    name: String,
    cash_count: i64,
    cash_total: Decimal,
    item_count: i64,
}

struct AuditEntry {
    date: String,
    ledger: &'static str,
    kind: &'static str,
    description: String,
    amount: String,
    docs: i64,
}

impl ReportsView {
    pub fn show(&mut self, ui: &mut egui::Ui, db: &Connection) {
        if !self.loaded {
            match self.reload(db) {
                Ok(()) => {
                    self.loaded = true;
                    self.error = None;
                    self.last_refreshed = Some(chrono::Local::now().format("%H:%M:%S").to_string());
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }

        ui.heading("Reports");
        ui.add_space(4.0);

        if let Some(err) = &self.error {
            ui.colored_label(egui::Color32::RED, err);
            ui.separator();
        }

        self.show_filters(ui);
        ui.add_space(8.0);
        self.show_tabs(ui);
        ui.separator();
        ui.add_space(4.0);

        egui::ScrollArea::vertical()
            .id_salt("reports_scroll")
            .show(ui, |ui| match self.tab {
                Tab::Donors => self.show_donor_breakdown(ui),
                Tab::Eur => self.show_eur_summary(ui),
                Tab::Brl => self.show_brl_summary(ui),
                Tab::Inventory => self.show_inventory_summary(ui),
                Tab::Outbound => self.show_outbound_summary(ui),
                Tab::AuditTrail => self.show_audit_trail(ui),
            });
    }

    fn reload(&mut self, db: &Connection) -> rusqlite::Result<()> {
        self.donors = donors_qry::list(db)?;
        self.donations = donors_qry::list_donations(db)?;
        self.eur_rows = eur_qry::list(db)?;
        self.brl_rows = brl_qry::list(db)?;
        self.inventory_rows = inventory_qry::list(db)?;
        self.outbound_rows = outbound_qry::list(db)?;
        self.recipient_projects = outbound_qry::list_recipient_projects(db)?;
        self.doc_counts = documents_qry::counts_by_record(db)?;
        Ok(())
    }

    fn show_filters(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("From");
            ui.add(
                egui::TextEdit::singleline(&mut self.date_from)
                    .hint_text("YYYY-MM-DD")
                    .desired_width(110.0),
            );
            ui.label("To");
            ui.add(
                egui::TextEdit::singleline(&mut self.date_to)
                    .hint_text("YYYY-MM-DD")
                    .desired_width(110.0),
            );

            ui.add_space(12.0);
            ui.label("Recipient project");
            let selected_label = self
                .recipient_filter
                .and_then(|id| self.recipient_projects.iter().find(|p| p.id == id))
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "All".to_string());
            egui::ComboBox::from_id_salt("reports_recipient_filter")
                .selected_text(selected_label)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.recipient_filter, None, "All");
                    for p in &self.recipient_projects {
                        ui.selectable_value(&mut self.recipient_filter, Some(p.id), &p.name);
                    }
                });

            ui.add_space(12.0);
            if ui.button("Clear filters").clicked() {
                self.date_from.clear();
                self.date_to.clear();
                self.recipient_filter = None;
            }
            if ui.button("⟳ Refresh").clicked() {
                self.loaded = false;
                ui.ctx().request_repaint();
            }
            if let Some(ts) = &self.last_refreshed {
                ui.weak(format!("↑ {ts}"));
            }
            ui.add_space(8.0);
            if ui.button("Export CSV").clicked() {
                let tab_label = TABS
                    .iter()
                    .find(|(t, _)| *t == self.tab)
                    .map(|(_, l)| *l)
                    .unwrap_or("report");
                let filename = format!("{}.csv", tab_label.to_lowercase().replace(' ', "-"));
                let (headers, rows) = match self.tab {
                    Tab::Donors => self.csv_data_donors(),
                    Tab::Eur => self.csv_data_eur(),
                    Tab::Brl => self.csv_data_brl(),
                    Tab::Inventory => self.csv_data_inventory(),
                    Tab::Outbound => self.csv_data_outbound(),
                    Tab::AuditTrail => self.csv_data_audit_trail(),
                };
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name(&filename)
                    .add_filter("CSV", &["csv"])
                    .save_file()
                {
                    self.export_status = Some(
                        crate::reports::csv::write(&path, &headers, &rows)
                            .map(|()| format!("Exported to {}", path.display()))
                            .map_err(|e| e.to_string()),
                    );
                }
            }
        });
        if let Some(status) = &self.export_status {
            match status {
                Ok(msg) => ui.colored_label(egui::Color32::DARK_GREEN, msg),
                Err(msg) => ui.colored_label(egui::Color32::RED, format!("Export failed: {msg}")),
            };
        }
        ui.weak("Date range applies to all tabs except Inventory. Recipient project filters Outbound and Audit trail.");
    }

    fn show_tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for &(tab, label) in TABS {
                if ui.selectable_label(self.tab == tab, label).clicked() {
                    self.tab = tab;
                }
            }
        });
    }

    fn show_donor_breakdown(&self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Contributions per donor").strong());
        ui.add_space(6.0);

        let rows = self.build_donor_rows();

        if rows.is_empty() {
            ui.weak("No donor activity in the selected date range.");
            return;
        }

        TableBuilder::new(ui)
            .id_salt("donor_breakdown_table")
            .striped(true)
            .column(Column::auto().at_least(160.0))
            .column(Column::auto().at_least(110.0))
            .column(Column::auto().at_least(110.0))
            .column(Column::remainder().at_least(130.0))
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.strong("Donor");
                });
                header.col(|ui| {
                    ui.strong("Cash donations");
                });
                header.col(|ui| {
                    ui.strong("Cash total (€)");
                });
                header.col(|ui| {
                    ui.strong("Physical donations");
                });
            })
            .body(|mut body| {
                for r in &rows {
                    body.row(22.0, |mut row| {
                        row.col(|ui| {
                            ui.label(&r.name);
                        });
                        row.col(|ui| {
                            ui.label(r.cash_count.to_string());
                        });
                        row.col(|ui| {
                            ui.label(format!("{:.2}", r.cash_total));
                        });
                        row.col(|ui| {
                            ui.label(r.item_count.to_string());
                        });
                    });
                }
            });
    }

    fn show_eur_summary(&self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("EUR ledger summary").strong());
        ui.add_space(6.0);

        let starting_balance: Decimal = self
            .eur_rows
            .iter()
            .filter(|r| !self.date_from.is_empty() && r.date.as_str() < self.date_from.as_str())
            .fold(Decimal::ZERO, |acc, r| {
                if r.tx_type.is_inflow() {
                    acc + r.amount
                } else {
                    acc - r.amount
                }
            });

        let period: Vec<&EurTxRow> = self
            .eur_rows
            .iter()
            .filter(|r| in_range(&r.date, &self.date_from, &self.date_to))
            .collect();

        let sum_for = |t: EurTxType| -> (i64, Decimal) {
            let matching: Vec<&&EurTxRow> = period.iter().filter(|r| r.tx_type == t).collect();
            (
                matching.len() as i64,
                matching.iter().map(|r| r.amount).sum(),
            )
        };

        let (don_count, don_total) = sum_for(EurTxType::DonationIn);
        let (sf_count, sf_total) = sum_for(EurTxType::SelfFundingIn);
        let (pur_count, pur_total) = sum_for(EurTxType::PurchaseOut);
        let (tr_count, tr_total) = sum_for(EurTxType::TransferToBrlOut);

        let net = don_total + sf_total - pur_total - tr_total;
        let ending_balance = starting_balance + net;

        egui::Grid::new("eur_summary_grid")
            .num_columns(3)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                ui.label("");
                ui.label(egui::RichText::new("Count").strong());
                ui.label(egui::RichText::new("Total (€)").strong());
                ui.end_row();

                ui.label("Donations in");
                ui.label(don_count.to_string());
                ui.label(format!("{:.2}", don_total));
                ui.end_row();

                ui.label("Self-funding in");
                ui.label(sf_count.to_string());
                ui.label(format!("{:.2}", sf_total));
                ui.end_row();

                ui.label("Purchases out");
                ui.label(pur_count.to_string());
                ui.label(format!("{:.2}", pur_total));
                ui.end_row();

                ui.label("Transfers out");
                ui.label(tr_count.to_string());
                ui.label(format!("{:.2}", tr_total));
                ui.end_row();
            });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        ui.label(format!("Starting balance: € {:.2}", starting_balance));
        ui.label(format!("Net for period: € {:.2}", net));
        ui.label(egui::RichText::new(format!("Ending balance: € {:.2}", ending_balance)).strong());
    }

    fn show_brl_summary(&self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("BRL ledger summary").strong());
        ui.add_space(6.0);

        let starting_balance: Decimal = self
            .brl_rows
            .iter()
            .filter(|r| !self.date_from.is_empty() && r.date.as_str() < self.date_from.as_str())
            .fold(Decimal::ZERO, |acc, r| {
                if r.tx_type.is_inflow() {
                    acc + r.amount
                } else {
                    acc - r.amount
                }
            });

        let period: Vec<&BrlTxRow> = self
            .brl_rows
            .iter()
            .filter(|r| in_range(&r.date, &self.date_from, &self.date_to))
            .collect();

        let sum_for = |t: BrlTxType| -> (i64, Decimal) {
            let matching: Vec<&&BrlTxRow> = period.iter().filter(|r| r.tx_type == t).collect();
            (
                matching.len() as i64,
                matching.iter().map(|r| r.amount).sum(),
            )
        };

        let (tr_count, tr_total) = sum_for(BrlTxType::TransferIn);
        let (pur_count, pur_total) = sum_for(BrlTxType::BrazilPurchaseOut);
        let (gift_count, gift_total) = sum_for(BrlTxType::CashGiftOut);

        let net = tr_total - pur_total - gift_total;
        let ending_balance = starting_balance + net;

        egui::Grid::new("brl_summary_grid")
            .num_columns(3)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                ui.label("");
                ui.label(egui::RichText::new("Count").strong());
                ui.label(egui::RichText::new("Total (R$)").strong());
                ui.end_row();

                ui.label("Transfer in");
                ui.label(tr_count.to_string());
                ui.label(format!("{:.2}", tr_total));
                ui.end_row();

                ui.label("Brazil purchases out");
                ui.label(pur_count.to_string());
                ui.label(format!("{:.2}", pur_total));
                ui.end_row();

                ui.label("Cash gifts out");
                ui.label(gift_count.to_string());
                ui.label(format!("{:.2}", gift_total));
                ui.end_row();
            });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        ui.label(format!("Starting balance: R$ {:.2}", starting_balance));
        ui.label(format!("Net for period: R$ {:.2}", net));
        ui.label(egui::RichText::new(format!("Ending balance: R$ {:.2}", ending_balance)).strong());
    }

    fn show_inventory_summary(&self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Inventory summary").strong());
        ui.weak("Snapshot of current inventory — not affected by the date/recipient filters.");
        ui.add_space(6.0);

        let mut by_cat_status: BTreeMap<(String, &'static str), i64> = BTreeMap::new();
        let mut by_location: BTreeMap<&'static str, i64> = BTreeMap::new();

        for item in &self.inventory_rows {
            *by_cat_status
                .entry((item.category_name.clone(), item.status.label()))
                .or_insert(0) += 1;
            *by_location.entry(item.location.label()).or_insert(0) += 1;
        }

        if self.inventory_rows.is_empty() {
            ui.weak("No inventory items yet.");
            return;
        }

        ui.label(egui::RichText::new("By category & status").italics());
        TableBuilder::new(ui)
            .id_salt("inv_cat_status_table")
            .striped(true)
            .column(Column::auto().at_least(160.0))
            .column(Column::auto().at_least(100.0))
            .column(Column::remainder().at_least(80.0))
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.strong("Category");
                });
                header.col(|ui| {
                    ui.strong("Status");
                });
                header.col(|ui| {
                    ui.strong("Count");
                });
            })
            .body(|mut body| {
                for ((cat, status), count) in &by_cat_status {
                    body.row(22.0, |mut row| {
                        row.col(|ui| {
                            ui.label(cat);
                        });
                        row.col(|ui| {
                            ui.label(*status);
                        });
                        row.col(|ui| {
                            ui.label(count.to_string());
                        });
                    });
                }
            });

        ui.add_space(12.0);
        ui.label(egui::RichText::new("By location").italics());
        TableBuilder::new(ui)
            .id_salt("inv_location_table")
            .striped(true)
            .column(Column::auto().at_least(120.0))
            .column(Column::remainder().at_least(80.0))
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.strong("Location");
                });
                header.col(|ui| {
                    ui.strong("Count");
                });
            })
            .body(|mut body| {
                for (loc, count) in &by_location {
                    body.row(22.0, |mut row| {
                        row.col(|ui| {
                            ui.label(*loc);
                        });
                        row.col(|ui| {
                            ui.label(count.to_string());
                        });
                    });
                }
            });
    }

    fn show_outbound_summary(&self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Outbound summary").strong());
        ui.add_space(6.0);

        let filtered: Vec<&OutboundEventRow> = self
            .outbound_rows
            .iter()
            .filter(|e| in_range(&e.date, &self.date_from, &self.date_to))
            .filter(|e| {
                self.recipient_filter
                    .is_none_or(|rid| e.recipient_project_id == rid)
            })
            .collect();

        if filtered.is_empty() {
            ui.weak("No outbound donations in the selected range.");
            return;
        }

        let mut by_recipient: BTreeMap<String, (i64, i64, Decimal)> = BTreeMap::new();
        for e in &filtered {
            let entry =
                by_recipient
                    .entry(e.recipient_name.clone())
                    .or_insert((0, 0, Decimal::ZERO));
            entry.0 += 1;
            entry.1 += e.item_count;
            entry.2 += e.cash_amount_brl.unwrap_or(Decimal::ZERO);
        }

        ui.label(egui::RichText::new("Per recipient project").italics());
        TableBuilder::new(ui)
            .id_salt("outbound_by_recipient_table")
            .striped(true)
            .column(Column::auto().at_least(160.0))
            .column(Column::auto().at_least(80.0))
            .column(Column::auto().at_least(80.0))
            .column(Column::remainder().at_least(100.0))
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.strong("Recipient");
                });
                header.col(|ui| {
                    ui.strong("Events");
                });
                header.col(|ui| {
                    ui.strong("Items");
                });
                header.col(|ui| {
                    ui.strong("Cash (R$)");
                });
            })
            .body(|mut body| {
                for (name, (events, items, cash)) in &by_recipient {
                    body.row(22.0, |mut row| {
                        row.col(|ui| {
                            ui.label(name);
                        });
                        row.col(|ui| {
                            ui.label(events.to_string());
                        });
                        row.col(|ui| {
                            ui.label(items.to_string());
                        });
                        row.col(|ui| {
                            ui.label(format!("{:.2}", cash));
                        });
                    });
                }
            });

        ui.add_space(12.0);
        ui.label(egui::RichText::new("Events").italics());
        TableBuilder::new(ui)
            .id_salt("outbound_events_table")
            .striped(true)
            .column(Column::auto().at_least(90.0))
            .column(Column::auto().at_least(140.0))
            .column(Column::auto().at_least(60.0))
            .column(Column::remainder().at_least(90.0))
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.strong("Date");
                });
                header.col(|ui| {
                    ui.strong("Recipient");
                });
                header.col(|ui| {
                    ui.strong("Items");
                });
                header.col(|ui| {
                    ui.strong("Cash (R$)");
                });
            })
            .body(|mut body| {
                for e in &filtered {
                    body.row(22.0, |mut row| {
                        row.col(|ui| {
                            ui.label(&e.date);
                        });
                        row.col(|ui| {
                            ui.label(&e.recipient_name);
                        });
                        row.col(|ui| {
                            ui.label(e.item_count.to_string());
                        });
                        row.col(|ui| {
                            let cash = e.cash_amount_brl.unwrap_or(Decimal::ZERO);
                            ui.label(if cash > Decimal::ZERO {
                                format!("{:.2}", cash)
                            } else {
                                "—".to_string()
                            });
                        });
                    });
                }
            });
    }

    fn show_audit_trail(&self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Full audit trail").strong());
        ui.weak("Every transaction and outbound event in the selected range, with a count of attached documents.");
        ui.add_space(6.0);

        let entries = self.build_audit_entries();

        if entries.is_empty() {
            ui.weak("No activity in the selected range.");
            return;
        }

        TableBuilder::new(ui)
            .id_salt("audit_trail_table")
            .striped(true)
            .column(Column::auto().at_least(90.0))
            .column(Column::auto().at_least(60.0))
            .column(Column::auto().at_least(110.0))
            .column(Column::auto().at_least(200.0))
            .column(Column::auto().at_least(90.0))
            .column(Column::remainder().at_least(60.0))
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.strong("Date");
                });
                header.col(|ui| {
                    ui.strong("Ledger");
                });
                header.col(|ui| {
                    ui.strong("Type");
                });
                header.col(|ui| {
                    ui.strong("Description");
                });
                header.col(|ui| {
                    ui.strong("Amount");
                });
                header.col(|ui| {
                    ui.strong("Docs");
                });
            })
            .body(|mut body| {
                for e in &entries {
                    body.row(22.0, |mut row| {
                        row.col(|ui| {
                            ui.label(&e.date);
                        });
                        row.col(|ui| {
                            ui.label(e.ledger);
                        });
                        row.col(|ui| {
                            ui.label(e.kind);
                        });
                        row.col(|ui| {
                            ui.label(&e.description);
                        });
                        row.col(|ui| {
                            ui.label(&e.amount);
                        });
                        row.col(|ui| {
                            ui.label(if e.docs > 0 {
                                e.docs.to_string()
                            } else {
                                "—".to_string()
                            });
                        });
                    });
                }
            });
    }
    fn csv_data_donors(&self) -> (Vec<String>, Vec<Vec<String>>) {
        let headers = [
            "Donor",
            "Cash donations",
            "Cash total (EUR)",
            "Physical donations",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let rows = self
            .build_donor_rows()
            .into_iter()
            .map(|r| {
                vec![
                    r.name,
                    r.cash_count.to_string(),
                    format!("{:.2}", r.cash_total),
                    r.item_count.to_string(),
                ]
            })
            .collect();
        (headers, rows)
    }

    fn build_donor_rows(&self) -> Vec<DonorRow> {
        let mut rows: Vec<DonorRow> = Vec::new();
        for donor in &self.donors {
            let cash: Vec<&EurTxRow> = self
                .eur_rows
                .iter()
                .filter(|r| {
                    r.tx_type == EurTxType::DonationIn
                        && r.donor_id == Some(donor.id)
                        && in_range(&r.date, &self.date_from, &self.date_to)
                })
                .collect();
            let item_count = self
                .donations
                .iter()
                .filter(|d| {
                    d.donor_id == Some(donor.id)
                        && in_range(&d.date_received, &self.date_from, &self.date_to)
                })
                .count() as i64;
            if cash.is_empty() && item_count == 0 {
                continue;
            }
            let cash_total: Decimal = cash.iter().map(|r| r.amount).sum();
            rows.push(DonorRow {
                name: donor.name.clone(),
                cash_count: cash.len() as i64,
                cash_total,
                item_count,
            });
        }
        let anon_cash: Vec<&EurTxRow> = self
            .eur_rows
            .iter()
            .filter(|r| {
                r.tx_type == EurTxType::DonationIn
                    && r.donor_id.is_none()
                    && in_range(&r.date, &self.date_from, &self.date_to)
            })
            .collect();
        let anon_items = self
            .donations
            .iter()
            .filter(|d| {
                d.donor_id.is_none() && in_range(&d.date_received, &self.date_from, &self.date_to)
            })
            .count() as i64;
        if !anon_cash.is_empty() || anon_items > 0 {
            let cash_total: Decimal = anon_cash.iter().map(|r| r.amount).sum();
            rows.push(DonorRow {
                name: "Anonymous".to_string(),
                cash_count: anon_cash.len() as i64,
                cash_total,
                item_count: anon_items,
            });
        }
        rows
    }

    fn csv_data_eur(&self) -> (Vec<String>, Vec<Vec<String>>) {
        let headers = ["Date", "Type", "Description", "Amount (EUR)"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let rows = self
            .eur_rows
            .iter()
            .filter(|r| in_range(&r.date, &self.date_from, &self.date_to))
            .map(|r| {
                let description = match r.tx_type {
                    EurTxType::DonationIn => r
                        .donor_name
                        .clone()
                        .unwrap_or_else(|| "Anonymous".to_string()),
                    EurTxType::SelfFundingIn => r.note.clone().unwrap_or_default(),
                    EurTxType::PurchaseOut => r.purchase_channel.clone().unwrap_or_default(),
                    EurTxType::TransferToBrlOut => "EUR→BRL transfer".to_string(),
                };
                let sign = if r.tx_type.is_inflow() { "" } else { "-" };
                vec![
                    r.date.clone(),
                    r.tx_type.label().to_string(),
                    description,
                    format!("{sign}{:.2}", r.amount),
                ]
            })
            .collect();
        (headers, rows)
    }

    fn csv_data_brl(&self) -> (Vec<String>, Vec<Vec<String>>) {
        let headers = ["Date", "Type", "Description", "Amount (BRL)"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let rows = self
            .brl_rows
            .iter()
            .filter(|r| in_range(&r.date, &self.date_from, &self.date_to))
            .map(|r| {
                let description = match r.tx_type {
                    BrlTxType::TransferIn => "EUR→BRL transfer".to_string(),
                    BrlTxType::BrazilPurchaseOut => r.purchase_channel.clone().unwrap_or_default(),
                    BrlTxType::CashGiftOut => r.recipient_name.clone().unwrap_or_default(),
                };
                let sign = if r.tx_type.is_inflow() { "" } else { "-" };
                vec![
                    r.date.clone(),
                    r.tx_type.label().to_string(),
                    description,
                    format!("{sign}{:.2}", r.amount),
                ]
            })
            .collect();
        (headers, rows)
    }

    fn csv_data_inventory(&self) -> (Vec<String>, Vec<Vec<String>>) {
        let headers = [
            "Name",
            "Category",
            "Status",
            "Location",
            "Source type",
            "Source",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let rows = self
            .inventory_rows
            .iter()
            .map(|item| {
                let source_type = match item.source_type {
                    SourceType::Donation => "Donation",
                    SourceType::Purchase => "Purchase",
                };
                vec![
                    item.name.clone(),
                    item.category_name.clone(),
                    item.status.label().to_string(),
                    item.location.label().to_string(),
                    source_type.to_string(),
                    item.source_desc.clone(),
                ]
            })
            .collect();
        (headers, rows)
    }

    fn csv_data_outbound(&self) -> (Vec<String>, Vec<Vec<String>>) {
        let headers = ["Date", "Recipient", "Items", "Cash (BRL)"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let rows = self
            .outbound_rows
            .iter()
            .filter(|e| in_range(&e.date, &self.date_from, &self.date_to))
            .filter(|e| {
                self.recipient_filter
                    .is_none_or(|rid| e.recipient_project_id == rid)
            })
            .map(|e| {
                vec![
                    e.date.clone(),
                    e.recipient_name.clone(),
                    e.item_count.to_string(),
                    e.cash_amount_brl
                        .map(|c| format!("{:.2}", c))
                        .unwrap_or_default(),
                ]
            })
            .collect();
        (headers, rows)
    }

    fn csv_data_audit_trail(&self) -> (Vec<String>, Vec<Vec<String>>) {
        let headers = [
            "Date",
            "Ledger",
            "Type",
            "Description",
            "Amount",
            "Documents",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let rows = self
            .build_audit_entries()
            .into_iter()
            .map(|e| {
                vec![
                    e.date,
                    e.ledger.to_string(),
                    e.kind.to_string(),
                    e.description,
                    e.amount,
                    if e.docs > 0 {
                        e.docs.to_string()
                    } else {
                        String::new()
                    },
                ]
            })
            .collect();
        (headers, rows)
    }

    fn build_audit_entries(&self) -> Vec<AuditEntry> {
        let mut entries: Vec<AuditEntry> = Vec::new();

        for r in &self.eur_rows {
            if !in_range(&r.date, &self.date_from, &self.date_to) {
                continue;
            }
            let docs = match r.tx_type {
                EurTxType::PurchaseOut => r
                    .linked_purchase_id
                    .and_then(|id| self.doc_counts.get(&("purchase".to_string(), id)).copied())
                    .unwrap_or(0),
                EurTxType::TransferToBrlOut => r
                    .linked_transfer_id
                    .and_then(|id| self.doc_counts.get(&("transfer".to_string(), id)).copied())
                    .unwrap_or(0),
                _ => 0,
            };
            let description = match r.tx_type {
                EurTxType::DonationIn => r
                    .donor_name
                    .clone()
                    .unwrap_or_else(|| "Anonymous".to_string()),
                EurTxType::SelfFundingIn => r.note.clone().unwrap_or_default(),
                EurTxType::PurchaseOut => r.purchase_channel.clone().unwrap_or_default(),
                EurTxType::TransferToBrlOut => "EUR→BRL transfer".to_string(),
            };
            let sign = if r.tx_type.is_inflow() { "+" } else { "-" };
            entries.push(AuditEntry {
                date: r.date.clone(),
                ledger: "EUR",
                kind: r.tx_type.label(),
                description,
                amount: format!("{sign}€{:.2}", r.amount),
                docs,
            });
        }

        for r in &self.brl_rows {
            if !in_range(&r.date, &self.date_from, &self.date_to) {
                continue;
            }
            if let (Some(rid), BrlTxType::CashGiftOut) = (self.recipient_filter, r.tx_type) {
                let matches = self
                    .outbound_rows
                    .iter()
                    .find(|e| Some(e.id) == r.linked_outbound_event_id)
                    .map(|e| e.recipient_project_id == rid)
                    .unwrap_or(false);
                if !matches {
                    continue;
                }
            }
            let docs = match r.tx_type {
                BrlTxType::BrazilPurchaseOut => r
                    .linked_purchase_id
                    .and_then(|id| self.doc_counts.get(&("purchase".to_string(), id)).copied())
                    .unwrap_or(0),
                BrlTxType::TransferIn => r
                    .linked_transfer_id
                    .and_then(|id| self.doc_counts.get(&("transfer".to_string(), id)).copied())
                    .unwrap_or(0),
                BrlTxType::CashGiftOut => 0,
            };
            let description = match r.tx_type {
                BrlTxType::TransferIn => "EUR→BRL transfer".to_string(),
                BrlTxType::BrazilPurchaseOut => r.purchase_channel.clone().unwrap_or_default(),
                BrlTxType::CashGiftOut => r.recipient_name.clone().unwrap_or_default(),
            };
            let sign = if r.tx_type.is_inflow() { "+" } else { "-" };
            entries.push(AuditEntry {
                date: r.date.clone(),
                ledger: "BRL",
                kind: r.tx_type.label(),
                description,
                amount: format!("{sign}R${:.2}", r.amount),
                docs,
            });
        }

        for e in &self.outbound_rows {
            if !in_range(&e.date, &self.date_from, &self.date_to) {
                continue;
            }
            if let Some(rid) = self.recipient_filter {
                if e.recipient_project_id != rid {
                    continue;
                }
            }
            let mut desc = format!("{} item(s) to {}", e.item_count, e.recipient_name);
            if let Some(cash) = e.cash_amount_brl {
                if cash > Decimal::ZERO {
                    desc.push_str(&format!(" + R$ {cash:.2} cash gift"));
                }
            }
            entries.push(AuditEntry {
                date: e.date.clone(),
                ledger: "Outbound",
                kind: "Donation",
                description: desc,
                amount: String::new(),
                docs: 0,
            });
        }

        entries.sort_by(|a, b| b.date.cmp(&a.date));
        entries
    }
}

fn in_range(date: &str, from: &str, to: &str) -> bool {
    (from.is_empty() || date >= from) && (to.is_empty() || date <= to)
}
