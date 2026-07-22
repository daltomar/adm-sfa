use eframe::egui;
use rusqlite::Connection;
use rust_i18n::t;
use std::path::{Path, PathBuf};

use crate::db::queries::{documents as docs_qry, settings as settings_qry, transfers as qry};
use crate::docs_fs;
use crate::format;
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
    /// True if `path` is a temp file this app created (a screenshot capture)
    /// rather than a user's own file — deleted once no longer needed instead
    /// of left in the OS temp dir.
    is_temp: bool,
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
    capture_note: Option<String>,
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
            capture_note: None,
        }
    }
}

impl TransfersView {
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
        self.labels.clear();
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
                    ui.weak(t!("transfers.hint.select_or_add").as_ref());
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
        ui.heading(t!("sidebar.transfers").as_ref());
        ui.add_space(4.0);

        if ui.button(t!("transfers.button.add").as_ref()).clicked() {
            self.draft = TransferDraft::default();
            self.mode = Mode::Adding;
            self.error = None;
            self.docs = Vec::new();
            self.discard_pending_doc();
            self.path_input = None;
            self.capture_note = None;
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
                    ui.weak(t!("transfers.empty").as_ref());
                    return;
                }
                for i in 0..self.transfers.len() {
                    let tr = &self.transfers[i];
                    let id = tr.id;
                    let row = t!(
                        "transfers.row",
                        date = format::date(&tr.date),
                        eur = format::amount(tr.eur_amount_sent),
                        brl = format::amount(tr.brl_amount_received),
                        rate = format::number(tr.exchange_rate, 4)
                    )
                    .into_owned();
                    let selected = matches!(self.mode, Mode::Editing(eid) if eid == id);
                    if ui.selectable_label(selected, &row).clicked() {
                        self.draft = TransferDraft {
                            date: format::date(&self.transfers[i].date),
                            eur_amount_sent_str: self.transfers[i].eur_amount_sent.to_string(),
                            exchange_rate_str: self.transfers[i].exchange_rate.to_string(),
                            notes: self.transfers[i].notes.clone().unwrap_or_default(),
                        };
                        self.mode = Mode::Editing(id);
                        self.error = None;
                        self.docs_needs_reload = true;
                        self.discard_pending_doc();
                        self.path_input = None;
                        self.capture_note = None;
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
            t!("transfers.heading.new")
        } else {
            t!("transfers.heading.edit")
        };
        ui.heading(heading.as_ref());
        ui.add_space(8.0);

        egui::Grid::new("transfer_form_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .min_col_width(120.0)
            .show(ui, |ui| {
                ui.label(t!("common.field.date").as_ref());
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.date)
                        .hint_text(t!("common.field.date_hint").as_ref())
                        .desired_width(140.0),
                );
                ui.end_row();

                ui.label(t!("transfers.field.eur_amount").as_ref());
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.eur_amount_sent_str)
                        .hint_text(t!("common.field.amount_hint").as_ref())
                        .desired_width(140.0),
                );
                ui.end_row();

                ui.label(t!("transfers.field.exchange_rate").as_ref());
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.exchange_rate_str)
                        .hint_text(t!("transfers.field.exchange_rate_hint").as_ref())
                        .desired_width(140.0),
                );
                ui.end_row();

                ui.label(t!("common.field.notes").as_ref());
                ui.add(
                    egui::TextEdit::multiline(&mut self.draft.notes)
                        .desired_width(280.0)
                        .desired_rows(3),
                );
                ui.end_row();
            });

        if let (Some(eur), Some(rate)) = (
            crate::money::parse_amount_input(self.draft.eur_amount_sent_str.trim()),
            crate::money::parse_amount_input(self.draft.exchange_rate_str.trim()),
        ) {
            ui.add_space(4.0);
            ui.label(
                t!(
                    "transfers.field.brl_received",
                    amount = format::amount(eur * rate)
                )
                .into_owned(),
            );
        }

        if let Some(err) = &self.error {
            ui.add_space(4.0);
            ui.colored_label(egui::Color32::RED, err);
        }

        let date_text = self.draft.date.trim();
        let date_ok = crate::date::parse_date_input(date_text).is_some();
        if !date_text.is_empty() && !date_ok {
            ui.colored_label(egui::Color32::RED, t!("common.error.invalid_date").as_ref());
        }

        let eur_text = self.draft.eur_amount_sent_str.trim();
        let eur_parsed = crate::money::parse_amount_input(eur_text);
        let eur_ok = eur_parsed
            .map(|d| d > rust_decimal::Decimal::ZERO)
            .unwrap_or(false);
        if !eur_text.is_empty() {
            if eur_parsed.is_none() {
                ui.colored_label(
                    egui::Color32::RED,
                    t!("transfers.error.eur_invalid").as_ref(),
                );
            } else if !eur_ok {
                ui.colored_label(egui::Color32::RED, t!("transfers.error.eur_zero").as_ref());
            }
        }
        let rate_text = self.draft.exchange_rate_str.trim();
        let rate_parsed = crate::money::parse_amount_input(rate_text);
        let rate_ok = rate_parsed
            .map(|d| d > rust_decimal::Decimal::ZERO)
            .unwrap_or(false);
        if !rate_text.is_empty() {
            if rate_parsed.is_none() {
                ui.colored_label(
                    egui::Color32::RED,
                    t!("transfers.error.rate_invalid").as_ref(),
                );
            } else if !rate_ok {
                ui.colored_label(egui::Color32::RED, t!("transfers.error.rate_zero").as_ref());
            }
        }
        let form_ok = date_ok && eur_ok && rate_ok;

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(form_ok, egui::Button::new(t!("common.save").as_ref()))
                .clicked()
            {
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

            if ui.button(t!("common.cancel").as_ref()).clicked() {
                self.mode = Mode::List;
                self.error = None;
                self.discard_pending_doc();
                self.path_input = None;
                self.capture_note = None;
            }
        });
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
                    // Filenames must stay ISO-sortable (T4) regardless of
                    // what the user has currently typed into the date
                    // field: prefer the parsed draft date, fall back to
                    // the persisted transfer's own ISO date if the draft
                    // is momentarily unparseable mid-edit, and to today's
                    // date as a last resort.
                    let filing_date = crate::date::parse_date_input(&self.draft.date)
                        .map(|d| d.format("%Y-%m-%d").to_string())
                        .or_else(|| {
                            self.transfers
                                .iter()
                                .find(|t| t.id == edit_id)
                                .map(|t| t.date.clone())
                        })
                        .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());
                    match docs_fs::file_document(
                        db,
                        &documents_dir,
                        &path,
                        &filing_date,
                        ("transfer", edit_id),
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
