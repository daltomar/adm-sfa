use eframe::egui;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

use crate::db::queries::{documents as docs_qry, purchases as qry};
use crate::docs_fs;
use crate::model::document::Document;
use crate::model::purchase::{Currency, Purchase, PurchaseDraft, PurchaseStatus};

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

pub struct PurchasesView {
    purchases: Vec<Purchase>,
    mode: Mode,
    draft: PurchaseDraft,
    error: Option<String>,
    needs_reload: bool,
    docs: Vec<Document>,
    labels: Vec<String>,
    docs_needs_reload: bool,
    pending_doc: Option<PendingAttachment>,
    path_input: Option<String>,
    confirm_drop: bool,
}

impl Default for PurchasesView {
    fn default() -> Self {
        Self {
            purchases: Vec::new(),
            mode: Mode::List,
            draft: PurchaseDraft::default(),
            error: None,
            needs_reload: true,
            docs: Vec::new(),
            labels: Vec::new(),
            docs_needs_reload: false,
            pending_doc: None,
            path_input: None,
            confirm_drop: false,
        }
    }
}

impl PurchasesView {
    pub fn invalidate(&mut self) {
        self.needs_reload = true;
        self.labels.clear();
    }

    pub fn show(&mut self, ui: &mut egui::Ui, db: &Connection, data_dir: &Path) {
        if self.needs_reload {
            match qry::list(db) {
                Ok(list) => {
                    self.purchases = list;
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
                match docs_qry::list_for_record(db, "purchase", id) {
                    Ok(docs) => {
                        self.docs = docs;
                        self.docs_needs_reload = false;
                    }
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
        }

        egui::Panel::left("purchases_list_panel")
            .resizable(true)
            .default_size(280.0)
            .show(ui, |ui| self.show_list(ui));

        egui::ScrollArea::vertical()
            .id_salt("purchases_detail_scroll")
            .show(ui, |ui| match self.mode {
                Mode::List => {
                    ui.add_space(16.0);
                    ui.weak("Select a purchase, or add a new one.");
                }
                Mode::Adding | Mode::Editing(_) => {
                    self.show_form(ui, db, data_dir);
                    if matches!(self.mode, Mode::Editing(_)) {
                        ui.add_space(16.0);
                        ui.separator();
                        self.show_documents(ui, db, data_dir);
                    }
                }
            });
    }

    fn show_list(&mut self, ui: &mut egui::Ui) {
        ui.heading("Purchases");
        ui.add_space(4.0);

        if ui.button("+ Add purchase").clicked() {
            self.draft = PurchaseDraft::default();
            self.mode = Mode::Adding;
            self.error = None;
            self.docs = Vec::new();
            self.pending_doc = None;
            self.path_input = None;
            self.confirm_drop = false;
        }

        ui.separator();

        if let Some(err) = &self.error {
            ui.colored_label(egui::Color32::RED, err);
            ui.separator();
        }

        egui::ScrollArea::vertical()
            .id_salt("purchases_list_scroll")
            .show(ui, |ui| {
                if self.purchases.is_empty() {
                    ui.weak("No purchases yet.");
                    return;
                }
                for i in 0..self.purchases.len() {
                    let p = &self.purchases[i];
                    let id = p.id;
                    let multi = if p.multiple_items { "  [multi]" } else { "" };
                    let status_tag = match p.status {
                        PurchaseStatus::Negotiating => "  [negotiating]",
                        PurchaseStatus::Bought => "",
                    };
                    let row = format!(
                        "{}  {}  {} {:.2}{}{}",
                        p.date,
                        p.channel,
                        p.currency.symbol(),
                        p.cost,
                        multi,
                        status_tag
                    );
                    let selected = matches!(self.mode, Mode::Editing(eid) if eid == id);
                    if ui.selectable_label(selected, &row).clicked() {
                        self.draft = PurchaseDraft {
                            date: self.purchases[i].date.clone(),
                            currency: self.purchases[i].currency,
                            cost_str: self.purchases[i].cost.to_string(),
                            channel: self.purchases[i].channel.clone(),
                            seller_info: self.purchases[i].seller_info.clone().unwrap_or_default(),
                            multiple_items: self.purchases[i].multiple_items,
                            status: self.purchases[i].status,
                        };
                        self.mode = Mode::Editing(id);
                        self.error = None;
                        self.docs_needs_reload = true;
                        self.pending_doc = None;
                        self.path_input = None;
                        self.confirm_drop = false;
                    }
                }
            });
    }

    fn show_form(&mut self, ui: &mut egui::Ui, db: &Connection, data_dir: &Path) {
        let is_adding = matches!(self.mode, Mode::Adding);
        let edit_id: Option<i64> = if let Mode::Editing(id) = self.mode {
            Some(id)
        } else {
            None
        };

        ui.heading(if is_adding {
            "New Purchase"
        } else {
            "Edit Purchase"
        });
        ui.add_space(8.0);

        egui::Grid::new("purchase_form_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .min_col_width(90.0)
            .show(ui, |ui| {
                ui.label("Date *");
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.date)
                        .hint_text("YYYY-MM-DD")
                        .desired_width(140.0),
                );
                ui.end_row();

                ui.label("Currency");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.draft.currency, Currency::Eur, "EUR (€)");
                    ui.radio_value(&mut self.draft.currency, Currency::Brl, "BRL (R$)");
                });
                ui.end_row();

                ui.label("Cost *");
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.cost_str)
                        .hint_text("0.00")
                        .desired_width(140.0),
                );
                ui.end_row();

                ui.label("Channel *");
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.channel)
                        .hint_text("Kleinanzeigen, local market, …")
                        .desired_width(280.0),
                );
                ui.end_row();

                ui.label("Seller / Notes");
                ui.add(
                    egui::TextEdit::multiline(&mut self.draft.seller_info)
                        .hint_text("Name, address, listing URL, …")
                        .desired_width(280.0)
                        .desired_rows(4),
                );
                ui.end_row();

                ui.label("Multiple items");
                ui.checkbox(
                    &mut self.draft.multiple_items,
                    "This purchase covers more than one inventory item",
                );
                ui.end_row();

                ui.label("Status");
                if is_adding {
                    let mut negotiating = self.draft.status == PurchaseStatus::Negotiating;
                    ui.checkbox(
                        &mut negotiating,
                        "Start as negotiating — no ledger entry until marked bought",
                    );
                    self.draft.status = if negotiating {
                        PurchaseStatus::Negotiating
                    } else {
                        PurchaseStatus::Bought
                    };
                } else {
                    ui.label(match self.draft.status {
                        PurchaseStatus::Negotiating => "Negotiating",
                        PurchaseStatus::Bought => "Bought",
                    });
                }
                ui.end_row();
            });

        if let Some(err) = &self.error {
            ui.add_space(4.0);
            ui.colored_label(egui::Color32::RED, err);
        }

        let cost_text = self.draft.cost_str.trim();
        let cost_parsed = crate::money::parse_amount_input(cost_text);
        let cost_ok = cost_parsed
            .map(|d| d > rust_decimal::Decimal::ZERO)
            .unwrap_or(false);
        if !cost_text.is_empty() {
            if cost_parsed.is_none() {
                ui.colored_label(
                    egui::Color32::RED,
                    "Not a valid amount — use e.g. 12.34 or 12,34",
                );
            } else if !cost_ok {
                ui.colored_label(egui::Color32::RED, "Cost must be greater than zero");
            }
        }
        let form_ok =
            !self.draft.date.trim().is_empty() && !self.draft.channel.trim().is_empty() && cost_ok;

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
                    // Guard: disallow removing the multiple_items flag when
                    // more than one inventory item is already linked.
                    let blocked = if !self.draft.multiple_items {
                        match qry::linked_item_count(db, id) {
                            Ok(n) if n > 1 => Some(n),
                            Ok(_) => None,
                            Err(e) => {
                                self.error = Some(e.to_string());
                                return;
                            }
                        }
                    } else {
                        None
                    };
                    if let Some(n) = blocked {
                        self.error = Some(format!(
                            "Cannot mark as single-item: {n} inventory items are already linked to this purchase."
                        ));
                    } else {
                        match qry::update(db, id, &self.draft) {
                            Ok(()) => {
                                self.needs_reload = true;
                                self.error = None;
                            }
                            Err(e) => self.error = Some(e.to_string()),
                        }
                    }
                }
            }

            if ui.button("Cancel").clicked() {
                self.mode = Mode::List;
                self.error = None;
                self.pending_doc = None;
                self.path_input = None;
                self.confirm_drop = false;
            }
        });

        if !is_adding && self.draft.status == PurchaseStatus::Negotiating {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(form_ok, egui::Button::new("Mark as bought"))
                    .clicked()
                {
                    if let Some(id) = edit_id {
                        let mut bought_draft = self.draft.clone();
                        bought_draft.status = PurchaseStatus::Bought;
                        match qry::update(db, id, &bought_draft) {
                            Ok(()) => {
                                self.draft.status = PurchaseStatus::Bought;
                                self.needs_reload = true;
                                self.error = None;
                            }
                            Err(e) => self.error = Some(e.to_string()),
                        }
                    }
                }

                if self.confirm_drop {
                    ui.colored_label(egui::Color32::RED, "Delete permanently?");
                    if ui.button("Yes, delete").clicked() {
                        if let Some(id) = edit_id {
                            self.drop_negotiating_purchase(db, id, data_dir);
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_drop = false;
                    }
                } else if ui.button("Drop negotiating purchase").clicked() {
                    self.confirm_drop = true;
                }
            });
        }
    }

    /// Hard-deletes a negotiating purchase. Soft-deletes any documents
    /// already attached to it first — they follow the normal document
    /// soft-delete path, never orphaned or hard-deleted alongside the
    /// purchase row (see CLAUDE.md / SPEC.md §3.6).
    fn drop_negotiating_purchase(&mut self, db: &Connection, id: i64, data_dir: &Path) {
        let documents_dir = data_dir.join("documents");
        for doc in self.docs.clone() {
            if let Err(e) = docs_qry::soft_delete(db, doc.id) {
                self.error = Some(format!("DB update failed: {e}"));
                self.confirm_drop = false;
                // Reload so a retry only re-attempts docs not yet soft-deleted
                // (list_for_record excludes deleted=1 rows) instead of
                // re-running fs::rename on a file that's already moved.
                self.docs_needs_reload = true;
                return;
            }
            if let Err(e) = docs_fs::soft_delete(&documents_dir, &doc.filename) {
                self.error = Some(format!("File move failed: {e}"));
                self.confirm_drop = false;
                self.docs_needs_reload = true;
                return;
            }
        }
        match qry::delete(db, id) {
            Ok(()) => {
                self.mode = Mode::List;
                self.needs_reload = true;
                self.docs = Vec::new();
                self.confirm_drop = false;
                self.error = None;
            }
            Err(e) => {
                self.error = Some(e.to_string());
                self.confirm_drop = false;
                self.docs_needs_reload = true;
            }
        }
    }

    fn show_documents(&mut self, ui: &mut egui::Ui, db: &Connection, data_dir: &Path) {
        let edit_id = match self.mode {
            Mode::Editing(id) => id,
            _ => return,
        };
        let documents_dir = data_dir.join("documents");

        ui.heading("Documents");
        ui.add_space(4.0);

        // Collect which doc to remove (defer mutation until after the borrow of self.docs).
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

        // Drag-and-drop: pick up a file dropped onto the window when no attachment is in progress.
        if self.pending_doc.is_none() && self.path_input.is_none() {
            let dropped = ui.input(|i| i.raw.dropped_files.clone());
            if let Some(file) = dropped.first() {
                if let Some(path) = &file.path {
                    if path.is_file() {
                        let default_label = self
                            .labels
                            .first()
                            .cloned()
                            .unwrap_or_else(|| "other".to_string());
                        self.pending_doc = Some(PendingAttachment {
                            path: path.clone(),
                            label: default_label,
                            error: None,
                        });
                    }
                }
            }
        }

        // Pending attachment — use deferred action to avoid split-borrow on self.pending_doc.
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
                        egui::ComboBox::from_id_salt("doc_label_combo")
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
            let hovering = ui.input(|i| !i.raw.hovered_files.is_empty());
            if hovering {
                ui.colored_label(egui::Color32::from_rgb(80, 160, 230), "↓ Drop file to attach");
            } else {
                ui.weak("or drag a file onto this window");
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

        // Apply the action now that all borrows of self.pending_doc are released.
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
                        "purchase",
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
                            match docs_qry::insert(db, "purchase", edit_id, &filename, &label) {
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
