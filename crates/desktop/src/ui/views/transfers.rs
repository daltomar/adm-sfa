use eframe::egui;
use rusqlite::Connection;
use rust_i18n::t;
use std::path::{Path, PathBuf};

use adm_sfa_core::db::queries::{
    documents as docs_qry, settings as settings_qry, transfers as qry,
};
use adm_sfa_core::docs_fs;
use adm_sfa_core::format;
use adm_sfa_core::model::document::Document;
use adm_sfa_core::model::transfer::{AnnualTransfer, TransferDraft};
use adm_sfa_core::service;

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

/// Result of the shared attachment picker (`show_attachment_picker`) —
/// module scope since both `show_documents` (Editing) and
/// `show_staged_documents` (Adding) apply it after the borrow of
/// `self.pending_doc` ends. Mirrors `purchases.rs`'s identical enum.
enum DocAction {
    None,
    Confirm,
    Cancel,
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
    /// Documents staged (label + file picked, not yet filed) while adding a
    /// new transfer — `Mode::Adding` only. Mirrors `PurchasesView`'s
    /// `staged_docs`: entries land here once confirmed via
    /// `show_attachment_picker`'s "Add to list" button, and are filed
    /// together with the transfer on Save
    /// (`service::create_transfer_with_documents`).
    staged_docs: Vec<PendingAttachment>,
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
            staged_docs: Vec::new(),
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

    /// Same idea as `discard_pending_doc`, for every staged-but-unsaved
    /// document — called alongside it at every reset point so an abandoned
    /// "new transfer" flow doesn't leak staged screenshot temp files.
    fn discard_staged_docs(&mut self) {
        for p in self.staged_docs.drain(..) {
            if p.is_temp {
                let _ = std::fs::remove_file(&p.path);
            }
        }
    }

    pub fn invalidate(&mut self) {
        self.needs_reload = true;
        self.labels.clear();
    }

    /// Jumps straight into editing transfer `id` — the entry point for EUR
    /// Ledger's "Open in Transfers" cross-section navigation (`app.rs`'s
    /// handling of `eur_ledger::LedgerNavTarget`). Fetches the row
    /// directly rather than relying on `self.transfers` already containing
    /// it, since the caller may be jumping here without this section ever
    /// having been visited yet this session.
    pub fn select_for_edit(&mut self, db: &Connection, id: i64) {
        let Ok(Some(t)) = qry::get(db, id) else {
            // Shouldn't happen — the caller only reaches this with an id
            // from a live `linked_transfer_id` FK — but silently landing on
            // an unexplained list view would be worse than a rare stale
            // error banner, so this surfaces something rather than nothing.
            self.mode = Mode::List;
            self.error = Some(t!("transfers.error.linked_record_not_found").into_owned());
            return;
        };
        self.draft = TransferDraft {
            date: format::date(&t.date),
            eur_amount_sent_str: t.eur_amount_sent.to_string(),
            exchange_rate_str: t.exchange_rate.to_string(),
            notes: t.notes.clone().unwrap_or_default(),
        };
        self.mode = Mode::Editing(id);
        self.error = None;
        self.docs_needs_reload = true;
        self.discard_pending_doc();
        self.discard_staged_docs();
        self.path_input = None;
        self.capture_note = None;
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
                Mode::Adding => {
                    self.show_form(ui, db, data_dir);
                    ui.add_space(16.0);
                    ui.separator();
                    self.show_staged_documents(ui, db);
                }
                Mode::Editing(_) => {
                    self.show_form(ui, db, data_dir);
                    ui.add_space(16.0);
                    ui.separator();
                    self.show_documents(ui, db, data_dir);
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
            self.discard_staged_docs();
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
                        self.discard_staged_docs();
                        self.path_input = None;
                        self.capture_note = None;
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
            adm_sfa_core::money::parse_amount_input(self.draft.eur_amount_sent_str.trim()),
            adm_sfa_core::money::parse_amount_input(self.draft.exchange_rate_str.trim()),
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
        let date_ok = adm_sfa_core::date::parse_date_input(date_text).is_some();
        if !date_text.is_empty() && !date_ok {
            ui.colored_label(egui::Color32::RED, t!("common.error.invalid_date").as_ref());
        }

        let eur_text = self.draft.eur_amount_sent_str.trim();
        let eur_parsed = adm_sfa_core::money::parse_amount_input(eur_text);
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
        let rate_parsed = adm_sfa_core::money::parse_amount_input(rate_text);
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
                    let documents_dir = data_dir.join("documents");
                    // Clone into owned data first — building borrowed
                    // `PendingDocument`s directly from `self.staged_docs`
                    // would hold `self` borrowed across the `Ok` arm's
                    // `self.mode = ...` mutation below and fail to compile,
                    // same reasoning as `PurchasesView`'s identical clone.
                    let staged: Vec<(PathBuf, String)> = self
                        .staged_docs
                        .iter()
                        .map(|p| (p.path.clone(), p.label.clone()))
                        .collect();
                    let pending: Vec<service::PendingDocument> = staged
                        .iter()
                        .map(|(path, label)| service::PendingDocument {
                            path: path.as_path(),
                            label: label.as_str(),
                        })
                        .collect();
                    match service::create_transfer_with_documents(
                        db,
                        &documents_dir,
                        &self.draft,
                        &pending,
                    ) {
                        Ok(created) => {
                            self.mode = Mode::Editing(created.id);
                            self.docs_needs_reload = true;
                            self.needs_reload = true;

                            // Walk in reverse index order so `remove(i)`
                            // doesn't shift later indices. Successes are
                            // cleared (deleting their temp file if any);
                            // failures stay in `staged_docs` with their
                            // error set, for show_documents's retry
                            // hand-off to offer again.
                            let mut failed = 0usize;
                            for (i, outcome) in created.attachments.iter().enumerate().rev() {
                                match &outcome.result {
                                    Ok(_) => {
                                        let entry = self.staged_docs.remove(i);
                                        if entry.is_temp {
                                            let _ = std::fs::remove_file(&entry.path);
                                        }
                                    }
                                    Err(msg) => {
                                        failed += 1;
                                        self.staged_docs[i].error = Some(msg.clone());
                                    }
                                }
                            }
                            self.error = if failed > 0 {
                                Some(t!("common.doc.status.failed_count", n = failed).into_owned())
                            } else {
                                None
                            };
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
                self.discard_staged_docs();
                self.path_input = None;
                self.capture_note = None;
            }
        });
    }

    /// Shared attachment picker: drag-and-drop pickup, the pending-file
    /// group (filename, label combo, error, confirm/cancel), the browse-
    /// by-path flow, and the screenshot capture button. Used by both
    /// `show_documents` (Editing — confirming attaches immediately) and
    /// `show_staged_documents` (Adding — confirming stages for the next
    /// Save). Mirrors `PurchasesView::show_attachment_picker` field-for-
    /// field, adjusted for transfer id salts.
    fn show_attachment_picker(
        &mut self,
        ui: &mut egui::Ui,
        db: &Connection,
        confirm_label: &str,
    ) -> DocAction {
        // Drag-and-drop: pick up a file dropped onto the window when
        // nothing is already in progress. `staged_docs.is_empty()` also
        // guards against a stray drop jumping ahead of the Editing-mode
        // retry queue (see show_documents's hand-off).
        if self.pending_doc.is_none() && self.path_input.is_none() && self.staged_docs.is_empty() {
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
                        if ui.button(confirm_label).clicked() {
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

        doc_action
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
            match docs_fs::remove_document(db, &documents_dir, doc_id, &filename) {
                Err(e) => self.error = Some(e),
                Ok(()) => {
                    self.docs_needs_reload = true;
                    self.error = None;
                }
            }
        }

        ui.add_space(8.0);

        // Retry hand-off: pull the first still-failed staged document (left
        // over from a create-with-documents Save that partially failed)
        // into the normal pending-attachment flow, so it goes through the
        // exact same Attach/Cancel affordance as any other document — no
        // separate retry UI. Must run before show_attachment_picker's own
        // drag-and-drop pickup, which additionally checks
        // `staged_docs.is_empty()` so a dropped file can't jump this queue.
        if self.pending_doc.is_none() && !self.staged_docs.is_empty() {
            self.pending_doc = Some(self.staged_docs.remove(0));
        }

        let confirm_label = t!("common.doc.button.attach").into_owned();
        let doc_action = self.show_attachment_picker(ui, db, &confirm_label);

        match doc_action {
            DocAction::Cancel => self.discard_pending_doc(),
            DocAction::Confirm => {
                if let Some(p) = self.pending_doc.as_ref() {
                    let (path, label, is_temp) = (p.path.clone(), p.label.clone(), p.is_temp);
                    let existing: Vec<String> =
                        self.docs.iter().map(|d| d.filename.clone()).collect();
                    let persisted_date = self
                        .transfers
                        .iter()
                        .find(|t| t.id == edit_id)
                        .map(|t| t.date.as_str());
                    match service::attach_document(
                        db,
                        &documents_dir,
                        &path,
                        &self.draft.date,
                        persisted_date,
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

    /// Adding-mode counterpart of `show_documents`: lists documents already
    /// staged (label + file picked, not yet filed) and offers the shared
    /// picker to stage more. Nothing is written to disk or the DB here —
    /// staged entries are filed together with the transfer itself on Save
    /// (`service::create_transfer_with_documents`), so this needs `db` (for
    /// labels and the screenshot command) but not a documents directory.
    /// Mirrors `PurchasesView::show_staged_documents`.
    fn show_staged_documents(&mut self, ui: &mut egui::Ui, db: &Connection) {
        ui.heading(t!("common.doc.heading.staged").as_ref());
        ui.add_space(4.0);

        let mut remove_idx: Option<usize> = None;
        if self.staged_docs.is_empty() {
            ui.weak(t!("common.doc.staged_none").as_ref());
        } else {
            for (i, doc) in self.staged_docs.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&doc.label).strong());
                    ui.label(doc.path.file_name().unwrap_or_default().to_string_lossy());
                    if let Some(err) = &doc.error {
                        ui.colored_label(egui::Color32::RED, err);
                    }
                    if ui.small_button(t!("common.doc.remove").as_ref()).clicked() {
                        remove_idx = Some(i);
                    }
                });
            }
        }

        if let Some(i) = remove_idx {
            let removed = self.staged_docs.remove(i);
            if removed.is_temp {
                let _ = std::fs::remove_file(&removed.path);
            }
        }

        ui.add_space(8.0);

        let confirm_label = t!("common.doc.button.add_to_list").into_owned();
        let doc_action = self.show_attachment_picker(ui, db, &confirm_label);

        match doc_action {
            DocAction::Cancel => self.discard_pending_doc(),
            DocAction::Confirm => {
                if let Some(p) = self.pending_doc.take() {
                    self.staged_docs.push(p);
                }
            }
            DocAction::None => {}
        }
    }
}
