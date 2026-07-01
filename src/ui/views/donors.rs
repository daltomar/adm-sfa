use eframe::egui;
use rusqlite::Connection;

use crate::db::queries::donors as qry;
use crate::model::donor::{Donor, DonorDraft};

enum Mode {
    List,
    Adding,
    Editing(i64),
}

pub struct DonorsView {
    donors: Vec<Donor>,
    mode: Mode,
    draft: DonorDraft,
    error: Option<String>,
    needs_reload: bool,
}

impl Default for DonorsView {
    fn default() -> Self {
        Self {
            donors: Vec::new(),
            mode: Mode::List,
            draft: DonorDraft::default(),
            error: None,
            needs_reload: true,
        }
    }
}

impl DonorsView {
    pub fn show(&mut self, ui: &mut egui::Ui, db: &Connection) {
        if self.needs_reload {
            match qry::list(db) {
                Ok(donors) => {
                    self.donors = donors;
                    self.needs_reload = false;
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }

        egui::Panel::left("donors_list_panel")
            .resizable(true)
            .default_size(220.0)
            .show(ui, |ui| self.show_list(ui));

        egui::ScrollArea::vertical()
            .id_salt("donors_detail_scroll")
            .show(ui, |ui| match self.mode {
                Mode::List => {
                    ui.add_space(16.0);
                    ui.weak("Select a donor from the list, or add a new one.");
                }
                Mode::Adding | Mode::Editing(_) => self.show_form(ui, db),
            });
    }

    fn show_list(&mut self, ui: &mut egui::Ui) {
        ui.heading("Donors");
        ui.add_space(4.0);

        if ui.button("+ Add donor").clicked() {
            self.draft = DonorDraft::default();
            self.mode = Mode::Adding;
            self.error = None;
        }

        ui.separator();

        if let Some(err) = &self.error {
            ui.colored_label(egui::Color32::RED, err);
            ui.separator();
        }

        egui::ScrollArea::vertical()
            .id_salt("donors_list_scroll")
            .show(ui, |ui| {
                if self.donors.is_empty() {
                    ui.weak("No donors yet.");
                }
                for i in 0..self.donors.len() {
                    let id = self.donors[i].id;
                    let name = self.donors[i].name.clone();
                    let selected = matches!(self.mode, Mode::Editing(eid) if eid == id);
                    if ui.selectable_label(selected, &name).clicked() {
                        self.draft = DonorDraft {
                            name: self.donors[i].name.clone(),
                            contact_info: self.donors[i].contact_info.clone().unwrap_or_default(),
                            notes: self.donors[i].notes.clone().unwrap_or_default(),
                        };
                        self.mode = Mode::Editing(id);
                        self.error = None;
                    }
                }
            });
    }

    fn show_form(&mut self, ui: &mut egui::Ui, db: &Connection) {
        let is_adding = matches!(self.mode, Mode::Adding);
        let edit_id: Option<i64> = match self.mode {
            Mode::Editing(id) => Some(id),
            _ => None,
        };

        ui.heading(if is_adding { "New Donor" } else { "Edit Donor" });
        ui.add_space(8.0);

        egui::Grid::new("donor_form_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .min_col_width(80.0)
            .show(ui, |ui| {
                ui.label("Name *");
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.name)
                        .desired_width(280.0),
                );
                ui.end_row();

                ui.label("Contact");
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.contact_info)
                        .hint_text("email, phone, …")
                        .desired_width(280.0),
                );
                ui.end_row();

                ui.label("Notes");
                ui.add(
                    egui::TextEdit::multiline(&mut self.draft.notes)
                        .desired_width(280.0)
                        .desired_rows(4),
                );
                ui.end_row();
            });

        if let Some(err) = &self.error {
            ui.add_space(4.0);
            ui.colored_label(egui::Color32::RED, err);
        }

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let name_ok = !self.draft.name.trim().is_empty();
            if ui
                .add_enabled(name_ok, egui::Button::new("Save"))
                .clicked()
            {
                let result = if is_adding {
                    qry::insert(db, &self.draft).map(|_| ())
                } else if let Some(id) = edit_id {
                    qry::update(db, id, &self.draft)
                } else {
                    Ok(())
                };

                match result {
                    Ok(()) => {
                        self.mode = Mode::List;
                        self.needs_reload = true;
                        self.error = None;
                    }
                    Err(e) => self.error = Some(e.to_string()),
                }
            }

            if ui.button("Cancel").clicked() {
                self.mode = Mode::List;
                self.error = None;
            }
        });
    }
}
