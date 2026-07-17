use eframe::egui;
use rusqlite::Connection;
use rust_i18n::t;
use std::path::{Path, PathBuf};

use crate::db::queries::{
    categories as cat_qry, documents as docs_qry, donors as donors_qry, inventory as qry,
    purchases as purchases_qry, settings as settings_qry,
};
use crate::docs_fs;
use crate::format;
use crate::model::category::Category;
use crate::model::document::Document;
use crate::model::donor::{DonorDraft, PhysicalDonation, PhysicalDonationDraft};
use crate::model::inventory::{
    InventoryItemDraft, InventoryItemRow, ItemStatus, Location, SourceType,
};
use crate::model::purchase::{Purchase, PurchaseStatus};

enum Mode {
    List,
    Adding,
    Editing(i64),
}

struct PendingAttachment {
    path: PathBuf,
    label: String,
    error: Option<String>,
    /// True if `path` is a temp file this app created (a screenshot capture)
    /// rather than a user's own file — deleted once no longer needed instead
    /// of left in the OS temp dir.
    is_temp: bool,
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
    new_donor: Option<DonorDraft>,

    docs: Vec<Document>,
    labels: Vec<String>,
    docs_needs_reload: bool,
    pending_doc: Option<PendingAttachment>,
    path_input: Option<String>,
    capture_note: Option<String>,
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
            new_donor: None,
            docs: Vec::new(),
            labels: Vec::new(),
            docs_needs_reload: false,
            pending_doc: None,
            path_input: None,
            capture_note: None,
        }
    }
}

impl InventoryView {
    /// Clears any in-progress attachment, deleting the source file first if
    /// it was a screenshot capture (a temp file this app made) rather than
    /// a file the user picked — otherwise capture temp files pile up in the
    /// OS temp dir every time a form is reset before confirming the attach.
    fn discard_pending_doc(&mut self) {
        if let Some(p) = self.pending_doc.take() {
            if p.is_temp {
                let _ = std::fs::remove_file(&p.path);
            }
        }
    }

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
                    ui.weak(t!("inventory.hint.select_or_add").as_ref());
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
        ui.heading(t!("sidebar.inventory").as_ref());
        ui.add_space(4.0);

        if ui.button(t!("inventory.button.add").as_ref()).clicked() {
            self.draft = InventoryItemDraft::default();
            self.mode = Mode::Adding;
            self.error = None;
            self.docs = Vec::new();
            self.discard_pending_doc();
            self.path_input = None;
            self.capture_note = None;
            self.new_donation = None;
            self.new_donor = None;
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
                    ui.weak(t!("inventory.empty").as_ref());
                    return;
                }
                for i in 0..self.items.len() {
                    let item = &self.items[i];
                    let id = item.id;
                    let row = t!(
                        "inventory.row",
                        name = item.name,
                        category = item.category_name,
                        location = item.location.label(),
                        status = item.status.label(),
                        source = item.source_desc
                    )
                    .into_owned();
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
                        self.discard_pending_doc();
                        self.path_input = None;
                        self.capture_note = None;
                        self.new_donation = None;
                        self.new_donor = None;
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

        let heading = if is_adding {
            t!("inventory.heading.new")
        } else {
            t!("inventory.heading.edit")
        };
        ui.heading(heading.as_ref());
        ui.add_space(8.0);

        egui::Grid::new("inventory_form_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .min_col_width(90.0)
            .show(ui, |ui| {
                ui.label(t!("common.field.name").as_ref());
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.name)
                        .hint_text(t!("inventory.field.name_hint").as_ref())
                        .desired_width(280.0),
                );
                ui.end_row();

                ui.label(t!("inventory.field.category").as_ref());
                let selected_name = self
                    .draft
                    .category_id
                    .and_then(|cid| self.categories.iter().find(|c| c.id == cid))
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| t!("common.combo.choose_one").into_owned());
                egui::ComboBox::from_id_salt("inventory_category_combo")
                    .selected_text(selected_name)
                    .show_ui(ui, |ui| {
                        for c in &self.categories {
                            ui.selectable_value(&mut self.draft.category_id, Some(c.id), &c.name);
                        }
                    });
                ui.end_row();

                ui.label(t!("common.field.location").as_ref());
                ui.horizontal(|ui| {
                    ui.radio_value(
                        &mut self.draft.location,
                        Location::Germany,
                        t!("status.location.germany").as_ref(),
                    );
                    ui.radio_value(
                        &mut self.draft.location,
                        Location::Brazil,
                        t!("status.location.brazil").as_ref(),
                    );
                });
                ui.end_row();

                ui.label(t!("common.field.status").as_ref());
                ui.horizontal(|ui| {
                    ui.radio_value(
                        &mut self.draft.status,
                        ItemStatus::Available,
                        t!("status.item.available").as_ref(),
                    );
                    ui.radio_value(
                        &mut self.draft.status,
                        ItemStatus::Reserved,
                        t!("status.item.reserved").as_ref(),
                    );
                    ui.radio_value(
                        &mut self.draft.status,
                        ItemStatus::Donated,
                        t!("status.item.donated").as_ref(),
                    );
                });
                ui.end_row();

                ui.label(t!("common.field.notes").as_ref());
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
        ui.label(egui::RichText::new(t!("common.field.source").as_ref()).strong());
        ui.horizontal(|ui| {
            ui.radio_value(
                &mut self.draft.source_type,
                SourceType::Donation,
                t!("status.source_type.donation").as_ref(),
            );
            ui.radio_value(
                &mut self.draft.source_type,
                SourceType::Purchase,
                t!("status.source_type.purchase").as_ref(),
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
            if ui
                .add_enabled(form_ok, egui::Button::new(t!("common.save").as_ref()))
                .clicked()
            {
                if let Some(msg) = self.purchase_source_conflict(edit_id) {
                    self.error = Some(msg);
                } else if is_adding {
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

            if ui.button(t!("common.cancel").as_ref()).clicked() {
                self.mode = Mode::List;
                self.error = None;
                self.discard_pending_doc();
                self.path_input = None;
                self.capture_note = None;
                self.new_donation = None;
                self.new_donor = None;
            }
        });
    }

    fn show_donation_source(&mut self, ui: &mut egui::Ui, db: &Connection) {
        let selected_label = self
            .draft
            .source_donation_id
            .and_then(|did| self.donations.iter().find(|d| d.id == did))
            .map(donation_label)
            .unwrap_or_else(|| t!("common.combo.choose_one").into_owned());

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
            if ui
                .button(t!("inventory.button.new_donation").as_ref())
                .clicked()
            {
                self.new_donation = Some(PhysicalDonationDraft::default());
            }
        });

        enum Action {
            None,
            Create,
            Cancel,
        }
        let mut action = Action::None;

        enum DonorAction {
            None,
            Create,
            Cancel,
        }
        let mut donor_action = DonorAction::None;

        if let Some(nd) = &mut self.new_donation {
            let donors = self.donors.clone();
            ui.add_space(6.0);
            ui.group(|ui| {
                egui::Grid::new("new_donation_grid")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .min_col_width(80.0)
                    .show(ui, |ui| {
                        ui.label(t!("inventory.donation.field.date_received").as_ref());
                        ui.add(
                            egui::TextEdit::singleline(&mut nd.date_received)
                                .hint_text(t!("common.field.date_hint").as_ref())
                                .desired_width(140.0),
                        );
                        ui.end_row();

                        ui.label(t!("common.field.donor").as_ref());
                        ui.horizontal(|ui| {
                            let name = nd
                                .donor_id
                                .and_then(|did| donors.iter().find(|(id, _)| *id == did))
                                .map(|(_, n)| n.clone())
                                .unwrap_or_else(|| t!("inventory.combo.anonymous").into_owned());
                            egui::ComboBox::from_id_salt("new_donation_donor_combo")
                                .selected_text(name)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut nd.donor_id,
                                        None,
                                        t!("inventory.combo.anonymous").as_ref(),
                                    );
                                    for (did, dname) in &donors {
                                        ui.selectable_value(&mut nd.donor_id, Some(*did), dname);
                                    }
                                });
                            if ui
                                .button(t!("inventory.button.new_donor").as_ref())
                                .clicked()
                            {
                                self.new_donor = Some(DonorDraft::default());
                            }
                        });
                        ui.end_row();

                        ui.label(t!("common.field.notes").as_ref());
                        ui.add(
                            egui::TextEdit::multiline(&mut nd.notes)
                                .desired_width(240.0)
                                .desired_rows(2),
                        );
                        ui.end_row();
                    });

                if let Some(newd) = &mut self.new_donor {
                    ui.add_space(6.0);
                    ui.group(|ui| {
                        egui::Grid::new("new_donor_grid")
                            .num_columns(2)
                            .spacing([12.0, 6.0])
                            .min_col_width(80.0)
                            .show(ui, |ui| {
                                ui.label(t!("common.field.name").as_ref());
                                ui.add(
                                    egui::TextEdit::singleline(&mut newd.name).desired_width(220.0),
                                );
                                ui.end_row();

                                ui.label(t!("common.field.contact_info").as_ref());
                                ui.add(
                                    egui::TextEdit::singleline(&mut newd.contact_info)
                                        .desired_width(220.0),
                                );
                                ui.end_row();

                                ui.label(t!("common.field.notes").as_ref());
                                ui.add(
                                    egui::TextEdit::multiline(&mut newd.notes)
                                        .desired_width(220.0)
                                        .desired_rows(2),
                                );
                                ui.end_row();
                            });

                        let ok = !newd.name.trim().is_empty();
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(ok, egui::Button::new(t!("common.create").as_ref()))
                                .clicked()
                            {
                                donor_action = DonorAction::Create;
                            }
                            if ui.button(t!("common.cancel").as_ref()).clicked() {
                                donor_action = DonorAction::Cancel;
                            }
                        });
                    });
                }

                let ok = !nd.date_received.trim().is_empty();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(ok, egui::Button::new(t!("common.create").as_ref()))
                        .clicked()
                    {
                        action = Action::Create;
                    }
                    if ui.button(t!("common.cancel").as_ref()).clicked() {
                        action = Action::Cancel;
                    }
                });
            });
        }

        match donor_action {
            DonorAction::Cancel => self.new_donor = None,
            DonorAction::Create => {
                let draft = self.new_donor.clone().unwrap();
                match donors_qry::insert(db, &draft) {
                    Ok(new_id) => {
                        if let Some(nd) = &mut self.new_donation {
                            nd.donor_id = Some(new_id);
                        }
                        self.new_donor = None;
                        self.donors_loaded = false;
                        self.error = None;
                    }
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
            DonorAction::None => {}
        }

        match action {
            Action::Cancel => {
                self.new_donation = None;
                self.new_donor = None;
            }
            Action::Create => {
                let draft = self.new_donation.clone().unwrap();
                match donors_qry::insert_donation(db, &draft) {
                    Ok(new_id) => {
                        self.draft.source_donation_id = Some(new_id);
                        self.new_donation = None;
                        self.new_donor = None;
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
        // Negotiating purchases haven't committed any money yet, so no
        // inventory item may be created against them — exclude entirely,
        // not just grey out (see CLAUDE.md "Purchase negotiation status").
        let purchases: Vec<Purchase> = self
            .purchases
            .iter()
            .filter(|p| p.status == PurchaseStatus::Bought)
            .cloned()
            .collect();

        if purchases.is_empty() {
            ui.weak(t!("inventory.hint.no_purchases").as_ref());
            return;
        }

        let edit_id = if let Mode::Editing(id) = self.mode {
            Some(id)
        } else {
            None
        };

        // Purchases that are single-item and already linked to a *different* item.
        let blocked: std::collections::HashSet<i64> = self
            .items
            .iter()
            .filter(|item| edit_id != Some(item.id))
            .filter_map(|item| item.source_purchase_id)
            .filter(|&pid| purchases.iter().any(|p| p.id == pid && !p.multiple_items))
            .collect();

        let selected_label = self
            .draft
            .source_purchase_id
            .and_then(|pid| purchases.iter().find(|p| p.id == pid))
            .map(purchase_label)
            .unwrap_or_else(|| t!("common.combo.choose_one").into_owned());

        egui::ComboBox::from_id_salt("inventory_purchase_combo")
            .selected_text(selected_label)
            .show_ui(ui, |ui| {
                for p in &purchases {
                    if blocked.contains(&p.id) {
                        let label =
                            t!("inventory.purchase_combo.in_use", label = purchase_label(p))
                                .into_owned();
                        ui.add(egui::Label::new(
                            egui::RichText::new(&label).color(egui::Color32::from_gray(140)),
                        ));
                    } else {
                        ui.selectable_value(
                            &mut self.draft.source_purchase_id,
                            Some(p.id),
                            purchase_label(p),
                        );
                    }
                }
            });
    }

    fn purchase_source_conflict(&self, edit_id: Option<i64>) -> Option<String> {
        if self.draft.source_type != SourceType::Purchase {
            return None;
        }
        let pid = self.draft.source_purchase_id?;
        // If pid is not in the cache the FK constraint in SQLite will catch it at insert/update.
        let p = self.purchases.iter().find(|p| p.id == pid)?;
        if p.multiple_items {
            return None;
        }
        let already_used = self
            .items
            .iter()
            .filter(|item| edit_id != Some(item.id))
            .any(|item| item.source_purchase_id == Some(pid));
        if already_used {
            Some(t!("inventory.error.purchase_conflict", channel = p.channel).into_owned())
        } else {
            None
        }
    }

    fn show_documents(&mut self, ui: &mut egui::Ui, db: &Connection, data_dir: &Path) {
        let edit_id = match self.mode {
            Mode::Editing(id) => id,
            _ => return,
        };
        let documents_dir = data_dir.join("documents");

        ui.heading(t!("common.doc.heading").as_ref());
        ui.add_space(4.0);

        let mut remove_doc: Option<(i64, String)> = None;
        if self.docs.is_empty() {
            ui.weak(t!("common.doc.none_attached").as_ref());
        } else {
            for doc in &self.docs {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&doc.label).strong());
                    ui.label(&doc.filename);
                    if ui.small_button(t!("common.doc.remove").as_ref()).clicked() {
                        remove_doc = Some((doc.id, doc.filename.clone()));
                    }
                });
            }
        }

        if let Some((doc_id, filename)) = remove_doc {
            match docs_qry::soft_delete(db, doc_id) {
                Err(e) => {
                    self.error =
                        Some(t!("common.doc.error.db_update_failed", error = e).into_owned())
                }
                Ok(()) => match docs_fs::soft_delete(&documents_dir, &filename) {
                    Err(e) => {
                        self.error =
                            Some(t!("common.doc.error.file_move_failed", error = e).into_owned())
                    }
                    Ok(()) => {
                        self.docs_needs_reload = true;
                        self.error = None;
                    }
                },
            }
        }

        ui.add_space(8.0);

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
                            is_temp: false,
                        });
                    }
                }
            }
        }

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
                    ui.label(
                        t!(
                            "common.doc.file_name",
                            name = pending
                                .path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                        )
                        .into_owned(),
                    );
                    ui.horizontal(|ui| {
                        ui.label(t!("common.doc.field.label").as_ref());
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
                        if ui.button(t!("common.doc.button.attach").as_ref()).clicked() {
                            doc_action = DocAction::Confirm;
                        }
                        if ui.button(t!("common.cancel").as_ref()).clicked() {
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
                    ui.label(t!("common.doc.field.path").as_ref());
                    ui.add(
                        egui::TextEdit::singleline(path_str)
                            .hint_text(t!("common.doc.field.path_hint").as_ref())
                            .desired_width(380.0),
                    );
                    let path = PathBuf::from(path_str.trim());
                    let is_file = path.is_file();
                    if !path_str.trim().is_empty() && !is_file {
                        ui.weak(t!("common.doc.error.file_not_found").as_ref());
                    }
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                is_file,
                                egui::Button::new(t!("common.doc.button.next").as_ref()),
                            )
                            .clicked()
                        {
                            confirmed_path = Some(path);
                        }
                        if ui.button(t!("common.cancel").as_ref()).clicked() {
                            path_cancelled = true;
                        }
                    });
                });
            } else if ui
                .button(t!("common.doc.button.attach_file").as_ref())
                .clicked()
            {
                self.path_input = Some(String::new());
            }
            if self.path_input.is_none()
                && ui
                    .button(t!("common.doc.button.capture_screenshot").as_ref())
                    .clicked()
            {
                self.capture_note = None;
                self.error = None;
                match settings_qry::get(db, "screenshot_command") {
                    Err(e) => self.error = Some(e.to_string()),
                    Ok(cmd) => match crate::screenshot::capture(cmd.as_deref().unwrap_or("")) {
                        Ok(crate::screenshot::CaptureOutcome::Success(path)) => {
                            let default_label = self
                                .labels
                                .first()
                                .cloned()
                                .unwrap_or_else(|| "other".to_string());
                            self.pending_doc = Some(PendingAttachment {
                                path,
                                label: default_label,
                                error: None,
                                is_temp: true,
                            });
                        }
                        Ok(crate::screenshot::CaptureOutcome::Cancelled) => {
                            self.capture_note =
                                Some(t!("common.doc.capture_cancelled").into_owned());
                        }
                        Err(e) => self.error = Some(e),
                    },
                }
            }
            let hovering = ui.input(|i| !i.raw.hovered_files.is_empty());
            if hovering {
                ui.colored_label(
                    egui::Color32::from_rgb(80, 160, 230),
                    t!("common.doc.drop_hint").as_ref(),
                );
            } else {
                ui.weak(t!("common.doc.drag_hint").as_ref());
            }
            if let Some(note) = &self.capture_note {
                ui.weak(note);
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
                    is_temp: false,
                });
                self.path_input = None;
            } else if path_cancelled {
                self.path_input = None;
            }
        }

        match doc_action {
            DocAction::Cancel => self.discard_pending_doc(),
            DocAction::Confirm => {
                if let Some(p) = self.pending_doc.as_ref() {
                    let (path, label, is_temp) = (p.path.clone(), p.label.clone(), p.is_temp);
                    let existing: Vec<String> =
                        self.docs.iter().map(|d| d.filename.clone()).collect();
                    // Items have no single "date" field of their own; use today's date
                    // so filenames stay chronologically sortable at attach time.
                    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                    match docs_fs::file_document(
                        db,
                        &documents_dir,
                        &path,
                        &today,
                        ("item", edit_id),
                        &label,
                        &existing,
                    ) {
                        Ok(_) => {
                            if is_temp {
                                let _ = std::fs::remove_file(&path);
                            }
                            self.pending_doc = None;
                            self.docs_needs_reload = true;
                            self.error = None;
                        }
                        Err(e) => {
                            if let Some(p) = &mut self.pending_doc {
                                p.error = Some(e);
                            }
                        }
                    }
                } // if let Some(p)
            }
            DocAction::None => {}
        }
    }
}

fn donation_label(d: &PhysicalDonation) -> String {
    match &d.donor_name {
        Some(name) => t!(
            "inventory.donation_label",
            date = format::date(&d.date_received),
            name = name
        )
        .into_owned(),
        None => t!(
            "inventory.donation_label_anonymous",
            date = format::date(&d.date_received)
        )
        .into_owned(),
    }
}

fn purchase_label(p: &Purchase) -> String {
    let multi = if p.multiple_items {
        t!("purchases.tag.multi").into_owned()
    } else {
        String::new()
    };
    t!(
        "inventory.purchase_label",
        date = format::date(&p.date),
        channel = p.channel,
        symbol = p.currency.symbol(),
        cost = format::amount(p.cost),
        multi = multi
    )
    .into_owned()
}
