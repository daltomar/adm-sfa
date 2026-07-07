use eframe::egui;
use rusqlite::Connection;
use std::collections::HashSet;

use crate::db::queries::{inventory as inventory_qry, outbound as qry};
use crate::model::inventory::InventoryItemRow;
use crate::model::outbound::{
    OutboundEventDraft, OutboundEventRow, RecipientProject, RecipientProjectDraft,
};
use rust_decimal::Decimal;

enum Mode {
    List,
    Adding,
    Editing(i64),
}

pub struct OutboundView {
    events: Vec<OutboundEventRow>,
    mode: Mode,
    draft: OutboundEventDraft,
    error: Option<String>,
    needs_reload: bool,

    recipient_projects: Vec<RecipientProject>,
    recipient_projects_loaded: bool,
    new_recipient_project: Option<RecipientProjectDraft>,

    inventory_items: Vec<InventoryItemRow>,
    inventory_loaded: bool,

    selected_item_ids: HashSet<i64>,
    items_needs_reload: bool,
}

impl Default for OutboundView {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            mode: Mode::List,
            draft: OutboundEventDraft::default(),
            error: None,
            needs_reload: true,
            recipient_projects: Vec::new(),
            recipient_projects_loaded: false,
            new_recipient_project: None,
            inventory_items: Vec::new(),
            inventory_loaded: false,
            selected_item_ids: HashSet::new(),
            items_needs_reload: false,
        }
    }
}

impl OutboundView {
    pub fn show(&mut self, ui: &mut egui::Ui, db: &Connection) {
        if self.needs_reload {
            match qry::list(db) {
                Ok(list) => {
                    self.events = list;
                    self.needs_reload = false;
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }

        if !self.recipient_projects_loaded {
            match qry::list_recipient_projects(db) {
                Ok(list) => {
                    self.recipient_projects = list;
                    self.recipient_projects_loaded = true;
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }

        if !self.inventory_loaded {
            match inventory_qry::list(db) {
                Ok(list) => {
                    self.inventory_items = list;
                    self.inventory_loaded = true;
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }

        if self.items_needs_reload {
            if let Mode::Editing(id) = self.mode {
                match qry::item_ids_for_event(db, id) {
                    Ok(ids) => {
                        self.selected_item_ids = ids.into_iter().collect();
                        self.items_needs_reload = false;
                    }
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
        }

        egui::Panel::left("outbound_list_panel")
            .resizable(true)
            .default_size(320.0)
            .show(ui, |ui| self.show_list(ui));

        egui::ScrollArea::vertical()
            .id_salt("outbound_detail_scroll")
            .show(ui, |ui| match self.mode {
                Mode::List => {
                    ui.add_space(16.0);
                    ui.weak("Select an outbound donation, or add a new one.");
                }
                Mode::Adding | Mode::Editing(_) => self.show_form(ui, db),
            });
    }

    fn show_list(&mut self, ui: &mut egui::Ui) {
        ui.heading("Outbound Donations");
        ui.add_space(4.0);

        if ui.button("+ Add outbound donation").clicked() {
            self.draft = OutboundEventDraft::default();
            self.mode = Mode::Adding;
            self.error = None;
            self.selected_item_ids = HashSet::new();
            self.new_recipient_project = None;
            // inventory_loaded is NOT reset here — the inventory hasn't changed.
            // It IS reset after a successful save, which marks items as donated.
        }

        ui.separator();

        if let Some(err) = &self.error {
            ui.colored_label(egui::Color32::RED, err);
            ui.separator();
        }

        egui::ScrollArea::vertical()
            .id_salt("outbound_list_scroll")
            .show(ui, |ui| {
                if self.events.is_empty() {
                    ui.weak("No outbound donations yet.");
                    return;
                }
                for i in 0..self.events.len() {
                    let ev = &self.events[i];
                    let id = ev.id;
                    let mut row = format!(
                        "{}  {}  {} item(s)",
                        ev.date, ev.recipient_name, ev.item_count
                    );
                    if let Some(cash) = ev.cash_amount_brl {
                        if cash > Decimal::ZERO {
                            row.push_str(&format!("  R$ {cash:.2}"));
                        }
                    }
                    let selected = matches!(self.mode, Mode::Editing(eid) if eid == id);
                    if ui.selectable_label(selected, &row).clicked() {
                        let ev = &self.events[i];
                        self.draft = OutboundEventDraft {
                            date: ev.date.clone(),
                            recipient_project_id: Some(ev.recipient_project_id),
                            cash_amount_brl_str: ev
                                .cash_amount_brl
                                .map(|d| d.to_string())
                                .unwrap_or_default(),
                            notes: ev.notes.clone().unwrap_or_default(),
                        };
                        self.mode = Mode::Editing(id);
                        self.error = None;
                        self.new_recipient_project = None;
                        self.inventory_loaded = false;
                        self.items_needs_reload = true;
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
            "New Outbound Donation"
        } else {
            "Edit Outbound Donation"
        });
        ui.add_space(8.0);

        egui::Grid::new("outbound_form_grid")
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

                ui.label("Recipient project *");
                ui.end_row();
            });

        self.show_recipient_project_picker(ui, db);

        egui::Grid::new("outbound_form_grid_2")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .min_col_width(120.0)
            .show(ui, |ui| {
                ui.label("Cash gift (BRL)");
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.cash_amount_brl_str)
                        .hint_text("0.00 (leave blank if items-only)")
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

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        self.show_item_picker(ui);

        if let Some(err) = &self.error {
            ui.add_space(4.0);
            ui.colored_label(egui::Color32::RED, err);
        }

        let cash_amount = {
            let t = self.draft.cash_amount_brl_str.trim();
            if t.is_empty() {
                Some(Decimal::ZERO)
            } else {
                t.parse::<Decimal>().ok()
            }
        };
        let cash_ok = cash_amount.is_some();
        let has_cash = cash_amount.map(|d| d > Decimal::ZERO).unwrap_or(false);
        let form_ok = !self.draft.date.trim().is_empty()
            && self.draft.recipient_project_id.is_some()
            && cash_ok
            && (has_cash || !self.selected_item_ids.is_empty());

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(form_ok, egui::Button::new("Save")).clicked() {
                let item_ids: Vec<i64> = self.selected_item_ids.iter().copied().collect();
                if is_adding {
                    match qry::insert(db, &self.draft, &item_ids) {
                        Ok(new_id) => {
                            self.mode = Mode::Editing(new_id);
                            self.needs_reload = true;
                            self.inventory_loaded = false;
                            self.error = None;
                        }
                        Err(e) => self.error = Some(e.to_string()),
                    }
                } else if let Some(id) = edit_id {
                    match qry::update(db, id, &self.draft, &item_ids) {
                        Ok(()) => {
                            self.needs_reload = true;
                            self.inventory_loaded = false;
                            self.error = None;
                        }
                        Err(e) => self.error = Some(e.to_string()),
                    }
                }
            }

            if ui.button("Cancel").clicked() {
                self.mode = Mode::List;
                self.error = None;
                self.new_recipient_project = None;
            }
        });
    }

    fn show_recipient_project_picker(&mut self, ui: &mut egui::Ui, db: &Connection) {
        let selected_label = self
            .draft
            .recipient_project_id
            .and_then(|rid| self.recipient_projects.iter().find(|p| p.id == rid))
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "(choose one)".to_string());

        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("outbound_recipient_combo")
                .selected_text(selected_label)
                .show_ui(ui, |ui| {
                    for p in &self.recipient_projects {
                        if !p.active && self.draft.recipient_project_id != Some(p.id) {
                            continue;
                        }
                        ui.selectable_value(
                            &mut self.draft.recipient_project_id,
                            Some(p.id),
                            &p.name,
                        );
                    }
                });
            if ui.button("+ New recipient project").clicked() {
                self.new_recipient_project = Some(RecipientProjectDraft::default());
            }
        });

        enum Action {
            None,
            Create,
            Cancel,
        }
        let mut action = Action::None;

        if let Some(np) = &mut self.new_recipient_project {
            ui.add_space(6.0);
            ui.group(|ui| {
                egui::Grid::new("new_recipient_project_grid")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .min_col_width(80.0)
                    .show(ui, |ui| {
                        ui.label("Name *");
                        ui.add(egui::TextEdit::singleline(&mut np.name).desired_width(220.0));
                        ui.end_row();

                        ui.label("Contact info");
                        ui.add(
                            egui::TextEdit::singleline(&mut np.contact_info).desired_width(220.0),
                        );
                        ui.end_row();

                        ui.label("Location");
                        ui.add(egui::TextEdit::singleline(&mut np.location).desired_width(220.0));
                        ui.end_row();
                    });

                let ok = !np.name.trim().is_empty();
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
            Action::Cancel => self.new_recipient_project = None,
            Action::Create => {
                let draft = self.new_recipient_project.clone().unwrap();
                match qry::insert_recipient_project(db, &draft) {
                    Ok(new_id) => {
                        self.draft.recipient_project_id = Some(new_id);
                        self.new_recipient_project = None;
                        self.recipient_projects_loaded = false;
                        self.error = None;
                    }
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
            Action::None => {}
        }
    }

    fn show_item_picker(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Items given").strong());
        ui.add_space(4.0);

        let eligible: Vec<&InventoryItemRow> = self
            .inventory_items
            .iter()
            .filter(|item| {
                item.status == crate::model::inventory::ItemStatus::Available
                    || self.selected_item_ids.contains(&item.id)
            })
            .collect();

        if eligible.is_empty() {
            ui.weak("No available inventory items — add some in the Inventory section first.");
            return;
        }

        egui::ScrollArea::vertical()
            .id_salt("outbound_item_picker_scroll")
            .max_height(220.0)
            .show(ui, |ui| {
                for item in eligible {
                    let mut checked = self.selected_item_ids.contains(&item.id);
                    let label = format!(
                        "{}  [{}] · {}  ({})",
                        item.name,
                        item.category_name,
                        item.location.label(),
                        item.source_desc,
                    );
                    if ui.checkbox(&mut checked, label).changed() {
                        if checked {
                            self.selected_item_ids.insert(item.id);
                        } else {
                            self.selected_item_ids.remove(&item.id);
                        }
                    }
                }
            });

        ui.add_space(4.0);
        ui.weak(format!("{} item(s) selected", self.selected_item_ids.len()));
    }
}
