use eframe::egui;
use egui_extras::{Column, TableBuilder};
use rusqlite::Connection;
use rust_decimal::Decimal;
use rust_i18n::t;
use std::collections::{BTreeMap, HashMap};

use crate::format;

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

// (tab, i18n key for the on-screen/tab label, fixed English slug for the CSV
// export filename — kept separate from the translated label so the exported
// filename doesn't change based on the active UI language, in the same
// spirit as T4's filename locale-independence).
const TABS: &[(Tab, &str, &str)] = &[
    (Tab::Donors, "reports.tab.donors", "donor-breakdown"),
    (Tab::Eur, "reports.tab.eur", "eur-ledger-summary"),
    (Tab::Brl, "reports.tab.brl", "brl-ledger-summary"),
    (Tab::Inventory, "reports.tab.inventory", "inventory-summary"),
    (Tab::Outbound, "reports.tab.outbound", "outbound-summary"),
    (Tab::AuditTrail, "reports.tab.audit_trail", "audit-trail"),
];

pub struct ReportsView {
    tab: Tab,
    date_from: String,
    date_to: String,
    /// ISO-normalized `date_from`/`date_to`, recomputed every frame in
    /// `show_filters` — empty means "no bound", same sentinel `in_range`
    /// already used before these fields existed. Everything that filters
    /// by date range reads these, never the raw typed fields directly,
    /// since `date_from`/`date_to` can hold typed "DD.MM.YYYY" (or
    /// momentarily invalid input) while every stored `.date` field being
    /// compared against stays ISO.
    date_from_iso: String,
    date_to_iso: String,
    date_from_invalid: bool,
    date_to_invalid: bool,
    recipient_filter: Option<i64>,

    loaded: bool,
    last_refreshed: Option<String>,
    error: Option<String>,
    export_status: Option<Result<String, String>>,
    csv_path_input: Option<String>,
    csv_pending_tab: Option<Tab>,
    pdf_path_input: Option<String>,
    /// Target report language (SPEC.md §6.3) — an explicit choice in the
    /// export dialog, defaulted from the ambient UI locale when a dialog
    /// opens but never read implicitly from it thereafter.
    export_locale: String,

    donors: Vec<Donor>,
    donations: Vec<PhysicalDonation>,
    eur_rows: Vec<EurTxRow>,
    brl_rows: Vec<BrlTxRow>,
    inventory_rows: Vec<InventoryItemRow>,
    outbound_rows: Vec<OutboundEventRow>,
    recipient_projects: Vec<RecipientProject>,
    doc_counts: HashMap<(String, i64), i64>,
    outbound_item_names: HashMap<i64, Vec<String>>,
}

impl Default for ReportsView {
    fn default() -> Self {
        Self {
            tab: Tab::Donors,
            date_from: String::new(),
            date_to: String::new(),
            date_from_iso: String::new(),
            date_to_iso: String::new(),
            date_from_invalid: false,
            date_to_invalid: false,
            recipient_filter: None,
            loaded: false,
            last_refreshed: None,
            error: None,
            export_status: None,
            csv_path_input: None,
            csv_pending_tab: None,
            pdf_path_input: None,
            export_locale: "en".to_string(),
            donors: Vec::new(),
            donations: Vec::new(),
            eur_rows: Vec::new(),
            brl_rows: Vec::new(),
            inventory_rows: Vec::new(),
            outbound_rows: Vec::new(),
            recipient_projects: Vec::new(),
            doc_counts: HashMap::new(),
            outbound_item_names: HashMap::new(),
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
    /// EUR/BRL rows only: pre-resolved at the ambient UI locale (via
    /// `r.tx_type.label()` for `kind`; `ledger` is always the fixed
    /// currency code "EUR"/"BRL", never translated). A disclosed, minor
    /// limitation — unlike everything else in this struct, this doesn't
    /// follow an export's explicit chosen locale. `None` for outbound rows,
    /// which use `outbound` below instead: those needed a real fix (not
    /// just a disclosed limitation) since Ledger/Type/Description are full
    /// locale-sensitive prose there, not a single enum label.
    ledger_kind: Option<(String, String)>,
    /// EUR/BRL only (donor name / purchase channel / note — mostly DB user
    /// data, not translated prose, see `eur_tx_description`/
    /// `brl_tx_description`). Empty for outbound rows.
    description: String,
    /// Outbound rows only — raw pieces, not pre-formatted text, so each
    /// consumer (on-screen table vs CSV vs PDF) can build Ledger/Type/
    /// Description in its own target locale via `outbound_audit_text()`
    /// instead of a value baked once at whichever locale was ambient when
    /// `build_audit_entries` ran.
    outbound: Option<OutboundAuditInfo>,
    amount: Option<AuditAmount>,
    docs: i64,
}

struct OutboundAuditInfo {
    item_count: i64,
    recipient_name: String,
    cash: Option<Decimal>,
}

struct AuditAmount {
    sign: &'static str,
    symbol: &'static str,
    value: Decimal,
}

/// Builds an outbound audit row's Ledger/Type/Description text in an
/// explicit `locale` — fixes a bug where this used to be baked once inside
/// `build_audit_entries` at the ambient UI locale, so a CSV/PDF export
/// chosen in a different language still showed these three columns in
/// whatever language happened to be active in the UI at the time.
fn outbound_audit_text(
    info: &OutboundAuditInfo,
    locale: &str,
    fmt_cash: impl Fn(Decimal) -> String,
) -> (String, String, String) {
    let ledger = t!("sidebar.outbound", locale = locale).into_owned();
    let kind = t!("status.source_type.donation", locale = locale).into_owned();
    let mut description = if info.item_count == 1 {
        t!(
            "reports.audit.outbound_desc_one",
            locale = locale,
            recipient = info.recipient_name.as_str()
        )
        .into_owned()
    } else {
        t!(
            "reports.audit.outbound_desc_other",
            locale = locale,
            count = info.item_count,
            recipient = info.recipient_name.as_str()
        )
        .into_owned()
    };
    if let Some(cash) = info.cash {
        description.push_str(&t!(
            "reports.audit.outbound_cash_suffix",
            locale = locale,
            cash = fmt_cash(cash)
        ));
    }
    (ledger, kind, description)
}

impl ReportsView {
    pub fn invalidate(&mut self) {
        self.loaded = false;
    }

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

        ui.heading(t!("sidebar.reports").as_ref());
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
        self.outbound_item_names = outbound_qry::item_names_by_event(db)?;
        Ok(())
    }

    fn show_filters(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(t!("reports.filter.from").as_ref());
            ui.add(
                egui::TextEdit::singleline(&mut self.date_from)
                    .hint_text(t!("common.field.date_hint").as_ref())
                    .desired_width(110.0),
            );
            ui.label(t!("reports.filter.to").as_ref());
            ui.add(
                egui::TextEdit::singleline(&mut self.date_to)
                    .hint_text(t!("common.field.date_hint").as_ref())
                    .desired_width(110.0),
            );
            let (from_iso, from_invalid) = normalize_filter_date(&self.date_from);
            let (to_iso, to_invalid) = normalize_filter_date(&self.date_to);
            self.date_from_iso = from_iso;
            self.date_to_iso = to_iso;
            self.date_from_invalid = from_invalid;
            self.date_to_invalid = to_invalid;

            ui.add_space(12.0);
            ui.label(t!("reports.filter.recipient").as_ref());
            let selected_label = self
                .recipient_filter
                .and_then(|id| self.recipient_projects.iter().find(|p| p.id == id))
                .map(|p| p.name.clone())
                .unwrap_or_else(|| t!("reports.combo.all").into_owned());
            egui::ComboBox::from_id_salt("reports_recipient_filter")
                .selected_text(selected_label)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.recipient_filter,
                        None,
                        t!("reports.combo.all").as_ref(),
                    );
                    for p in &self.recipient_projects {
                        ui.selectable_value(&mut self.recipient_filter, Some(p.id), &p.name);
                    }
                });

            ui.add_space(12.0);
            if ui
                .button(t!("reports.button.clear_filters").as_ref())
                .clicked()
            {
                self.date_from.clear();
                self.date_to.clear();
                self.date_from_iso.clear();
                self.date_to_iso.clear();
                self.date_from_invalid = false;
                self.date_to_invalid = false;
                self.recipient_filter = None;
            }
            if ui.button(t!("reports.button.refresh").as_ref()).clicked() {
                self.loaded = false;
                ui.ctx().request_repaint();
            }
            if let Some(ts) = &self.last_refreshed {
                ui.weak(t!("reports.refreshed_at", time = ts).into_owned());
            }
            ui.add_space(8.0);
            if ui
                .button(t!("reports.button.export_csv").as_ref())
                .clicked()
            {
                self.export_status = None;
                let tab_slug = TABS
                    .iter()
                    .find(|(t, _, _)| *t == self.tab)
                    .map(|(_, _, slug)| *slug)
                    .unwrap_or("report");
                let filename = format!("{tab_slug}.csv");
                let default_path = dirs::download_dir()
                    .unwrap_or_else(|| {
                        dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."))
                    })
                    .join(&filename)
                    .to_string_lossy()
                    .into_owned();
                self.export_locale = rust_i18n::locale().to_string();
                self.csv_path_input = Some(default_path);
                self.csv_pending_tab = Some(self.tab);
            }
            if ui
                .button(t!("reports.button.export_pdf").as_ref())
                .clicked()
            {
                self.export_status = None;
                let default_path = dirs::download_dir()
                    .unwrap_or_else(|| {
                        dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."))
                    })
                    .join("adm-sfa-report.pdf")
                    .to_string_lossy()
                    .into_owned();
                self.export_locale = rust_i18n::locale().to_string();
                self.pdf_path_input = Some(default_path);
            }
        });

        if self.date_from_invalid || self.date_to_invalid {
            ui.colored_label(egui::Color32::RED, t!("common.error.invalid_date").as_ref());
        }

        let mut csv_save = false;
        let mut csv_cancel = false;
        if let Some(ref mut path_str) = self.csv_path_input {
            ui.group(|ui| {
                ui.label(t!("reports.csv.save_to").as_ref());
                ui.add(egui::TextEdit::singleline(path_str).desired_width(500.0));
                show_export_locale_picker(ui, &mut self.export_locale, "csv_export_locale_combo");
                ui.horizontal(|ui| {
                    if ui.button(t!("common.save").as_ref()).clicked() {
                        csv_save = true;
                    }
                    if ui.button(t!("common.cancel").as_ref()).clicked() {
                        csv_cancel = true;
                    }
                });
            });
        }
        if csv_save {
            if let (Some(path_str), Some(tab)) =
                (self.csv_path_input.take(), self.csv_pending_tab.take())
            {
                let path = std::path::PathBuf::from(path_str.trim());
                let locale = self.export_locale.clone();
                let (headers, rows) = match tab {
                    Tab::Donors => self.csv_data_donors(&locale, true),
                    Tab::Eur => self.csv_data_eur(&locale, true),
                    Tab::Brl => self.csv_data_brl(&locale, true),
                    Tab::Inventory => self.csv_data_inventory(&locale),
                    Tab::Outbound => self.csv_data_outbound(&locale, true),
                    Tab::AuditTrail => self.csv_data_audit_trail(&locale, true),
                };
                self.export_status = Some(
                    crate::reports::csv::write(&path, &headers, &rows)
                        .map(|()| t!("common.status.saved_to", path = path.display()).into_owned())
                        .map_err(|e| e.to_string()),
                );
            }
        } else if csv_cancel {
            self.csv_path_input = None;
            self.csv_pending_tab = None;
        }

        let mut pdf_save = false;
        let mut pdf_cancel = false;
        if let Some(ref mut path_str) = self.pdf_path_input {
            ui.group(|ui| {
                ui.label(t!("reports.pdf.save_to").as_ref());
                ui.add(egui::TextEdit::singleline(path_str).desired_width(500.0));
                show_export_locale_picker(ui, &mut self.export_locale, "pdf_export_locale_combo");
                ui.horizontal(|ui| {
                    if ui.button(t!("common.save").as_ref()).clicked() {
                        pdf_save = true;
                    }
                    if ui.button(t!("common.cancel").as_ref()).clicked() {
                        pdf_cancel = true;
                    }
                });
            });
        }
        if pdf_save {
            let path_str = self
                .pdf_path_input
                .as_deref()
                .unwrap_or("")
                .trim()
                .to_string();
            let path = std::path::PathBuf::from(&path_str);
            if path.as_os_str().is_empty() {
                self.export_status = Some(Err(t!("common.error.path_required").into_owned()));
            } else {
                self.pdf_path_input = None;
                let generated_iso = chrono::Local::now().format("%Y-%m-%d").to_string();
                let generated = format::date_in(&generated_iso, &self.export_locale);
                let from_display = format::date_in(&self.date_from_iso, &self.export_locale);
                let to_display = format::date_in(&self.date_to_iso, &self.export_locale);
                let sections = self.build_pdf_sections(&self.export_locale);
                self.export_status = Some(
                    crate::reports::pdf::export(
                        &path,
                        &generated,
                        &from_display,
                        &to_display,
                        &sections,
                    )
                    .map(|()| t!("common.status.saved_to", path = path.display()).into_owned())
                    .map_err(|e| e.to_string()),
                );
            }
        } else if pdf_cancel {
            self.pdf_path_input = None;
        }

        if let Some(status) = &self.export_status {
            match status {
                Ok(msg) => ui.colored_label(egui::Color32::DARK_GREEN, msg),
                Err(msg) => ui.colored_label(
                    egui::Color32::RED,
                    t!("reports.error.export_failed", msg = msg).into_owned(),
                ),
            };
        }
        ui.weak(t!("reports.hint.filters").as_ref());
    }

    fn show_tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for &(tab, key, _) in TABS {
                let label = t!(key);
                if ui
                    .selectable_label(self.tab == tab, label.as_ref())
                    .clicked()
                {
                    self.tab = tab;
                }
            }
        });
    }

    fn show_donor_breakdown(&self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new(t!("reports.donor.heading").as_ref()).strong());
        ui.add_space(6.0);

        let rows = self.build_donor_rows();

        if rows.is_empty() {
            ui.weak(t!("reports.donor.empty").as_ref());
        } else {
            TableBuilder::new(ui)
                .id_salt("donor_breakdown_table")
                .striped(true)
                .vscroll(false)
                .column(Column::auto().at_least(160.0))
                .column(Column::auto().at_least(110.0))
                .column(Column::auto().at_least(110.0))
                .column(Column::remainder().at_least(130.0))
                .header(20.0, |mut header| {
                    header.col(|ui| {
                        ui.strong(t!("common.field.donor").as_ref());
                    });
                    header.col(|ui| {
                        ui.strong(t!("reports.donor.col.cash_count").as_ref());
                    });
                    header.col(|ui| {
                        ui.strong(t!("reports.donor.col.cash_total").as_ref());
                    });
                    header.col(|ui| {
                        ui.strong(t!("reports.donor.col.items").as_ref());
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
                                ui.label(format::amount(r.cash_total));
                            });
                            row.col(|ui| {
                                ui.label(r.item_count.to_string());
                            });
                        });
                    }
                });
        }

        self.show_donor_activity_log(ui);
    }

    fn show_donor_activity_log(&self, ui: &mut egui::Ui) {
        enum DonorLogRow {
            Cash {
                date: String,
                donor: String,
                amount: Decimal,
            },
            Physical {
                date: String,
                donor: String,
                items: String,
            },
        }

        ui.add_space(14.0);
        ui.separator();
        ui.add_space(6.0);
        ui.label(egui::RichText::new(t!("reports.donor.log.heading").as_ref()).strong());
        ui.weak(t!("reports.donor.log.hint").as_ref());
        ui.add_space(6.0);

        let mut log: Vec<DonorLogRow> = Vec::new();

        for r in self
            .eur_rows
            .iter()
            .filter(|r| r.tx_type == EurTxType::DonationIn)
        {
            log.push(DonorLogRow::Cash {
                date: r.date.clone(),
                donor: donor_or_anonymous(&r.donor_name),
                amount: r.amount,
            });
        }

        for d in &self.donations {
            let items: Vec<String> = self
                .inventory_rows
                .iter()
                .filter(|i| i.source_donation_id == Some(d.id))
                .map(|i| i.name.clone())
                .collect();
            log.push(DonorLogRow::Physical {
                date: d.date_received.clone(),
                donor: donor_or_anonymous(&d.donor_name),
                items: items.join(", "),
            });
        }

        if log.is_empty() {
            ui.weak(t!("reports.donor.log.empty").as_ref());
            return;
        }

        // No cross-table "registration order" tiebreak exists here (cash
        // and physical rows come from separate tables with independent id
        // sequences) — same-date ties just put cash rows first, a simple
        // deterministic rule rather than a claimed ordering guarantee.
        log.sort_by(|a, b| {
            let (date_a, rank_a) = match a {
                DonorLogRow::Cash { date, .. } => (date, 0),
                DonorLogRow::Physical { date, .. } => (date, 1),
            };
            let (date_b, rank_b) = match b {
                DonorLogRow::Cash { date, .. } => (date, 0),
                DonorLogRow::Physical { date, .. } => (date, 1),
            };
            date_a.cmp(date_b).then(rank_a.cmp(&rank_b))
        });

        TableBuilder::new(ui)
            .id_salt("donor_activity_log_table")
            .striped(true)
            .vscroll(false)
            .column(Column::auto().at_least(90.0))
            .column(Column::auto().at_least(140.0))
            .column(Column::auto().at_least(90.0))
            .column(Column::remainder().at_least(160.0))
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.strong(t!("common.col.date").as_ref());
                });
                header.col(|ui| {
                    ui.strong(t!("common.field.donor").as_ref());
                });
                header.col(|ui| {
                    ui.strong(t!("reports.donor.log.col.cash").as_ref());
                });
                header.col(|ui| {
                    ui.strong(t!("reports.donor.log.col.items").as_ref());
                });
            })
            .body(|mut body| {
                for entry in &log {
                    body.row(18.0, |mut row| match entry {
                        DonorLogRow::Cash {
                            date,
                            donor,
                            amount,
                        } => {
                            row.col(|ui| {
                                ui.label(format::date(date));
                            });
                            row.col(|ui| {
                                ui.label(donor);
                            });
                            row.col(|ui| {
                                ui.label(format::amount(*amount));
                            });
                            row.col(|_| {});
                        }
                        DonorLogRow::Physical { date, donor, items } => {
                            row.col(|ui| {
                                ui.label(format::date(date));
                            });
                            row.col(|ui| {
                                ui.label(donor);
                            });
                            row.col(|_| {});
                            row.col(|ui| {
                                ui.label(items);
                            });
                        }
                    });
                }
            });
    }

    fn show_eur_summary(&self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new(t!("reports.tab.eur").as_ref()).strong());
        ui.add_space(6.0);

        let starting_balance: Decimal = self
            .eur_rows
            .iter()
            .filter(|r| {
                !self.date_from_iso.is_empty() && r.date.as_str() < self.date_from_iso.as_str()
            })
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
            .filter(|r| in_range(&r.date, &self.date_from_iso, &self.date_to_iso))
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
                ui.label(egui::RichText::new(t!("reports.grid.col.count").as_ref()).strong());
                ui.label(egui::RichText::new(t!("reports.eur.col.total").as_ref()).strong());
                ui.end_row();

                ui.label(t!("reports.eur.row.donations_in").as_ref());
                ui.label(don_count.to_string());
                ui.label(format::amount(don_total));
                ui.end_row();

                ui.label(t!("reports.eur.row.self_funding_in").as_ref());
                ui.label(sf_count.to_string());
                ui.label(format::amount(sf_total));
                ui.end_row();

                ui.label(t!("reports.eur.row.purchases_out").as_ref());
                ui.label(pur_count.to_string());
                ui.label(format::amount(pur_total));
                ui.end_row();

                ui.label(t!("reports.eur.row.transfers_out").as_ref());
                ui.label(tr_count.to_string());
                ui.label(format::amount(tr_total));
                ui.end_row();
            });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        ui.label(
            t!(
                "reports.eur.starting_balance",
                amount = format::amount(starting_balance)
            )
            .into_owned(),
        );
        ui.label(t!("reports.eur.net_for_period", amount = format::amount(net)).into_owned());
        ui.label(
            egui::RichText::new(
                t!(
                    "reports.eur.ending_balance",
                    amount = format::amount(ending_balance)
                )
                .into_owned(),
            )
            .strong(),
        );

        self.show_eur_running_ledger(ui);
    }

    fn show_eur_running_ledger(&self, ui: &mut egui::Ui) {
        ui.add_space(14.0);
        ui.separator();
        ui.add_space(6.0);
        ui.label(egui::RichText::new(t!("reports.running_ledger.heading").as_ref()).strong());
        ui.weak(t!("reports.eur.running_ledger.hint").as_ref());
        ui.add_space(6.0);

        let mut rows: Vec<&EurTxRow> = self.eur_rows.iter().collect();
        rows.sort_by(|a, b| a.date.cmp(&b.date).then(a.id.cmp(&b.id)));

        TableBuilder::new(ui)
            .id_salt("eur_running_ledger_table")
            .striped(true)
            .vscroll(false)
            .column(Column::auto().at_least(90.0))
            .column(Column::remainder().at_least(160.0))
            .column(Column::auto().at_least(90.0))
            .column(Column::auto().at_least(90.0))
            .column(Column::auto().at_least(90.0))
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.strong(t!("common.col.date").as_ref());
                });
                header.col(|ui| {
                    ui.strong(t!("common.col.description").as_ref());
                });
                header.col(|ui| {
                    ui.strong(t!("reports.running_ledger.col.inbound", symbol = "€").into_owned());
                });
                header.col(|ui| {
                    ui.strong(t!("reports.running_ledger.col.outbound", symbol = "€").into_owned());
                });
                header.col(|ui| {
                    ui.strong(t!("reports.running_ledger.col.balance", symbol = "€").into_owned());
                });
            })
            .body(|mut body| {
                let mut balance = Decimal::ZERO;
                body.row(20.0, |mut row| {
                    row.col(|_| {});
                    row.col(|ui| {
                        ui.weak(t!("reports.running_ledger.initial_balance").as_ref());
                    });
                    row.col(|_| {});
                    row.col(|_| {});
                    row.col(|ui| {
                        ui.label(format::amount(balance));
                    });
                });
                for r in rows {
                    let description = eur_tx_description(r);
                    let (inbound, outbound) = if r.tx_type.is_inflow() {
                        balance += r.amount;
                        (Some(r.amount), None)
                    } else {
                        balance -= r.amount;
                        (None, Some(r.amount))
                    };
                    body.row(18.0, |mut row| {
                        row.col(|ui| {
                            ui.label(format::date(&r.date));
                        });
                        row.col(|ui| {
                            ui.label(&description);
                        });
                        row.col(|ui| {
                            if let Some(v) = inbound {
                                ui.label(format::amount(v));
                            }
                        });
                        row.col(|ui| {
                            if let Some(v) = outbound {
                                ui.label(format::amount(v));
                            }
                        });
                        row.col(|ui| {
                            ui.label(format::amount(balance));
                        });
                    });
                }
            });
    }

    fn show_brl_summary(&self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new(t!("reports.tab.brl").as_ref()).strong());
        ui.add_space(6.0);

        let starting_balance: Decimal = self
            .brl_rows
            .iter()
            .filter(|r| {
                !self.date_from_iso.is_empty() && r.date.as_str() < self.date_from_iso.as_str()
            })
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
            .filter(|r| in_range(&r.date, &self.date_from_iso, &self.date_to_iso))
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
                ui.label(egui::RichText::new(t!("reports.grid.col.count").as_ref()).strong());
                ui.label(egui::RichText::new(t!("reports.brl.col.total").as_ref()).strong());
                ui.end_row();

                ui.label(t!("reports.brl.row.transfer_in").as_ref());
                ui.label(tr_count.to_string());
                ui.label(format::amount(tr_total));
                ui.end_row();

                ui.label(t!("reports.brl.row.purchases_out").as_ref());
                ui.label(pur_count.to_string());
                ui.label(format::amount(pur_total));
                ui.end_row();

                ui.label(t!("reports.brl.row.gifts_out").as_ref());
                ui.label(gift_count.to_string());
                ui.label(format::amount(gift_total));
                ui.end_row();
            });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        ui.label(
            t!(
                "reports.brl.starting_balance",
                amount = format::amount(starting_balance)
            )
            .into_owned(),
        );
        ui.label(t!("reports.brl.net_for_period", amount = format::amount(net)).into_owned());
        ui.label(
            egui::RichText::new(
                t!(
                    "reports.brl.ending_balance",
                    amount = format::amount(ending_balance)
                )
                .into_owned(),
            )
            .strong(),
        );

        self.show_brl_running_ledger(ui);
    }

    fn show_brl_running_ledger(&self, ui: &mut egui::Ui) {
        ui.add_space(14.0);
        ui.separator();
        ui.add_space(6.0);
        ui.label(egui::RichText::new(t!("reports.running_ledger.heading").as_ref()).strong());
        ui.weak(t!("reports.brl.running_ledger.hint").as_ref());
        ui.add_space(6.0);

        let mut rows: Vec<&BrlTxRow> = self.brl_rows.iter().collect();
        rows.sort_by(|a, b| a.date.cmp(&b.date).then(a.id.cmp(&b.id)));

        TableBuilder::new(ui)
            .id_salt("brl_running_ledger_table")
            .striped(true)
            .vscroll(false)
            .column(Column::auto().at_least(90.0))
            .column(Column::remainder().at_least(160.0))
            .column(Column::auto().at_least(90.0))
            .column(Column::auto().at_least(90.0))
            .column(Column::auto().at_least(90.0))
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.strong(t!("common.col.date").as_ref());
                });
                header.col(|ui| {
                    ui.strong(t!("common.col.description").as_ref());
                });
                header.col(|ui| {
                    ui.strong(t!("reports.running_ledger.col.inbound", symbol = "R$").into_owned());
                });
                header.col(|ui| {
                    ui.strong(
                        t!("reports.running_ledger.col.outbound", symbol = "R$").into_owned(),
                    );
                });
                header.col(|ui| {
                    ui.strong(t!("reports.running_ledger.col.balance", symbol = "R$").into_owned());
                });
            })
            .body(|mut body| {
                let mut balance = Decimal::ZERO;
                body.row(20.0, |mut row| {
                    row.col(|_| {});
                    row.col(|ui| {
                        ui.weak(t!("reports.running_ledger.initial_balance").as_ref());
                    });
                    row.col(|_| {});
                    row.col(|_| {});
                    row.col(|ui| {
                        ui.label(format::amount(balance));
                    });
                });
                for r in rows {
                    let description = brl_tx_description(r);
                    let (inbound, outbound) = if r.tx_type.is_inflow() {
                        balance += r.amount;
                        (Some(r.amount), None)
                    } else {
                        balance -= r.amount;
                        (None, Some(r.amount))
                    };
                    body.row(18.0, |mut row| {
                        row.col(|ui| {
                            ui.label(format::date(&r.date));
                        });
                        row.col(|ui| {
                            ui.label(&description);
                        });
                        row.col(|ui| {
                            if let Some(v) = inbound {
                                ui.label(format::amount(v));
                            }
                        });
                        row.col(|ui| {
                            if let Some(v) = outbound {
                                ui.label(format::amount(v));
                            }
                        });
                        row.col(|ui| {
                            ui.label(format::amount(balance));
                        });
                    });
                }
            });
    }

    fn show_inventory_summary(&self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new(t!("reports.tab.inventory").as_ref()).strong());
        ui.weak(t!("reports.inventory.hint").as_ref());
        ui.add_space(6.0);

        let mut by_cat_status: BTreeMap<(String, String), i64> = BTreeMap::new();
        let mut by_location: BTreeMap<String, i64> = BTreeMap::new();

        for item in &self.inventory_rows {
            *by_cat_status
                .entry((item.category_name.clone(), item.status.label()))
                .or_insert(0) += 1;
            *by_location.entry(item.location.label()).or_insert(0) += 1;
        }

        if self.inventory_rows.is_empty() {
            ui.weak(t!("reports.inventory.empty").as_ref());
            return;
        }

        ui.label(
            egui::RichText::new(t!("reports.inventory.by_cat_status.heading").as_ref()).italics(),
        );
        TableBuilder::new(ui)
            .id_salt("inv_cat_status_table")
            .striped(true)
            .vscroll(false)
            .column(Column::auto().at_least(160.0))
            .column(Column::auto().at_least(100.0))
            .column(Column::remainder().at_least(80.0))
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.strong(t!("common.col.category").as_ref());
                });
                header.col(|ui| {
                    ui.strong(t!("common.field.status").as_ref());
                });
                header.col(|ui| {
                    ui.strong(t!("common.col.count").as_ref());
                });
            })
            .body(|mut body| {
                for ((cat, status), count) in &by_cat_status {
                    body.row(22.0, |mut row| {
                        row.col(|ui| {
                            ui.label(cat);
                        });
                        row.col(|ui| {
                            ui.label(status);
                        });
                        row.col(|ui| {
                            ui.label(count.to_string());
                        });
                    });
                }
            });

        ui.add_space(12.0);
        ui.label(
            egui::RichText::new(t!("reports.inventory.by_location.heading").as_ref()).italics(),
        );
        TableBuilder::new(ui)
            .id_salt("inv_location_table")
            .striped(true)
            .vscroll(false)
            .column(Column::auto().at_least(120.0))
            .column(Column::remainder().at_least(80.0))
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.strong(t!("common.field.location").as_ref());
                });
                header.col(|ui| {
                    ui.strong(t!("common.col.count").as_ref());
                });
            })
            .body(|mut body| {
                for (loc, count) in &by_location {
                    body.row(22.0, |mut row| {
                        row.col(|ui| {
                            ui.label(loc);
                        });
                        row.col(|ui| {
                            ui.label(count.to_string());
                        });
                    });
                }
            });

        self.show_inventory_item_log(ui);
    }

    fn show_inventory_item_log(&self, ui: &mut egui::Ui) {
        ui.add_space(14.0);
        ui.separator();
        ui.add_space(6.0);
        ui.label(egui::RichText::new(t!("reports.inventory.log.heading").as_ref()).strong());
        ui.weak(t!("reports.inventory.log.hint").as_ref());
        ui.add_space(6.0);

        let mut rows: Vec<&InventoryItemRow> = self.inventory_rows.iter().collect();
        // None sorts before Some, so an item with a missing source join (shouldn't
        // happen given the FK, see InventoryItemRow::acquired_date) would land first
        // rather than last — an intentional "surface it, don't hide it" choice, not
        // an oversight.
        rows.sort_by(|a, b| a.acquired_date.cmp(&b.acquired_date).then(a.id.cmp(&b.id)));

        TableBuilder::new(ui)
            .id_salt("inventory_item_log_table")
            .striped(true)
            .vscroll(false)
            .column(Column::auto().at_least(140.0))
            .column(Column::auto().at_least(120.0))
            .column(Column::auto().at_least(90.0))
            .column(Column::auto().at_least(80.0))
            .column(Column::remainder().at_least(160.0))
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.strong(t!("common.col.name").as_ref());
                });
                header.col(|ui| {
                    ui.strong(t!("common.col.category").as_ref());
                });
                header.col(|ui| {
                    ui.strong(t!("common.field.status").as_ref());
                });
                header.col(|ui| {
                    ui.strong(t!("common.field.location").as_ref());
                });
                header.col(|ui| {
                    ui.strong(t!("common.field.source").as_ref());
                });
            })
            .body(|mut body| {
                for item in rows {
                    body.row(18.0, |mut row| {
                        row.col(|ui| {
                            ui.label(&item.name);
                        });
                        row.col(|ui| {
                            ui.label(&item.category_name);
                        });
                        row.col(|ui| {
                            ui.label(item.status.label());
                        });
                        row.col(|ui| {
                            ui.label(item.location.label());
                        });
                        row.col(|ui| {
                            ui.label(&item.source_desc);
                        });
                    });
                }
            });
    }

    fn show_outbound_summary(&self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new(t!("reports.tab.outbound").as_ref()).strong());
        ui.add_space(6.0);

        let filtered: Vec<&OutboundEventRow> = self
            .outbound_rows
            .iter()
            .filter(|e| in_range(&e.date, &self.date_from_iso, &self.date_to_iso))
            .filter(|e| {
                self.recipient_filter
                    .is_none_or(|rid| e.recipient_project_id == rid)
            })
            .collect();

        if filtered.is_empty() {
            ui.weak(t!("reports.outbound.empty").as_ref());
        } else {
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

            ui.label(
                egui::RichText::new(t!("reports.outbound.by_recipient.heading").as_ref()).italics(),
            );
            TableBuilder::new(ui)
                .id_salt("outbound_by_recipient_table")
                .striped(true)
                .vscroll(false)
                .column(Column::auto().at_least(160.0))
                .column(Column::auto().at_least(80.0))
                .column(Column::auto().at_least(80.0))
                .column(Column::remainder().at_least(100.0))
                .header(20.0, |mut header| {
                    header.col(|ui| {
                        ui.strong(t!("reports.col.recipient").as_ref());
                    });
                    header.col(|ui| {
                        ui.strong(t!("reports.col.events").as_ref());
                    });
                    header.col(|ui| {
                        ui.strong(t!("reports.col.items").as_ref());
                    });
                    header.col(|ui| {
                        ui.strong(t!("reports.outbound.col.cash").as_ref());
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
                                ui.label(format::amount(*cash));
                            });
                        });
                    }
                });

            ui.add_space(12.0);
            ui.label(egui::RichText::new(t!("reports.col.events").as_ref()).italics());
            TableBuilder::new(ui)
                .id_salt("outbound_events_table")
                .striped(true)
                .vscroll(false)
                .column(Column::auto().at_least(90.0))
                .column(Column::auto().at_least(140.0))
                .column(Column::auto().at_least(60.0))
                .column(Column::remainder().at_least(90.0))
                .header(20.0, |mut header| {
                    header.col(|ui| {
                        ui.strong(t!("common.col.date").as_ref());
                    });
                    header.col(|ui| {
                        ui.strong(t!("reports.col.recipient").as_ref());
                    });
                    header.col(|ui| {
                        ui.strong(t!("reports.col.items").as_ref());
                    });
                    header.col(|ui| {
                        ui.strong(t!("reports.outbound.col.cash").as_ref());
                    });
                })
                .body(|mut body| {
                    for e in &filtered {
                        body.row(22.0, |mut row| {
                            row.col(|ui| {
                                ui.label(format::date(&e.date));
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
                                    format::amount(cash)
                                } else {
                                    "—".to_string()
                                });
                            });
                        });
                    }
                });
        }

        self.show_outbound_history(ui);
    }

    fn show_outbound_history(&self, ui: &mut egui::Ui) {
        ui.add_space(14.0);
        ui.separator();
        ui.add_space(6.0);
        ui.label(egui::RichText::new(t!("reports.outbound.history.heading").as_ref()).strong());
        ui.weak(t!("reports.outbound.history.hint").as_ref());
        ui.add_space(6.0);

        if self.outbound_rows.is_empty() {
            ui.weak(t!("reports.outbound.history.empty").as_ref());
            return;
        }

        let mut events: Vec<&OutboundEventRow> = self.outbound_rows.iter().collect();
        events.sort_by(|a, b| a.date.cmp(&b.date).then(a.id.cmp(&b.id)));

        TableBuilder::new(ui)
            .id_salt("outbound_history_table")
            .striped(true)
            .vscroll(false)
            .column(Column::auto().at_least(90.0))
            .column(Column::auto().at_least(140.0))
            .column(Column::auto().at_least(80.0))
            .column(Column::remainder().at_least(160.0))
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.strong(t!("common.col.date").as_ref());
                });
                header.col(|ui| {
                    ui.strong(t!("reports.col.recipient").as_ref());
                });
                header.col(|ui| {
                    ui.strong(t!("reports.outbound.col.cash").as_ref());
                });
                header.col(|ui| {
                    ui.strong(t!("reports.outbound.history.col.items_given").as_ref());
                });
            })
            .body(|mut body| {
                for e in events {
                    let cash = e.cash_amount_brl.unwrap_or(Decimal::ZERO);
                    let items = self
                        .outbound_item_names
                        .get(&e.id)
                        .map(|names| names.join(", "))
                        .unwrap_or_default();
                    body.row(18.0, |mut row| {
                        row.col(|ui| {
                            ui.label(format::date(&e.date));
                        });
                        row.col(|ui| {
                            ui.label(&e.recipient_name);
                        });
                        row.col(|ui| {
                            if cash > Decimal::ZERO {
                                ui.label(format::amount(cash));
                            }
                        });
                        row.col(|ui| {
                            ui.label(items);
                        });
                    });
                }
            });
    }

    fn show_audit_trail(&self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new(t!("reports.audit.heading").as_ref()).strong());
        ui.weak(t!("reports.audit.hint").as_ref());
        ui.add_space(6.0);

        let entries = self.build_audit_entries();

        if entries.is_empty() {
            ui.weak(t!("reports.audit.empty").as_ref());
            return;
        }

        TableBuilder::new(ui)
            .id_salt("audit_trail_table")
            .striped(true)
            .vscroll(false)
            .column(Column::auto().at_least(90.0))
            .column(Column::auto().at_least(60.0))
            .column(Column::auto().at_least(110.0))
            .column(Column::auto().at_least(200.0))
            .column(Column::auto().at_least(90.0))
            .column(Column::remainder().at_least(60.0))
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.strong(t!("common.col.date").as_ref());
                });
                header.col(|ui| {
                    ui.strong(t!("reports.audit.col.ledger").as_ref());
                });
                header.col(|ui| {
                    ui.strong(t!("reports.col.type").as_ref());
                });
                header.col(|ui| {
                    ui.strong(t!("common.col.description").as_ref());
                });
                header.col(|ui| {
                    ui.strong(t!("common.col.amount").as_ref());
                });
                header.col(|ui| {
                    ui.strong(t!("reports.audit.col.docs").as_ref());
                });
            })
            .body(|mut body| {
                let locale = rust_i18n::locale().to_string();
                for e in &entries {
                    let (ledger, kind, description) = match &e.outbound {
                        Some(info) => outbound_audit_text(info, &locale, format::amount),
                        None => {
                            let (ledger, kind) = e.ledger_kind.clone().unwrap_or_default();
                            (ledger, kind, e.description.clone())
                        }
                    };
                    body.row(22.0, |mut row| {
                        row.col(|ui| {
                            ui.label(format::date(&e.date));
                        });
                        row.col(|ui| {
                            ui.label(&ledger);
                        });
                        row.col(|ui| {
                            ui.label(&kind);
                        });
                        row.col(|ui| {
                            ui.label(&description);
                        });
                        row.col(|ui| {
                            if let Some(a) = &e.amount {
                                ui.label(format!(
                                    "{}{}{}",
                                    a.sign,
                                    a.symbol,
                                    format::amount(a.value)
                                ));
                            }
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
    /// Builds the Donor Breakdown export data in an explicit target
    /// `locale`, per SPEC.md §6.3 — never the ambient UI locale. `for_csv`
    /// additionally pins amounts to the fixed German format CSV always uses
    /// (SPEC.md §6.4/T6), regardless of `locale`; PDF export (`for_csv =
    /// false`) instead follows `locale` for both text and numbers.
    fn csv_data_donors(&self, locale: &str, for_csv: bool) -> (Vec<String>, Vec<Vec<String>>) {
        let headers = vec![
            t!("common.field.donor", locale = locale).into_owned(),
            t!("reports.donor.col.cash_count", locale = locale).into_owned(),
            t!("reports.donor.csv.col.cash_total", locale = locale).into_owned(),
            t!("reports.donor.col.items", locale = locale).into_owned(),
        ];
        let rows = self
            .build_donor_rows()
            .into_iter()
            .map(|r| {
                let cash_total = if for_csv {
                    format::csv_amount(r.cash_total)
                } else {
                    format::amount_in(r.cash_total, locale)
                };
                vec![
                    r.name,
                    r.cash_count.to_string(),
                    cash_total,
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
                        && in_range(&r.date, &self.date_from_iso, &self.date_to_iso)
                })
                .collect();
            let item_count = self
                .donations
                .iter()
                .filter(|d| {
                    d.donor_id == Some(donor.id)
                        && in_range(&d.date_received, &self.date_from_iso, &self.date_to_iso)
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
                    && in_range(&r.date, &self.date_from_iso, &self.date_to_iso)
            })
            .collect();
        let anon_items = self
            .donations
            .iter()
            .filter(|d| {
                d.donor_id.is_none()
                    && in_range(&d.date_received, &self.date_from_iso, &self.date_to_iso)
            })
            .count() as i64;
        if !anon_cash.is_empty() || anon_items > 0 {
            let cash_total: Decimal = anon_cash.iter().map(|r| r.amount).sum();
            rows.push(DonorRow {
                name: t!("common.anonymous").into_owned(),
                cash_count: anon_cash.len() as i64,
                cash_total,
                item_count: anon_items,
            });
        }
        rows
    }

    fn csv_data_eur(&self, locale: &str, for_csv: bool) -> (Vec<String>, Vec<Vec<String>>) {
        let headers = vec![
            t!("common.col.date", locale = locale).into_owned(),
            t!("reports.col.type", locale = locale).into_owned(),
            t!("common.col.description", locale = locale).into_owned(),
            t!("reports.eur.csv.col.amount", locale = locale).into_owned(),
        ];
        let rows = self
            .eur_rows
            .iter()
            .filter(|r| in_range(&r.date, &self.date_from_iso, &self.date_to_iso))
            .map(|r| {
                let description = eur_tx_description(r);
                let sign = if r.tx_type.is_inflow() { "" } else { "-" };
                let date = if for_csv {
                    r.date.clone()
                } else {
                    format::date_in(&r.date, locale)
                };
                let amount = if for_csv {
                    format::csv_amount(r.amount)
                } else {
                    format::amount_in(r.amount, locale)
                };
                vec![
                    date,
                    r.tx_type.label(),
                    description,
                    format!("{sign}{amount}"),
                ]
            })
            .collect();
        (headers, rows)
    }

    fn csv_data_brl(&self, locale: &str, for_csv: bool) -> (Vec<String>, Vec<Vec<String>>) {
        let headers = vec![
            t!("common.col.date", locale = locale).into_owned(),
            t!("reports.col.type", locale = locale).into_owned(),
            t!("common.col.description", locale = locale).into_owned(),
            t!("reports.brl.csv.col.amount", locale = locale).into_owned(),
        ];
        let rows = self
            .brl_rows
            .iter()
            .filter(|r| in_range(&r.date, &self.date_from_iso, &self.date_to_iso))
            .map(|r| {
                let description = brl_tx_description(r);
                let sign = if r.tx_type.is_inflow() { "" } else { "-" };
                let date = if for_csv {
                    r.date.clone()
                } else {
                    format::date_in(&r.date, locale)
                };
                let amount = if for_csv {
                    format::csv_amount(r.amount)
                } else {
                    format::amount_in(r.amount, locale)
                };
                vec![
                    date,
                    r.tx_type.label(),
                    description,
                    format!("{sign}{amount}"),
                ]
            })
            .collect();
        (headers, rows)
    }

    // No amounts to format, so no for_csv branching needed — only the
    // headers vary by locale. Row *values* (status/location/source_type)
    // still resolve through the enum `.label()` methods' ambient UI locale,
    // not `locale`, a known scope limit (see the module-level note above
    // `build_pdf_sections` for why this wasn't threaded further).
    fn csv_data_inventory(&self, locale: &str) -> (Vec<String>, Vec<Vec<String>>) {
        let headers = vec![
            t!("common.col.name", locale = locale).into_owned(),
            t!("common.col.category", locale = locale).into_owned(),
            t!("common.field.status", locale = locale).into_owned(),
            t!("common.field.location", locale = locale).into_owned(),
            t!("reports.inventory.csv.col.source_type", locale = locale).into_owned(),
            t!("common.field.source", locale = locale).into_owned(),
        ];
        let rows = self
            .inventory_rows
            .iter()
            .map(|item| {
                let source_type = match item.source_type {
                    SourceType::Donation => t!("status.source_type.donation"),
                    SourceType::Purchase => t!("status.source_type.purchase"),
                };
                vec![
                    item.name.clone(),
                    item.category_name.clone(),
                    item.status.label(),
                    item.location.label(),
                    source_type.into_owned(),
                    item.source_desc.clone(),
                ]
            })
            .collect();
        (headers, rows)
    }

    fn csv_data_outbound(&self, locale: &str, for_csv: bool) -> (Vec<String>, Vec<Vec<String>>) {
        let headers = vec![
            t!("common.col.date", locale = locale).into_owned(),
            t!("reports.col.recipient", locale = locale).into_owned(),
            t!("reports.col.items", locale = locale).into_owned(),
            t!("reports.outbound.csv.col.cash", locale = locale).into_owned(),
        ];
        let rows = self
            .outbound_rows
            .iter()
            .filter(|e| in_range(&e.date, &self.date_from_iso, &self.date_to_iso))
            .filter(|e| {
                self.recipient_filter
                    .is_none_or(|rid| e.recipient_project_id == rid)
            })
            .map(|e| {
                let date = if for_csv {
                    e.date.clone()
                } else {
                    format::date_in(&e.date, locale)
                };
                let cash = e
                    .cash_amount_brl
                    .map(|c| {
                        if for_csv {
                            format::csv_amount(c)
                        } else {
                            format::amount_in(c, locale)
                        }
                    })
                    .unwrap_or_default();
                vec![
                    date,
                    e.recipient_name.clone(),
                    e.item_count.to_string(),
                    cash,
                ]
            })
            .collect();
        (headers, rows)
    }

    fn csv_data_audit_trail(&self, locale: &str, for_csv: bool) -> (Vec<String>, Vec<Vec<String>>) {
        let headers = vec![
            t!("common.col.date", locale = locale).into_owned(),
            t!("reports.audit.col.ledger", locale = locale).into_owned(),
            t!("reports.col.type", locale = locale).into_owned(),
            t!("common.col.description", locale = locale).into_owned(),
            t!("common.col.amount", locale = locale).into_owned(),
            t!("reports.audit.csv.col.documents", locale = locale).into_owned(),
        ];
        let fmt_amount = |v: Decimal| {
            if for_csv {
                format::csv_amount(v)
            } else {
                format::amount_in(v, locale)
            }
        };
        let rows = self
            .build_audit_entries()
            .into_iter()
            .map(|e| {
                let date = if for_csv {
                    e.date
                } else {
                    format::date_in(&e.date, locale)
                };
                let (ledger, kind, description) = match &e.outbound {
                    Some(info) => outbound_audit_text(info, locale, fmt_amount),
                    None => {
                        let (ledger, kind) = e.ledger_kind.unwrap_or_default();
                        (ledger, kind, e.description)
                    }
                };
                vec![
                    date,
                    ledger,
                    kind,
                    description,
                    e.amount
                        .map(|a| format!("{}{}{}", a.sign, a.symbol, fmt_amount(a.value)))
                        .unwrap_or_default(),
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

    /// Builds all PDF sections in an explicit target `locale` (SPEC.md
    /// §6.3) — the operator's choice in the export dialog, never the
    /// ambient UI locale. Unlike CSV, PDF numbers follow `locale` too
    /// (`for_csv = false` on every shared `csv_data_*` call).
    fn build_pdf_sections(&self, locale: &str) -> Vec<crate::reports::pdf::PdfSection> {
        let (dh, dr) = self.csv_data_donors(locale, false);
        let (eh, er) = self.csv_data_eur(locale, false);
        let (bh, br) = self.csv_data_brl(locale, false);
        let (ih, ir) = self.csv_data_inventory(locale);
        let (oh, or_) = self.csv_data_outbound(locale, false);
        let (ah, ar) = self.csv_data_audit_trail(locale, false);
        vec![
            crate::reports::pdf::PdfSection {
                title: t!("reports.pdf.section.donor", locale = locale).into_owned(),
                headers: dh,
                rows: dr,
            },
            crate::reports::pdf::PdfSection {
                title: t!("reports.pdf.section.eur", locale = locale).into_owned(),
                headers: eh,
                rows: er,
            },
            crate::reports::pdf::PdfSection {
                title: t!("reports.pdf.section.brl", locale = locale).into_owned(),
                headers: bh,
                rows: br,
            },
            crate::reports::pdf::PdfSection {
                title: t!("sidebar.inventory", locale = locale).into_owned(),
                headers: ih,
                rows: ir,
            },
            crate::reports::pdf::PdfSection {
                title: t!("reports.pdf.section.outbound", locale = locale).into_owned(),
                headers: oh,
                rows: or_,
            },
            crate::reports::pdf::PdfSection {
                title: t!("reports.pdf.section.audit", locale = locale).into_owned(),
                headers: ah,
                rows: ar,
            },
        ]
    }

    fn build_audit_entries(&self) -> Vec<AuditEntry> {
        let mut entries: Vec<AuditEntry> = Vec::new();

        for r in &self.eur_rows {
            if !in_range(&r.date, &self.date_from_iso, &self.date_to_iso) {
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
            let description = eur_tx_description(r);
            let sign = if r.tx_type.is_inflow() { "+" } else { "-" };
            entries.push(AuditEntry {
                date: r.date.clone(),
                ledger_kind: Some(("EUR".to_string(), r.tx_type.label())),
                description,
                outbound: None,
                amount: Some(AuditAmount {
                    sign,
                    symbol: "€",
                    value: r.amount,
                }),
                docs,
            });
        }

        for r in &self.brl_rows {
            if !in_range(&r.date, &self.date_from_iso, &self.date_to_iso) {
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
            let description = brl_tx_description(r);
            let sign = if r.tx_type.is_inflow() { "+" } else { "-" };
            entries.push(AuditEntry {
                date: r.date.clone(),
                ledger_kind: Some(("BRL".to_string(), r.tx_type.label())),
                description,
                outbound: None,
                amount: Some(AuditAmount {
                    sign,
                    symbol: "R$",
                    value: r.amount,
                }),
                docs,
            });
        }

        for e in &self.outbound_rows {
            if !in_range(&e.date, &self.date_from_iso, &self.date_to_iso) {
                continue;
            }
            if let Some(rid) = self.recipient_filter {
                if e.recipient_project_id != rid {
                    continue;
                }
            }
            let outbound_cash = e.cash_amount_brl.filter(|&cash| cash > Decimal::ZERO);
            entries.push(AuditEntry {
                date: e.date.clone(),
                ledger_kind: None,
                description: String::new(),
                outbound: Some(OutboundAuditInfo {
                    item_count: e.item_count,
                    recipient_name: e.recipient_name.clone(),
                    cash: outbound_cash,
                }),
                amount: None,
                docs: 0,
            });
        }

        entries.sort_by(|a, b| b.date.cmp(&a.date));
        entries
    }
}

/// Report-language picker shown inside the CSV/PDF save dialogs (SPEC.md
/// §6.3) — defaults to the ambient UI locale (set when the export dialog
/// is opened) but is an explicit, independent choice from then on, same as
/// the language selector in Settings. A free function, not a method: it's
/// called from inside a block that already holds `self.csv_path_input`/
/// `self.pdf_path_input` borrowed, so it must take `export_locale`
/// directly rather than `&mut self` to keep the borrows disjoint.
fn show_export_locale_picker(ui: &mut egui::Ui, export_locale: &mut String, id_salt: &str) {
    let current_label = format::LOCALES
        .iter()
        .find(|(code, _)| *code == export_locale.as_str())
        .map(|(_, label)| *label)
        .unwrap_or(export_locale.as_str())
        .to_string();
    ui.horizontal(|ui| {
        ui.label(t!("settings.locale.field.language").as_ref());
        egui::ComboBox::from_id_salt(id_salt)
            .selected_text(current_label)
            .show_ui(ui, |ui| {
                for (code, label) in format::LOCALES {
                    ui.selectable_value(export_locale, code.to_string(), *label);
                }
            });
    });
}

fn in_range(date: &str, from: &str, to: &str) -> bool {
    (from.is_empty() || date >= from) && (to.is_empty() || date <= to)
}

/// Normalizes a typed date-range boundary (may be "DD.MM.YYYY", ISO, empty,
/// or garbage) to ISO for comparison against stored `.date` fields, which
/// always stay ISO. Empty input normalizes to "" (in_range's "no bound"
/// sentinel). Unparseable non-empty input also degrades to "no bound" on
/// that side rather than blocking the whole report — the caller is
/// responsible for surfacing the returned `bool` as a visible error so an
/// invalid boundary doesn't silently widen the report instead of erroring.
fn normalize_filter_date(raw: &str) -> (String, bool) {
    let t = raw.trim();
    if t.is_empty() {
        return (String::new(), false);
    }
    match crate::date::parse_date_input(t) {
        Some(d) => (d.format("%Y-%m-%d").to_string(), false),
        None => (String::new(), true),
    }
}

fn donor_or_anonymous(name: &Option<String>) -> String {
    name.clone()
        .unwrap_or_else(|| t!("common.anonymous").into_owned())
}

fn eur_tx_description(r: &EurTxRow) -> String {
    match r.tx_type {
        EurTxType::DonationIn => r
            .donor_name
            .clone()
            .unwrap_or_else(|| t!("common.anonymous").into_owned()),
        EurTxType::SelfFundingIn => r.note.clone().unwrap_or_default(),
        EurTxType::PurchaseOut => r.purchase_channel.clone().unwrap_or_default(),
        EurTxType::TransferToBrlOut => t!("reports.tx.eur_to_brl_transfer").into_owned(),
    }
}

fn brl_tx_description(r: &BrlTxRow) -> String {
    match r.tx_type {
        BrlTxType::TransferIn => t!("reports.tx.eur_to_brl_transfer").into_owned(),
        BrlTxType::BrazilPurchaseOut => r.purchase_channel.clone().unwrap_or_default(),
        BrlTxType::CashGiftOut => r.recipient_name.clone().unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::outbound::OutboundEventRow;

    fn outbound_fixture() -> ReportsView {
        ReportsView {
            outbound_rows: vec![OutboundEventRow {
                id: 1,
                date: "2026-01-15".to_string(),
                recipient_project_id: 1,
                recipient_name: "Projeto Teste".to_string(),
                cash_amount_brl: Some(Decimal::new(500, 2)),
                notes: None,
                item_count: 2,
            }],
            ..Default::default()
        }
    }

    // Regression test for a bug caught in code review: an outbound row's
    // Ledger/Type/Description text used to be baked once, at whichever UI
    // locale happened to be ambient when build_audit_entries ran, instead
    // of following the export's explicitly chosen locale like every other
    // field. Deliberately does not touch rust_i18n::set_locale (global,
    // shared across parallel test threads) — csv_data_audit_trail must
    // produce correct output from the `locale` argument alone.
    #[test]
    fn csv_audit_trail_outbound_row_follows_the_explicit_export_locale() {
        let view = outbound_fixture();

        let (_headers, rows) = view.csv_data_audit_trail("de", true);
        assert_eq!(rows.len(), 1);
        let row = &rows[0];

        assert_eq!(row[1], "Ausgehend", "ledger column must be German");
        assert_eq!(row[2], "Spende", "type column must be German");
        assert!(row[3].contains("Projeto Teste"));
        assert!(row[3].contains("2 Artikel"), "description: {}", row[3]);
        assert!(row[3].contains("Geldspende"), "cash suffix: {}", row[3]);
        assert!(
            !row[3].contains("items"),
            "must not leak English: {}",
            row[3]
        );
        assert!(
            !row[3].contains("cash gift"),
            "must not leak English: {}",
            row[3]
        );
    }

    #[test]
    fn pdf_audit_trail_outbound_row_also_follows_the_explicit_export_locale() {
        let view = outbound_fixture();

        // for_csv = false is the PDF path — same shared function, must not
        // silently fall back to CSV's fixed formatting or the ambient locale.
        let (_headers, rows) = view.csv_data_audit_trail("pt-BR", false);
        assert_eq!(rows.len(), 1);
        let row = &rows[0];

        assert_eq!(row[1], "Saídas", "ledger column must be Portuguese");
        assert_eq!(row[2], "Doação", "type column must be Portuguese");
        assert!(row[3].contains("Projeto Teste"));
        assert!(row[3].contains("2 itens"), "description: {}", row[3]);
        assert!(row[3].contains("em doação"), "cash suffix: {}", row[3]);
    }
}
