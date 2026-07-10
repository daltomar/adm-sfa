use std::path::Path;

use eframe::egui;
use rusqlite::Connection;

use crate::db::queries::{categories as cat_qry, documents as docs_qry};
use crate::model::category::Category;

pub struct SettingsView {
    categories: Vec<Category>,
    labels: Vec<(i64, String)>,
    needs_reload: bool,

    cat_new_name: String,
    cat_editing: Option<(i64, String)>,
    cat_error: Option<String>,

    lbl_new_name: String,
    lbl_editing: Option<(i64, String)>,
    lbl_error: Option<String>,

    backup_path_input: Option<String>,
    backup_status: Option<Result<String, String>>,
}

impl Default for SettingsView {
    fn default() -> Self {
        Self {
            categories: Vec::new(),
            labels: Vec::new(),
            needs_reload: true,
            cat_new_name: String::new(),
            cat_editing: None,
            cat_error: None,
            lbl_new_name: String::new(),
            lbl_editing: None,
            lbl_error: None,
            backup_path_input: None,
            backup_status: None,
        }
    }
}

impl SettingsView {
    pub fn invalidate(&mut self) {
        self.needs_reload = true;
    }

    pub fn show(&mut self, ui: &mut egui::Ui, db: &Connection, data_dir: &Path) {
        if self.needs_reload {
            match (cat_qry::list(db), docs_qry::list_labels(db)) {
                (Ok(cats), Ok(lbls)) => {
                    self.categories = cats;
                    self.labels = lbls;
                    self.needs_reload = false;
                    self.cat_editing = None;
                    self.lbl_editing = None;
                }
                (Err(e), _) => self.cat_error = Some(e.to_string()),
                (_, Err(e)) => self.lbl_error = Some(e.to_string()),
            }
        }

        ui.heading("Settings");
        ui.add_space(8.0);

        egui::ScrollArea::vertical()
            .id_salt("settings_scroll")
            .show(ui, |ui| {
                self.show_categories_panel(ui, db);
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(8.0);
                self.show_labels_panel(ui, db);
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(8.0);
                self.show_backup_panel(ui, data_dir);
            });
    }

    fn show_categories_panel(&mut self, ui: &mut egui::Ui, db: &Connection) {
        ui.label(egui::RichText::new("Item Categories").strong());
        ui.add_space(4.0);
        ui.weak("Used as a classification for inventory items. Cannot delete a category that is assigned to an item.");
        ui.add_space(6.0);

        let cat_ids_names: Vec<(i64, String)> = self
            .categories
            .iter()
            .map(|c| (c.id, c.name.clone()))
            .collect();
        let editing_id = self.cat_editing.as_ref().map(|(id, _)| *id);
        let was_editing = editing_id.is_some();
        let mut edit_draft = self
            .cat_editing
            .as_ref()
            .map(|(_, d)| d.clone())
            .unwrap_or_default();

        enum CatAction {
            None,
            StartEdit(i64, String),
            CancelEdit,
            SaveEdit(i64),
            Delete(i64),
            Add,
        }
        let mut action = CatAction::None;

        for (id, name) in &cat_ids_names {
            let id = *id;
            if editing_id == Some(id) {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut edit_draft).desired_width(200.0),
                    );
                    let ok = !edit_draft.trim().is_empty();
                    if ui.add_enabled(ok, egui::Button::new("Save")).clicked() {
                        action = CatAction::SaveEdit(id);
                    }
                    if ui.button("Cancel").clicked() {
                        action = CatAction::CancelEdit;
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    ui.label(name);
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if ui.small_button("Delete").clicked() {
                                action = CatAction::Delete(id);
                            }
                            if ui.small_button("Rename").clicked() {
                                action = CatAction::StartEdit(id, name.clone());
                            }
                        },
                    );
                });
            }
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("New:");
            ui.add(
                egui::TextEdit::singleline(&mut self.cat_new_name)
                    .hint_text("category name")
                    .desired_width(180.0),
            );
            if ui
                .add_enabled(
                    !self.cat_new_name.trim().is_empty(),
                    egui::Button::new("Add"),
                )
                .clicked()
            {
                action = CatAction::Add;
            }
        });

        if let Some(err) = &self.cat_error {
            ui.add_space(4.0);
            ui.colored_label(egui::Color32::RED, err);
        }

        match action {
            CatAction::None => {}
            CatAction::StartEdit(id, name) => {
                self.cat_editing = Some((id, name));
                self.cat_error = None;
            }
            CatAction::CancelEdit => {
                self.cat_editing = None;
                self.cat_error = None;
            }
            CatAction::SaveEdit(id) => {
                if edit_draft.trim().is_empty() {
                    self.cat_error = Some("Name cannot be empty.".to_string());
                } else {
                    match cat_qry::update(db, id, &edit_draft) {
                        Ok(()) => {
                            self.cat_editing = None;
                            self.needs_reload = true;
                            self.cat_error = None;
                        }
                        Err(e) => self.cat_error = Some(e.to_string()),
                    }
                }
            }
            CatAction::Delete(id) => {
                match cat_qry::in_use(db, id) {
                    Err(e) => self.cat_error = Some(e.to_string()),
                    Ok(true) => {
                        self.cat_error = Some(
                            "This category is in use by one or more inventory items and cannot be deleted.".to_string(),
                        );
                    }
                    Ok(false) => match cat_qry::delete(db, id) {
                        Ok(()) => {
                            if self.cat_editing.as_ref().map(|(eid, _)| *eid) == Some(id) {
                                self.cat_editing = None;
                            }
                            self.needs_reload = true;
                            self.cat_error = None;
                        }
                        Err(e) => self.cat_error = Some(e.to_string()),
                    },
                }
            }
            CatAction::Add => {
                let name = self.cat_new_name.trim().to_string();
                match cat_qry::insert(db, &name) {
                    Ok(_) => {
                        self.cat_new_name.clear();
                        self.needs_reload = true;
                        self.cat_error = None;
                    }
                    Err(e) => self.cat_error = Some(e.to_string()),
                }
            }
        }

        if was_editing {
            if let Some((_, ref mut d)) = self.cat_editing {
                *d = edit_draft;
            }
        }
    }

    fn show_labels_panel(&mut self, ui: &mut egui::Ui, db: &Connection) {
        ui.label(egui::RichText::new("Document Labels").strong());
        ui.add_space(4.0);
        ui.weak("Labels used when attaching documents to purchases, transfers, and items. Renaming or deleting a label does not affect documents already filed under that name.");
        ui.add_space(6.0);

        let lbl_ids_names: Vec<(i64, String)> = self.labels.clone();
        let editing_id = self.lbl_editing.as_ref().map(|(id, _)| *id);
        let was_editing = editing_id.is_some();
        let mut edit_draft = self
            .lbl_editing
            .as_ref()
            .map(|(_, d)| d.clone())
            .unwrap_or_default();

        enum LblAction {
            None,
            StartEdit(i64, String),
            CancelEdit,
            SaveEdit(i64),
            Delete(i64),
            Add,
        }
        let mut action = LblAction::None;

        for (id, name) in &lbl_ids_names {
            let id = *id;
            if editing_id == Some(id) {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut edit_draft).desired_width(200.0),
                    );
                    let ok = !edit_draft.trim().is_empty();
                    if ui.add_enabled(ok, egui::Button::new("Save")).clicked() {
                        action = LblAction::SaveEdit(id);
                    }
                    if ui.button("Cancel").clicked() {
                        action = LblAction::CancelEdit;
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    ui.label(name);
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if ui.small_button("Delete").clicked() {
                                action = LblAction::Delete(id);
                            }
                            if ui.small_button("Rename").clicked() {
                                action = LblAction::StartEdit(id, name.clone());
                            }
                        },
                    );
                });
            }
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("New:");
            ui.add(
                egui::TextEdit::singleline(&mut self.lbl_new_name)
                    .hint_text("label name")
                    .desired_width(180.0),
            );
            if ui
                .add_enabled(
                    !self.lbl_new_name.trim().is_empty(),
                    egui::Button::new("Add"),
                )
                .clicked()
            {
                action = LblAction::Add;
            }
        });

        if let Some(err) = &self.lbl_error {
            ui.add_space(4.0);
            ui.colored_label(egui::Color32::RED, err);
        }

        match action {
            LblAction::None => {}
            LblAction::StartEdit(id, name) => {
                self.lbl_editing = Some((id, name));
                self.lbl_error = None;
            }
            LblAction::CancelEdit => {
                self.lbl_editing = None;
                self.lbl_error = None;
            }
            LblAction::SaveEdit(id) => {
                if edit_draft.trim().is_empty() {
                    self.lbl_error = Some("Name cannot be empty.".to_string());
                } else {
                    match docs_qry::update_label(db, id, &edit_draft) {
                        Ok(()) => {
                            self.lbl_editing = None;
                            self.needs_reload = true;
                            self.lbl_error = None;
                        }
                        Err(e) => self.lbl_error = Some(e.to_string()),
                    }
                }
            }
            LblAction::Delete(id) => match docs_qry::delete_label(db, id) {
                Ok(()) => {
                    if self.lbl_editing.as_ref().map(|(eid, _)| *eid) == Some(id) {
                        self.lbl_editing = None;
                    }
                    self.needs_reload = true;
                    self.lbl_error = None;
                }
                Err(e) => self.lbl_error = Some(e.to_string()),
            },
            LblAction::Add => {
                let name = self.lbl_new_name.trim().to_string();
                match docs_qry::insert_label(db, &name) {
                    Ok(_) => {
                        self.lbl_new_name.clear();
                        self.needs_reload = true;
                        self.lbl_error = None;
                    }
                    Err(e) => self.lbl_error = Some(e.to_string()),
                }
            }
        }

        if was_editing {
            if let Some((_, ref mut d)) = self.lbl_editing {
                *d = edit_draft;
            }
        }
    }

    fn show_backup_panel(&mut self, ui: &mut egui::Ui, data_dir: &Path) {
        ui.label(egui::RichText::new("Backup").strong());
        ui.add_space(4.0);
        ui.weak("Zips the database and all documents into a single archive.");
        ui.add_space(6.0);

        if ui.button("Backup Now").clicked() {
            self.backup_status = None;
            let default_name = format!(
                "adm-sfa-backup-{}.zip",
                chrono::Local::now().format("%Y-%m-%d")
            );
            let default_path = dirs::download_dir()
                .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from(".")))
                .join(default_name)
                .to_string_lossy()
                .into_owned();
            self.backup_path_input = Some(default_path);
        }

        let mut save = false;
        let mut cancel = false;
        if let Some(ref mut path_str) = self.backup_path_input {
            ui.group(|ui| {
                ui.label("Save backup to:");
                ui.add(egui::TextEdit::singleline(path_str).desired_width(500.0));
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        save = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        }
        if save {
            let path_str = self
                .backup_path_input
                .as_deref()
                .unwrap_or("")
                .trim()
                .to_string();
            let path = std::path::PathBuf::from(&path_str);
            if path.as_os_str().is_empty() {
                self.backup_status = Some(Err("Please enter a file path.".to_string()));
            } else {
                self.backup_path_input = None;
                self.backup_status = Some(
                    crate::backup::backup_to_zip(data_dir, &path)
                        .map(|()| format!("Saved to {}", path.display()))
                        .map_err(|e| e.to_string()),
                );
            }
        } else if cancel {
            self.backup_path_input = None;
        }

        if let Some(status) = &self.backup_status {
            ui.add_space(4.0);
            match status {
                Ok(msg) => {
                    ui.colored_label(egui::Color32::DARK_GREEN, msg);
                }
                Err(msg) => {
                    ui.colored_label(egui::Color32::RED, format!("Backup failed: {msg}"));
                }
            };
        }
    }
}
