use std::path::Path;

use eframe::egui;
use rusqlite::Connection;
use rust_i18n::t;

use crate::db::queries::{categories as cat_qry, documents as docs_qry, settings as settings_qry};
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

    screenshot_command: String,
    screenshot_error: Option<String>,
    screenshot_status: Option<Result<String, String>>,

    locale_error: Option<String>,
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
            screenshot_command: String::new(),
            screenshot_error: None,
            screenshot_status: None,
            locale_error: None,
        }
    }
}

impl SettingsView {
    pub fn invalidate(&mut self) {
        self.needs_reload = true;
    }

    pub fn show(&mut self, ui: &mut egui::Ui, db: &Connection, data_dir: &Path) {
        if self.needs_reload {
            match (
                cat_qry::list(db),
                docs_qry::list_labels(db),
                settings_qry::get(db, "screenshot_command"),
            ) {
                (Ok(cats), Ok(lbls), Ok(cmd)) => {
                    self.categories = cats;
                    self.labels = lbls;
                    self.screenshot_command = cmd.unwrap_or_default();
                    self.needs_reload = false;
                    self.cat_editing = None;
                    self.lbl_editing = None;
                }
                (Err(e), _, _) => self.cat_error = Some(e.to_string()),
                (_, Err(e), _) => self.lbl_error = Some(e.to_string()),
                (_, _, Err(e)) => self.screenshot_error = Some(e.to_string()),
            }
        }

        ui.heading(t!("settings.heading").as_ref());
        ui.add_space(8.0);

        egui::ScrollArea::vertical()
            .id_salt("settings_scroll")
            .show(ui, |ui| {
                self.show_locale_panel(ui, db);
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(8.0);
                self.show_categories_panel(ui, db);
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(8.0);
                self.show_labels_panel(ui, db);
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(8.0);
                self.show_screenshot_panel(ui, db);
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(8.0);
                self.show_backup_panel(ui, data_dir);
            });
    }

    /// Language selector (SPEC.md §6.1/§6.2). Switches live via
    /// `rust_i18n::set_locale` — no restart, no separate Save button, since
    /// SPEC.md requires the change take effect immediately. Language names
    /// are shown as their own endonyms ("Deutsch", not a translation of
    /// "German") — the universal convention for language pickers, and not
    /// routed through `t!()` for the same reason donor/recipient names
    /// aren't: they're proper nouns, not UI chrome.
    fn show_locale_panel(&mut self, ui: &mut egui::Ui, db: &Connection) {
        use crate::format::LOCALES;

        ui.label(egui::RichText::new(t!("settings.locale.heading").as_ref()).strong());
        ui.add_space(4.0);
        ui.weak(t!("settings.locale.hint").as_ref());
        ui.add_space(6.0);

        let current = rust_i18n::locale().to_string();
        let current_label = LOCALES
            .iter()
            .find(|(code, _)| *code == current)
            .map(|(_, label)| *label)
            .unwrap_or(current.as_str());

        let mut selected = current.clone();
        ui.horizontal(|ui| {
            ui.label(t!("settings.locale.field.language").as_ref());
            egui::ComboBox::from_id_salt("ui_locale_combo")
                .selected_text(current_label)
                .show_ui(ui, |ui| {
                    for (code, label) in LOCALES {
                        ui.selectable_value(&mut selected, code.to_string(), *label);
                    }
                });
        });

        if selected != current {
            match settings_qry::set(db, "ui_locale", &selected) {
                Ok(()) => {
                    rust_i18n::set_locale(&selected);
                    self.locale_error = None;
                }
                Err(e) => self.locale_error = Some(e.to_string()),
            }
        }

        if let Some(err) = &self.locale_error {
            ui.add_space(4.0);
            ui.colored_label(egui::Color32::RED, err);
        }
    }

    fn show_categories_panel(&mut self, ui: &mut egui::Ui, db: &Connection) {
        ui.label(egui::RichText::new(t!("settings.category.heading").as_ref()).strong());
        ui.add_space(4.0);
        ui.weak(t!("settings.category.hint").as_ref());
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
                    ui.add(egui::TextEdit::singleline(&mut edit_draft).desired_width(200.0));
                    let ok = !edit_draft.trim().is_empty();
                    if ui
                        .add_enabled(ok, egui::Button::new(t!("common.save").as_ref()))
                        .clicked()
                    {
                        action = CatAction::SaveEdit(id);
                    }
                    if ui.button(t!("common.cancel").as_ref()).clicked() {
                        action = CatAction::CancelEdit;
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    ui.label(name);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button(t!("common.delete").as_ref()).clicked() {
                            action = CatAction::Delete(id);
                        }
                        if ui.small_button(t!("common.rename").as_ref()).clicked() {
                            action = CatAction::StartEdit(id, name.clone());
                        }
                    });
                });
            }
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(t!("settings.label.new").as_ref());
            ui.add(
                egui::TextEdit::singleline(&mut self.cat_new_name)
                    .hint_text(t!("settings.category.hint.new").as_ref())
                    .desired_width(180.0),
            );
            if ui
                .add_enabled(
                    !self.cat_new_name.trim().is_empty(),
                    egui::Button::new(t!("common.add").as_ref()),
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
                    self.cat_error = Some(t!("settings.error.name_required").into_owned());
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
            CatAction::Delete(id) => match cat_qry::in_use(db, id) {
                Err(e) => self.cat_error = Some(e.to_string()),
                Ok(true) => {
                    self.cat_error = Some(t!("settings.category.error.in_use").into_owned());
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
            },
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
        ui.label(egui::RichText::new(t!("settings.label.heading").as_ref()).strong());
        ui.add_space(4.0);
        ui.weak(t!("settings.label.hint").as_ref());
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
                    ui.add(egui::TextEdit::singleline(&mut edit_draft).desired_width(200.0));
                    let ok = !edit_draft.trim().is_empty();
                    if ui
                        .add_enabled(ok, egui::Button::new(t!("common.save").as_ref()))
                        .clicked()
                    {
                        action = LblAction::SaveEdit(id);
                    }
                    if ui.button(t!("common.cancel").as_ref()).clicked() {
                        action = LblAction::CancelEdit;
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    ui.label(name);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button(t!("common.delete").as_ref()).clicked() {
                            action = LblAction::Delete(id);
                        }
                        if ui.small_button(t!("common.rename").as_ref()).clicked() {
                            action = LblAction::StartEdit(id, name.clone());
                        }
                    });
                });
            }
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(t!("settings.label.new").as_ref());
            ui.add(
                egui::TextEdit::singleline(&mut self.lbl_new_name)
                    .hint_text(t!("settings.label.hint.new").as_ref())
                    .desired_width(180.0),
            );
            if ui
                .add_enabled(
                    !self.lbl_new_name.trim().is_empty(),
                    egui::Button::new(t!("common.add").as_ref()),
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
                    self.lbl_error = Some(t!("settings.error.name_required").into_owned());
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

    fn show_screenshot_panel(&mut self, ui: &mut egui::Ui, db: &Connection) {
        ui.label(egui::RichText::new(t!("settings.screenshot.heading").as_ref()).strong());
        ui.add_space(4.0);
        ui.weak(t!("settings.screenshot.hint").as_ref());
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.label(t!("settings.screenshot.field.command").as_ref());
            ui.add(
                egui::TextEdit::singleline(&mut self.screenshot_command)
                    .hint_text(t!("settings.screenshot.field.command_hint").as_ref())
                    .desired_width(400.0),
            );
        });

        let is_empty = self.screenshot_command.trim().is_empty();
        let has_placeholder = self.screenshot_command.contains("{path}");
        if !is_empty && !has_placeholder {
            ui.colored_label(
                egui::Color32::RED,
                t!("settings.screenshot.error.missing_placeholder").as_ref(),
            );
        }

        // Blank is allowed to save (clears/disables capture) — only a
        // non-empty command without the placeholder is rejected.
        if ui
            .add_enabled(
                is_empty || has_placeholder,
                egui::Button::new(t!("common.save").as_ref()),
            )
            .clicked()
        {
            match settings_qry::set(db, "screenshot_command", self.screenshot_command.trim()) {
                Ok(()) => {
                    self.screenshot_status =
                        Some(Ok(t!("settings.screenshot.status.saved").into_owned()))
                }
                Err(e) => self.screenshot_status = Some(Err(e.to_string())),
            }
        }

        if let Some(err) = &self.screenshot_error {
            ui.add_space(4.0);
            ui.colored_label(egui::Color32::RED, err);
        }
        if let Some(status) = &self.screenshot_status {
            ui.add_space(4.0);
            match status {
                Ok(msg) => {
                    ui.colored_label(egui::Color32::DARK_GREEN, msg);
                }
                Err(msg) => {
                    ui.colored_label(
                        egui::Color32::RED,
                        t!("settings.screenshot.status.save_failed", msg = msg).into_owned(),
                    );
                }
            };
        }
    }

    fn show_backup_panel(&mut self, ui: &mut egui::Ui, data_dir: &Path) {
        ui.label(egui::RichText::new(t!("settings.backup.heading").as_ref()).strong());
        ui.add_space(4.0);
        ui.weak(t!("settings.backup.hint").as_ref());
        ui.add_space(6.0);

        if ui.button(t!("settings.backup.button").as_ref()).clicked() {
            self.backup_status = None;
            let default_name = format!(
                "adm-sfa-backup-{}.zip",
                chrono::Local::now().format("%Y-%m-%d")
            );
            let default_path = dirs::download_dir()
                .unwrap_or_else(|| {
                    dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."))
                })
                .join(default_name)
                .to_string_lossy()
                .into_owned();
            self.backup_path_input = Some(default_path);
        }

        let mut save = false;
        let mut cancel = false;
        if let Some(ref mut path_str) = self.backup_path_input {
            ui.group(|ui| {
                ui.label(t!("settings.backup.field.save_to").as_ref());
                ui.add(egui::TextEdit::singleline(path_str).desired_width(500.0));
                ui.horizontal(|ui| {
                    if ui.button(t!("common.save").as_ref()).clicked() {
                        save = true;
                    }
                    if ui.button(t!("common.cancel").as_ref()).clicked() {
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
                self.backup_status = Some(Err(t!("common.error.path_required").into_owned()));
            } else {
                self.backup_path_input = None;
                self.backup_status = Some(
                    crate::backup::backup_to_zip(data_dir, &path)
                        .map(|()| t!("common.status.saved_to", path = path.display()).into_owned())
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
                    ui.colored_label(
                        egui::Color32::RED,
                        t!("settings.backup.status.failed", msg = msg).into_owned(),
                    );
                }
            };
        }
    }
}
