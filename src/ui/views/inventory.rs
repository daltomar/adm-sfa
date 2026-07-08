use eframe::egui;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

use crate::db::queries::{
    categories as cat_qry, documents as docs_qry, donors as donors_qry, inventory as qry,
    purchases as purchases_qry,
};
use crate::docs_fs;
use crate::model::category::Category;
use crate::model::document::Document;
use crate::model::donor::{PhysicalDonation, PhysicalDonationDraft};
use crate::model::inventory::{
    InventoryItemDraft, InventoryItemRow, ItemStatus, Location, SourceType,
};
use crate::model::purchase::Purchase;

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

pub struct InventoryView {
    items: Vec<InventoryItemRow>,
    mode: Mode,
    draft: InventoryItemDraft,
    error: Option<String>,
    needs_reload: bool,

    categories: Vec<Category>,
    categories_loaded: bool,

    donations: Vec<PhysicalDonation>,
    donations_loaded: bool,

    purchases: Vec<Purchase>,
    purchases_loaded: bool,

    donors: Vec<(i64, String)>,
    donors_loaded: bool,

    new_donation: Option<PhysicalDonationDraft>,

    docs: Vec<Document>,
    labels: Vec<String>,
    docs_needs_reload: bool,
    pending_doc: Option<PendingAttachment>,
    path_input: Option<String>,
}

impl Default for InventoryView {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            mode: Mode::List,
            draft: InventoryItemDraft::default(),
            error: None,
            needs_reload: true,
            categories: Vec::new(),
            categories_loaded: false,
            donations: Vec::new(),
            donations_loaded: false,
            purchases: Vec::new(),
            purchases_loaded: false,
            donors: Vec::new(),
            donors_loaded: false,
            new_donation: None,
            docs: Vec::new(),
            labels: Vec::new(),
            docs_needs_reload: false,
            pending_doc: None,
            path_input: None,
        }
    }
}

impl InventoryView {
    pub fn invalidate(&mut self) {
        self.needs_reload = true;
        self.purchases_loaded = false;
        self.donations_loaded = false;
        self.donors_loaded = false;
        self.labels.clear();
    }

    pub fn show(&mut self, ui: &mut egui::Ui, db: &Connection, data_dir: &Path) {
        if self.needs_reload {
            match qry::list(db) {
                Ok(list) => {
                    self.items = list;
                    self.needs_reload = false;
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }

        if !self.categories_loaded {
            match cat_qry::list(db) {
                Ok(list) => {
                    self.categories = list;
                    self.categories_loaded = true;
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }

        if !self.donations_loaded {
            match donors_qry::list_donations(db) {
                Ok(list) => {
                    self.donations = list;
                    self.donations_loaded = true;
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }

        if !self.purchases_loaded {
            match purchases_qry::list(db) {
                Ok(list) => {
                    self.purchases = list;
                    self.purchases_loaded = true;
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

        if self.labels.is_empty() {
            match docs_qry::labels(db) {
                Ok(l) => self.labels = l,
                Err(e) => self.error = Some(e.to_string()),
            }
        }

        if self.docs_needs_reload {
            if let Mode::Editing(id) = self.mode {
                match docs_qry::list_for_record(db, "item", id) {
                    Ok(docs) => {
                        self.docs = docs;
                        self.docs_needs_reload = false;
                    }
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
        }

        egui::Panel::left("inventory_list_panel")
            .resizable(true)
            .default_size(320.0)
            .show(ui, |ui| self.show_list(ui));

        egui::ScrollArea::vertical()
            .id_salt("inventory_detail_scroll")
            .show(ui, |ui| match self.mode {
                Mode::List => {
                    ui.add_space(16.0);
                    ui.weak("Select an item, or add a new one.");
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
        ui.heading("Inventory");
        ui.add_space(4.0);

        if ui.button("+ Add item").clicked() {
            self.draft = InventoryItemDraft::default();
            self.mode = Mode::Adding;
            self.error = None;
            self.docs = Vec::new();
            self.pending_doc = None;
            self.path_input = None;
            self.new_donation = None;
            self.purchases_loaded = false;
            self.donations_loaded = false;
            self.donors_loaded = false;
        }

        ui.separator();

        if let Some(err) = &self.error {
            ui.colored_label(egui::Color32::RED, err);
            ui.separator();
        }

        egui::ScrollArea::vertical()
            .id_salt("inventory_list_scroll")
            .show(ui, |ui| {
                if self.items.is_empty() {
                    ui.weak("No items yet.");
                    return;
                }
                for i in 0..self.items.len() {
                    let item = &self.items[i];
                    let id = item.id;
                    let row = format!(
                        "{}  [{}]  {} · {}  ({})",
                        item.name,
                        item.category_name,
                        item.location.label(),
                        item.status.label(),
                        item.source_desc,
                    );
                    let selected = matches!(self.mode, Mode::Editing(eid) if eid == id);
                    if ui.selectable_label(selected, &row).clicked() {
                        let item = &self.items[i];
                        self.draft = InventoryItemDraft {
                            name: item.name.clone(),
                            category_id: Some(item.category_id),
                            source_type: item.source_type,
                            source_donation_id: item.source_donation_id,
                            source_purchase_id: item.source_purchase_id,
                            location: item.location,
                            status: item.status,
                            notes: item.notes.clone().unwrap_or_default(),
                        };
                        self.mode = Mode::Editing(id);
                        self.error = None;
                        self.docs_needs_reload = true;
                        self.pending_doc = None;
                        self.path_input = None;
                        self.new_donation = None;
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

        ui.heading(if is_adding { "New Item" } else { "Edit Item" });
        ui.add_space(8.0);

        egui::Grid::new("inventory_form_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .min_col_width(90.0)
            .show(ui, |ui| {
                ui.label("Name *");
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.name)
                        .hint_text("Complete skateboard, Helmet size M, …")
                        .desired_width(280.0),
                );
                ui.end_row();

                ui.label("Category *");
                let selected_name = self
                    .draft
                    .category_id
                    .and_then(|cid| self.categories.iter().find(|c| c.id == cid))
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| "(choose one)".to_string());
                egui::ComboBox::from_id_salt("inventory_category_combo")
                    .selected_text(selected_name)
                    .show_ui(ui, |ui| {
                        for c in &self.categories {
                            ui.selectable_value(&mut self.draft.category_id, Some(c.id), &c.name);
                        }
                    });
                ui.end_row();

                ui.label("Location");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.draft.location, Location::Germany, "Germany");
                    ui.radio_value(&mut self.draft.location, Location::Brazil, "Brazil");
                });
                ui.end_row();

                ui.label("Status");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.draft.status, ItemStatus::Available, "Available");
                    ui.radio_value(&mut self.draft.status, ItemStatus::Reserved, "Reserved");
                    ui.radio_value(&mut self.draft.status, ItemStatus::Donated, "Donated");
                });
                ui.end_row();

                ui.label("Notes");
                ui.add(
                    egui::TextEdit::multiline(&mut self.draft.notes)
                        .desired_width(280.0)
                        .desired_rows(3),
                );
                ui.end_row();
            });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Source").strong());
        ui.horizontal(|ui| {
            ui.radio_value(
                &mut self.draft.source_type,
                SourceType::Donation,
                "Donation",
            );
            ui.radio_value(
                &mut self.draft.source_type,
                SourceType::Purchase,
                "Purchase",
            );
        });
        ui.add_space(4.0);

        match self.draft.source_type {
            SourceType::Donation => self.show_donation_source(ui, db),
            SourceType::Purchase => self.show_purchase_source(ui),
        }

        if let Some(err) = &self.error {
            ui.add_space(4.0);
            ui.colored_label(egui::Color32::RED, err);
        }

        let source_ok = match self.draft.source_type {
            SourceType::Donation => self.draft.source_donation_id.is_some(),
            SourceType::Purchase => self.draft.source_purchase_id.is_some(),
        };
        let form_ok =
            !self.draft.name.trim().is_empty() && self.draft.category_id.is_some() && source_ok;

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
                self.new_donation = None;
            }
        });
    }

    fn show_donation_source(&mut self, ui: &mut egui::Ui, db: &Connection) {
        let selected_label = self
            .draft
            .source_donation_id
            .and_then(|did| self.donations.iter().find(|d| d.id == did))
            .map(donation_label)
            .unwrap_or_else(|| "(choose one)".to_string());

        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("inventory_donation_combo")
                .selected_text(selected_label)
                .show_ui(ui, |ui| {
                    for d in &self.donations {
                        ui.selectable_value(
                            &mut self.draft.source_donation_id,
                            Some(d.id),
                            donation_label(d),
                        );
                    }
                });
            if ui.button("+ New donation").clicked() {
                self.new_donation = Some(PhysicalDonationDraft::default());
            }
        });

        enum Action {
            None,
            Create,
            Cancel,
        }
        let mut action = Action::None;

        if let Some(nd) = &mut self.new_donation {
            let donors = self.donors.clone();
            ui.add_space(6.0);
            ui.group(|ui| {
                egui::Grid::new("new_donation_grid")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .min_col_width(80.0)
                    .show(ui, |ui| {
                        ui.label("Date received *");
                        ui.add(
                            egui::TextEdit::singleline(&mut nd.date_received)
                                .hint_text("YYYY-MM-DD")
                                .desired_width(140.0),
                        );
                        ui.end_row();

                        ui.label("Donor");
                        let name = nd
                            .donor_id
                            .and_then(|did| donors.iter().find(|(id, _)| *id == did))
                            .map(|(_, n)| n.clone())
                            .unwrap_or_else(|| "(anonymous)".to_string());
                        egui::ComboBox::from_id_salt("new_donation_donor_combo")
                            .selected_text(name)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut nd.donor_id, None, "(anonymous)");
                                for (did, dname) in &donors {
                                    ui.selectable_value(&mut nd.donor_id, Some(*did), dname);
                                }
                            });
                        ui.end_row();

                        ui.label("Notes");
                        ui.add(
                            egui::TextEdit::multiline(&mut nd.notes)
                                .desired_width(240.0)
                                .desired_rows(2),
                        );
                        ui.end_row();
                    });

                let ok = !nd.date_received.trim().is_empty();
                ui.horizontal(|ui| {
                    if ui.add_enabled(ok, egui::Button::new("Create")).clicked() {
                        action = Action::Create;
                    }
                    if ui.button("Cancel").clicked() {
                        action = Action::Cancel;
                    }
                });
            });
        }

        match action {
            Action::Cancel => self.new_donation = None,
            Action::Create => {
                let draft = self.new_donation.clone().unwrap();
                match donors_qry::insert_donation(db, &draft) {
                    Ok(new_id) => {
                        self.draft.source_donation_id = Some(new_id);
                        self.new_donation = None;
                        self.donations_loaded = false;
                        self.error = None;
                    }
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
            Action::None => {}
        }
    }

    fn show_purchase_source(&mut self, ui: &mut egui::Ui) {
        if self.purchases.is_empty() {
            ui.weak("No purchases yet — add one in the Purchases section first.");
            return;
        }
        let selected_label = self
            .draft
            .source_purchase_id
            .and_then(|pid| self.purchases.iter().find(|p| p.id == pid))
            .map(purchase_label)
            .unwrap_or_else(|| "(choose one)".to_string());
        egui::ComboBox::from_id_salt("inventory_purchase_combo")
            .selected_text(selected_label)
            .show_ui(ui, |ui| {
                for p in &self.purchases {
                    ui.selectable_value(
                        &mut self.draft.source_purchase_id,
                        Some(p.id),
                        purchase_label(p),
                    );
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
                        egui::ComboBox::from_id_salt("inventory_doc_label_combo")
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
                    // Items have no single "date" field of their own; use today's date
                    // so filenames stay chronologically sortable at attach time.
                    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                    let filename = docs_fs::generate_filename(
                        &today, "item", edit_id, &label, &existing, &ext,
                    );
                    match docs_fs::copy_to_documents(&path, &documents_dir, &filename) {
                        Err(e) => {
                            if let Some(p) = &mut self.pending_doc {
                                p.error = Some(format!("Copy failed: {e}"));
                            }
                        }
                        Ok(()) => match docs_qry::insert(db, "item", edit_id, &filename, &label) {
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
                        },
                    }
                } // if let Some(p)
            }
            DocAction::None => {}
        }
    }
}

fn donation_label(d: &PhysicalDonation) -> String {
    match &d.donor_name {
        Some(name) => format!("{} — {}", d.date_received, name),
        None => format!("{} — Anonymous", d.date_received),
    }
}

fn purchase_label(p: &Purchase) -> String {
    format!(
        "{}  {}  {}{:.2}",
        p.date,
        p.channel,
        p.currency.symbol(),
        p.cost
    )
}
