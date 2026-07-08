use eframe::egui;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

use crate::db::queries::{documents as docs_qry, transfers as qry};
use crate::docs_fs;
use crate::model::document::Document;
use crate::model::transfer::{AnnualTransfer, TransferDraft};

enum Mode {
    List,
    Adding,
    Editing(i64),
}

struct PendingAttachment {
    path: PathBuf,
    label: String,
    error: Option<String>,
}

pub struct TransfersView {
    transfers: Vec<AnnualTransfer>,
    mode: Mode,
    draft: TransferDraft,
    error: Option<String>,
    needs_reload: bool,
    docs: Vec<Document>,
    labels: Vec<String>,
    docs_needs_reload: bool,
    pending_doc: Option<PendingAttachment>,
    path_input: Option<String>,
}

impl Default for TransfersView {
    fn default() -> Self {
        Self {
            transfers: Vec::new(),
            mode: Mode::List,
            draft: TransferDraft::default(),
            error: None,
            needs_reload: true,
            docs: Vec::new(),
            labels: Vec::new(),
            docs_needs_reload: false,
            pending_doc: None,
            path_input: None,
        }
    }
}

impl TransfersView {
    pub fn invalidate(&mut self) {
        self.needs_reload = true;
    }

    pub fn show(&mut self, ui: &mut egui::Ui, db: &Connection, data_dir: &Path) {
        if self.needs_reload {
            match qry::list(db) {
                Ok(list) => {
                    self.transfers = list;
                    self.needs_reload = false;
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }

        if self.labels.is_empty() {
            match docs_qry::labels(db) {
                Ok(l) => self.labels = l,
                Err(e) => self.error = Some(e.to_string()),
            }
        }

        if self.docs_needs_reload {
            if let Mode::Editing(id) = self.mode {
                match docs_qry::list_for_record(db, "transfer", id) {
                    Ok(docs) => {
                        self.docs = docs;
                        self.docs_needs_reload = false;
                    }
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
        }

        egui::Panel::left("transfers_list_panel")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| self.show_list(ui));

        egui::ScrollArea::vertical()
            .id_salt("transfers_detail_scroll")
            .show(ui, |ui| match self.mode {
                Mode::List => {
                    ui.add_space(16.0);
                    ui.weak("Select a transfer, or add a new one.");
                }
                Mode::Adding | Mode::Editing(_) => {
                    self.show_form(ui, db);
                    if matches!(self.mode, Mode::Editing(_)) {
                        ui.add_space(16.0);
                        ui.separator();
                        self.show_documents(ui, db, data_dir);
                    }
                }
            });
    }

    fn show_list(&mut self, ui: &mut egui::Ui) {
        ui.heading("Transfers");
        ui.add_space(4.0);

        if ui.button("+ Add transfer").clicked() {
            self.draft = TransferDraft::default();
            self.mode = Mode::Adding;
            self.error = None;
            self.docs = Vec::new();
            self.pending_doc = None;
            self.path_input = None;
        }

        ui.separator();

        if let Some(err) = &self.error {
            ui.colored_label(egui::Color32::RED, err);
            ui.separator();
        }

        egui::ScrollArea::vertical()
            .id_salt("transfers_list_scroll")
            .show(ui, |ui| {
                if self.transfers.is_empty() {
                    ui.weak("No transfers yet.");
                    return;
                }
                for i in 0..self.transfers.len() {
                    let t = &self.transfers[i];
                    let id = t.id;
                    let row = format!(
                        "{}  €{:.2} → R${:.2}  (rate {:.4})",
                        t.date, t.eur_amount_sent, t.brl_amount_received, t.exchange_rate
                    );
                    let selected = matches!(self.mode, Mode::Editing(eid) if eid == id);
                    if ui.selectable_label(selected, &row).clicked() {
                        self.draft = TransferDraft {
                            date: self.transfers[i].date.clone(),
                            eur_amount_sent_str: self.transfers[i].eur_amount_sent.to_string(),
                            exchange_rate_str: self.transfers[i].exchange_rate.to_string(),
                            notes: self.transfers[i].notes.clone().unwrap_or_default(),
                        };
                        self.mode = Mode::Editing(id);
                        self.error = None;
                        self.docs_needs_reload = true;
                        self.pending_doc = None;
                        self.path_input = None;
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

        ui.heading(if is_adding {
            "New Transfer"
        } else {
            "Edit Transfer"
        });
        ui.add_space(8.0);

        egui::Grid::new("transfer_form_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .min_col_width(120.0)
            .show(ui, |ui| {
                ui.label("Date *");
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.date)
                        .hint_text("YYYY-MM-DD")
                        .desired_width(140.0),
                );
                ui.end_row();

                ui.label("EUR amount sent *");
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.eur_amount_sent_str)
                        .hint_text("0.00")
                        .desired_width(140.0),
                );
                ui.end_row();

                ui.label("Exchange rate (EUR→BRL) *");
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.exchange_rate_str)
                        .hint_text("0.0000")
                        .desired_width(140.0),
                );
                ui.end_row();

                ui.label("Notes");
                ui.add(
                    egui::TextEdit::multiline(&mut self.draft.notes)
                        .desired_width(280.0)
                        .desired_rows(3),
                );
                ui.end_row();
            });

        if let (Ok(eur), Ok(rate)) = (
            self.draft
                .eur_amount_sent_str
                .trim()
                .parse::<rust_decimal::Decimal>(),
            self.draft
                .exchange_rate_str
                .trim()
                .parse::<rust_decimal::Decimal>(),
        ) {
            ui.add_space(4.0);
            ui.label(format!("BRL amount received: R$ {:.2}", eur * rate));
        }

        if let Some(err) = &self.error {
            ui.add_space(4.0);
            ui.colored_label(egui::Color32::RED, err);
        }

        let eur_ok = self
            .draft
            .eur_amount_sent_str
            .trim()
            .parse::<rust_decimal::Decimal>()
            .map(|d| d > rust_decimal::Decimal::ZERO)
            .unwrap_or(false);
        let rate_ok = self
            .draft
            .exchange_rate_str
            .trim()
            .parse::<rust_decimal::Decimal>()
            .map(|d| d > rust_decimal::Decimal::ZERO)
            .unwrap_or(false);
        let form_ok = !self.draft.date.trim().is_empty() && eur_ok && rate_ok;

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(form_ok, egui::Button::new("Save")).clicked() {
                if is_adding {
                    match qry::insert(db, &self.draft) {
                        Ok(new_id) => {
                            self.mode = Mode::Editing(new_id);
                            self.docs_needs_reload = true;
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
                self.pending_doc = None;
                self.path_input = None;
            }
        });
    }

    fn show_documents(&mut self, ui: &mut egui::Ui, db: &Connection, data_dir: &Path) {
        let edit_id = match self.mode {
            Mode::Editing(id) => id,
            _ => return,
        };
        let documents_dir = data_dir.join("documents");

        ui.heading("Documents");
        ui.add_space(4.0);

        let mut remove_doc: Option<(i64, String)> = None;
        if self.docs.is_empty() {
            ui.weak("No documents attached yet.");
        } else {
            for doc in &self.docs {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&doc.label).strong());
                    ui.label(&doc.filename);
                    if ui.small_button("Remove").clicked() {
                        remove_doc = Some((doc.id, doc.filename.clone()));
                    }
                });
            }
        }

        if let Some((doc_id, filename)) = remove_doc {
            match docs_qry::soft_delete(db, doc_id) {
                Err(e) => self.error = Some(format!("DB update failed: {e}")),
                Ok(()) => match docs_fs::soft_delete(&documents_dir, &filename) {
                    Err(e) => self.error = Some(format!("File move failed: {e}")),
                    Ok(()) => {
                        self.docs_needs_reload = true;
                        self.error = None;
                    }
                },
            }
        }

        ui.add_space(8.0);

        enum DocAction {
            None,
            Confirm,
            Cancel,
        }
        let mut doc_action = DocAction::None;

        if self.pending_doc.is_some() {
            let labels = self.labels.clone();
            if let Some(pending) = &mut self.pending_doc {
                ui.group(|ui| {
                    ui.label(format!(
                        "File: {}",
                        pending
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                    ));
                    ui.horizontal(|ui| {
                        ui.label("Label:");
                        egui::ComboBox::from_id_salt("transfer_doc_label_combo")
                            .selected_text(&pending.label)
                            .show_ui(ui, |ui| {
                                for lbl in &labels {
                                    ui.selectable_value(&mut pending.label, lbl.clone(), lbl);
                                }
                            });
                    });
                    if let Some(err) = &pending.error {
                        ui.colored_label(egui::Color32::RED, err);
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Attach").clicked() {
                            doc_action = DocAction::Confirm;
                        }
                        if ui.button("Cancel").clicked() {
                            doc_action = DocAction::Cancel;
                        }
                    });
                });
            }
        } else {
            let mut confirmed_path: Option<PathBuf> = None;
            let mut path_cancelled = false;
            if let Some(ref mut path_str) = self.path_input {
                ui.group(|ui| {
                    ui.label("File path:");
                    ui.add(
                        egui::TextEdit::singleline(path_str)
                            .hint_text("/home/user/scan.pdf")
                            .desired_width(380.0),
                    );
                    let path = PathBuf::from(path_str.trim());
                    let is_file = path.is_file();
                    if !path_str.trim().is_empty() && !is_file {
                        ui.weak("File not found.");
                    }
                    ui.horizontal(|ui| {
                        if ui.add_enabled(is_file, egui::Button::new("Next →")).clicked() {
                            confirmed_path = Some(path);
                        }
                        if ui.button("Cancel").clicked() {
                            path_cancelled = true;
                        }
                    });
                });
            } else if ui.button("Attach file…").clicked() {
                self.path_input = Some(String::new());
            }
            if let Some(path) = confirmed_path {
                let default_label = self
                    .labels
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "other".to_string());
                self.pending_doc = Some(PendingAttachment {
                    path,
                    label: default_label,
                    error: None,
                });
                self.path_input = None;
            } else if path_cancelled {
                self.path_input = None;
            }
        }

        match doc_action {
            DocAction::Cancel => self.pending_doc = None,
            DocAction::Confirm => {
                if let Some(p) = self.pending_doc.as_ref() {
                    let (path, label) = (p.path.clone(), p.label.clone());
                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("bin")
                        .to_lowercase();
                    let existing: Vec<String> =
                        self.docs.iter().map(|d| d.filename.clone()).collect();
                    let filename = docs_fs::generate_filename(
                        &self.draft.date,
                        "transfer",
                        edit_id,
                        &label,
                        &existing,
                        &ext,
                    );
                    match docs_fs::copy_to_documents(&path, &documents_dir, &filename) {
                        Err(e) => {
                            if let Some(p) = &mut self.pending_doc {
                                p.error = Some(format!("Copy failed: {e}"));
                            }
                        }
                        Ok(()) => {
                            match docs_qry::insert(db, "transfer", edit_id, &filename, &label) {
                                Ok(()) => {
                                    self.pending_doc = None;
                                    self.docs_needs_reload = true;
                                    self.error = None;
                                }
                                Err(e) => {
                                    if let Some(p) = &mut self.pending_doc {
                                        p.error = Some(format!("DB insert failed: {e}"));
                                    }
                                }
                            }
                        }
                    }
                } // if let Some(p)
            }
            DocAction::None => {}
        }
    }
}
