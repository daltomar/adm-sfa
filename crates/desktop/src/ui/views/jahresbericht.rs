use eframe::egui;
use rusqlite::Connection;
use rust_i18n::t;

use adm_sfa_core::db::queries::annual_report_draft as draft_qry;
use adm_sfa_core::format;
use adm_sfa_core::statutory::{self, AnnualReportDraft, StatutoryReportData};
use reports::statutory::{OutboundEventLine, StatutoryInput};

/// Which PDF export is waiting for a save path from the user.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PendingExport {
    EinnahmenAusgaben,
    Vermoegen,
    Taetigkeitsbericht,
    Protokoll,
    All,
}

pub struct JahresberichtView {
    year: i32,
    data: Option<StatutoryReportData>,
    // Editable draft fields (mirrors AnnualReportDraft)
    member_count_str: String,
    meeting_date: String,
    meeting_time_from: String,
    meeting_time_to: String,
    tb_activities: String,
    tb_continuous: String,
    tb_outlook: String,
    pk_activities_next_year: String,
    pk_decisions: String,
    // Export path dialog
    pending_export: Option<PendingExport>,
    export_path_input: Option<String>,
    export_status: Option<Result<String, String>>,
    save_status: Option<Result<String, String>>,
}

impl Default for JahresberichtView {
    fn default() -> Self {
        let year: i32 = chrono::Local::now()
            .format("%Y")
            .to_string()
            .parse()
            .unwrap_or(2025)
            - 1;
        Self {
            year,
            data: None,
            member_count_str: String::new(),
            meeting_date: String::new(),
            meeting_time_from: String::new(),
            meeting_time_to: String::new(),
            tb_activities: String::new(),
            tb_continuous: String::new(),
            tb_outlook: String::new(),
            pk_activities_next_year: String::new(),
            pk_decisions: String::new(),
            pending_export: None,
            export_path_input: None,
            export_status: None,
            save_status: None,
        }
    }
}

impl JahresberichtView {
    pub fn invalidate(&mut self) {
        self.data = None;
        self.save_status = None;
        self.export_status = None;
    }

    pub fn show(&mut self, ui: &mut egui::Ui, db: &Connection) {
        // Load data on first render or after invalidate
        if self.data.is_none() {
            match statutory::load(db, self.year) {
                Ok(d) => {
                    self.member_count_str = d.draft.member_count.to_string();
                    self.meeting_date = d.draft.meeting_date.clone();
                    self.meeting_time_from = d.draft.meeting_time_from.clone();
                    self.meeting_time_to = d.draft.meeting_time_to.clone();
                    self.tb_activities = d.draft.tb_activities.clone();
                    self.tb_continuous = d.draft.tb_continuous.clone();
                    self.tb_outlook = d.draft.tb_outlook.clone();
                    self.pk_activities_next_year = d.draft.pk_activities_next_year.clone();
                    self.pk_decisions = d.draft.pk_decisions.clone();
                    self.data = Some(d);
                }
                Err(e) => {
                    self.export_status = Some(Err(e.to_string()));
                }
            }
        }

        ui.heading(t!("jahresbericht.heading").as_ref());
        ui.add_space(8.0);

        crate::ui::widgets::status_banner::show(ui, &self.save_status);
        crate::ui::widgets::status_banner::show(ui, &self.export_status);

        egui::ScrollArea::vertical()
            .id_salt("jahresbericht_scroll")
            .show(ui, |ui| {
                // ── Year picker ──────────────────────────────────────────────
                ui.horizontal(|ui| {
                    ui.label(t!("jahresbericht.year.label").as_ref());
                    let old_year = self.year;
                    ui.add(egui::DragValue::new(&mut self.year).range(2020..=2100));
                    if self.year != old_year {
                        self.data = None;
                        self.save_status = None;
                        self.export_status = None;
                    }
                });
                ui.add_space(8.0);

                let org_name = self
                    .data
                    .as_ref()
                    .map(|d| d.org_name.as_str())
                    .unwrap_or("");
                if org_name.is_empty() {
                    ui.colored_label(
                        egui::Color32::DARK_RED,
                        t!("jahresbericht.warning.org_missing").as_ref(),
                    );
                    ui.add_space(8.0);
                }

                // ── Financial summary (read-only) ────────────────────────────
                if let Some(d) = &self.data {
                    ui.separator();
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(t!("jahresbericht.section.financial").as_ref())
                            .strong(),
                    );
                    egui::Grid::new("jb_financial")
                        .num_columns(2)
                        .spacing([12.0, 4.0])
                        .show(ui, |ui| {
                            let ea = &d.einnahmen_ausgaben;
                            ui.label("Einnahmen:");
                            ui.label(format!("€ {}", format::amount(ea.income_total)));
                            ui.end_row();
                            ui.label("Ausgaben:");
                            ui.label(format!("€ {}", format::amount(ea.expense_total)));
                            ui.end_row();
                            ui.label("Überschuss:");
                            ui.label(format!("€ {}", format::amount(ea.surplus)));
                            ui.end_row();
                            ui.label("Kontostand 31.12.:");
                            ui.label(format!("€ {}", format::amount(d.vermoegen.bank_balance)));
                            ui.end_row();
                            ui.label("Inventarwert:");
                            ui.label(format!("€ {}", format::amount(d.vermoegen.inventory_value)));
                            ui.end_row();
                        });
                    ui.add_space(6.0);

                    // Outbound events
                    ui.separator();
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(t!("jahresbericht.section.events").as_ref()).strong(),
                    );
                    if d.outbound_events.is_empty() {
                        ui.weak(t!("jahresbericht.events.none").as_ref());
                    } else {
                        for e in &d.outbound_events {
                            ui.label(format!(
                                "{} — {} ({} items)",
                                e.date, e.recipient_name, e.item_count
                            ));
                        }
                    }
                    ui.add_space(6.0);
                }

                // ── Narrative inputs ─────────────────────────────────────────
                ui.separator();
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(t!("jahresbericht.section.narrative").as_ref()).strong(),
                );
                ui.add_space(4.0);

                let text_width = ui.available_width() - 20.0;
                macro_rules! text_field {
                    ($label:expr, $field:expr, $height:expr) => {{
                        ui.label(egui::RichText::new($label).small());
                        ui.add(
                            egui::TextEdit::multiline(&mut $field)
                                .desired_width(text_width)
                                .desired_rows($height),
                        );
                        ui.add_space(6.0);
                    }};
                }

                egui::Grid::new("jb_meta")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        ui.label(t!("jahresbericht.field.member_count").as_ref());
                        ui.add(
                            egui::TextEdit::singleline(&mut self.member_count_str)
                                .desired_width(60.0),
                        );
                        ui.end_row();
                        ui.label(t!("jahresbericht.field.meeting_date").as_ref());
                        ui.add(
                            egui::TextEdit::singleline(&mut self.meeting_date).desired_width(140.0),
                        );
                        ui.end_row();
                        ui.label(t!("jahresbericht.field.meeting_time_from").as_ref());
                        ui.add(
                            egui::TextEdit::singleline(&mut self.meeting_time_from)
                                .desired_width(80.0),
                        );
                        ui.end_row();
                        ui.label(t!("jahresbericht.field.meeting_time_to").as_ref());
                        ui.add(
                            egui::TextEdit::singleline(&mut self.meeting_time_to)
                                .desired_width(80.0),
                        );
                        ui.end_row();
                    });

                text_field!(
                    t!("jahresbericht.field.tb_activities").as_ref(),
                    self.tb_activities,
                    5
                );
                text_field!(
                    t!("jahresbericht.field.tb_continuous").as_ref(),
                    self.tb_continuous,
                    4
                );
                text_field!(
                    t!("jahresbericht.field.tb_outlook").as_ref(),
                    self.tb_outlook,
                    4
                );
                text_field!(
                    t!("jahresbericht.field.pk_activities_next_year").as_ref(),
                    self.pk_activities_next_year,
                    4
                );
                text_field!(
                    t!("jahresbericht.field.pk_decisions").as_ref(),
                    self.pk_decisions,
                    3
                );

                // Save draft button
                if ui.button(t!("common.save").as_ref()).clicked() {
                    let draft = self.build_draft();
                    self.save_status = Some(
                        draft_qry::upsert(db, &draft)
                            .map(|()| t!("jahresbericht.status.saved").into_owned())
                            .map_err(|e| e.to_string()),
                    );
                    if let Some(d) = &mut self.data {
                        d.draft = draft;
                    }
                }

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);

                // ── Export buttons ───────────────────────────────────────────
                ui.horizontal_wrapped(|ui| {
                    if ui.button(t!("jahresbericht.export.all").as_ref()).clicked() {
                        self.start_export(PendingExport::All);
                    }
                    if ui
                        .button(t!("jahresbericht.export.einnahmen_ausgaben").as_ref())
                        .clicked()
                    {
                        self.start_export(PendingExport::EinnahmenAusgaben);
                    }
                    if ui
                        .button(t!("jahresbericht.export.vermoegen").as_ref())
                        .clicked()
                    {
                        self.start_export(PendingExport::Vermoegen);
                    }
                    if ui
                        .button(t!("jahresbericht.export.taetigkeitsbericht").as_ref())
                        .clicked()
                    {
                        self.start_export(PendingExport::Taetigkeitsbericht);
                    }
                    if ui
                        .button(t!("jahresbericht.export.protokoll").as_ref())
                        .clicked()
                    {
                        self.start_export(PendingExport::Protokoll);
                    }
                });

                // Save path dialog for exports
                let mut do_export = false;
                let mut cancel_export = false;
                if let Some(ref mut path_str) = self.export_path_input {
                    ui.add_space(6.0);
                    ui.group(|ui| {
                        ui.label(t!("settings.backup.field.save_to").as_ref());
                        ui.add(egui::TextEdit::singleline(path_str).desired_width(500.0));
                        ui.horizontal(|ui| {
                            if ui.button(t!("common.save").as_ref()).clicked() {
                                do_export = true;
                            }
                            if ui.button(t!("common.cancel").as_ref()).clicked() {
                                cancel_export = true;
                            }
                        });
                    });
                }

                if do_export {
                    self.run_export(db);
                } else if cancel_export {
                    self.export_path_input = None;
                    self.pending_export = None;
                }
            });
    }

    fn build_draft(&self) -> AnnualReportDraft {
        AnnualReportDraft {
            year: self.year,
            member_count: self.member_count_str.trim().parse().unwrap_or(0),
            meeting_date: self.meeting_date.trim().to_string(),
            meeting_time_from: self.meeting_time_from.trim().to_string(),
            meeting_time_to: self.meeting_time_to.trim().to_string(),
            tb_activities: self.tb_activities.clone(),
            tb_continuous: self.tb_continuous.clone(),
            tb_outlook: self.tb_outlook.clone(),
            pk_activities_next_year: self.pk_activities_next_year.clone(),
            pk_decisions: self.pk_decisions.clone(),
        }
    }

    fn start_export(&mut self, kind: PendingExport) {
        let suffix = match kind {
            PendingExport::All => "jahresbericht",
            PendingExport::EinnahmenAusgaben => "einnahmen-ausgaben",
            PendingExport::Vermoegen => "vermoegen",
            PendingExport::Taetigkeitsbericht => "taetigkeitsbericht",
            PendingExport::Protokoll => "protokoll",
        };
        let default_name = format!(
            "adm-sfa-{}-{}.pdf",
            suffix,
            chrono::Local::now().format("%Y-%m-%d")
        );
        let default_path = dirs::download_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from(".")))
            .join(default_name)
            .to_string_lossy()
            .into_owned();
        self.pending_export = Some(kind);
        self.export_path_input = Some(default_path);
        self.export_status = None;
    }

    fn run_export(&mut self, db: &Connection) {
        let path_str = self
            .export_path_input
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_string();
        self.export_path_input = None;

        let Some(kind) = self.pending_export.take() else {
            return;
        };
        let path = std::path::PathBuf::from(&path_str);
        if path.as_os_str().is_empty() {
            self.export_status = Some(Err(t!("common.error.path_required").into_owned()));
            return;
        }

        let Some(d) = &self.data else {
            return;
        };

        let locale = adm_sfa_core::db::queries::settings::get(db, "ui_locale")
            .ok()
            .flatten()
            .unwrap_or_else(|| "de".to_string());
        let input = build_input(d, &locale);

        let result = match kind {
            PendingExport::All => reports::statutory::export_all(&input, &path),
            PendingExport::EinnahmenAusgaben => {
                reports::statutory::export_einnahmen_ausgaben(&input, &path)
            }
            PendingExport::Vermoegen => reports::statutory::export_vermoegen(&input, &path),
            PendingExport::Taetigkeitsbericht => {
                reports::statutory::export_taetigkeitsbericht(&input, &path)
            }
            PendingExport::Protokoll => reports::statutory::export_protokoll(&input, &path),
        };

        self.export_status = Some(
            result
                .map(|()| t!("common.status.saved_to", path = path.display()).into_owned())
                .map_err(|e| e.to_string()),
        );
    }
}

// ── Conversion helper ─────────────────────────────────────────────────────────

fn split_paragraphs(text: &str) -> Vec<String> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    text.split("\n\n")
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

pub(crate) fn build_input(data: &StatutoryReportData, locale: &str) -> StatutoryInput {
    let ea = &data.einnahmen_ausgaben;
    let v = &data.vermoegen;
    let d = &data.draft;

    StatutoryInput {
        org_name: data.org_name.clone(),
        org_address: data.org_address.clone(),
        org_tax_number: data.org_tax_number.clone(),
        year: data.year.to_string(),
        generated: chrono::Local::now().format("%d.%m.%Y").to_string(),
        donations: format::amount_in(ea.donations, locale),
        self_funding: format::amount_in(ea.other_income, locale),
        project_expenses: format::amount_in(ea.project_expenses, locale),
        income_total: format::amount_in(ea.income_total, locale),
        expense_total: format::amount_in(ea.expense_total, locale),
        surplus: format::amount_in(ea.surplus, locale),
        bank_balance: format::amount_in(v.bank_balance, locale),
        inventory_value: format::amount_in(v.inventory_value, locale),
        total_assets: format::amount_in(v.total_assets, locale),
        member_count: d.member_count.to_string(),
        meeting_date: d.meeting_date.clone(),
        meeting_time_from: d.meeting_time_from.clone(),
        meeting_time_to: d.meeting_time_to.clone(),
        outbound_events: data
            .outbound_events
            .iter()
            .map(|e| OutboundEventLine {
                date: format::date_in(&e.date, locale),
                recipient: e.recipient_name.clone(),
                item_count: e.item_count.to_string(),
            })
            .collect(),
        tb_activities_paras: split_paragraphs(&d.tb_activities),
        tb_continuous_paras: split_paragraphs(&d.tb_continuous),
        tb_outlook_paras: split_paragraphs(&d.tb_outlook),
        pk_activities_paras: split_paragraphs(&d.pk_activities_next_year),
        pk_decisions_paras: split_paragraphs(&d.pk_decisions),
    }
}
