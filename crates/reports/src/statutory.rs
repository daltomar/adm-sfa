use std::path::Path;

use ecow::EcoString;
use typst::foundations::{Array, Dict, IntoValue, Str, Value};
use typst_as_lib::typst_kit_options::TypstKitFontOptions;
use typst_as_lib::TypstEngine;
use typst_layout::PagedDocument;
use typst_pdf::PdfOptions;

// ── Input struct ─────────────────────────────────────────────────────────────

/// Pre-formatted strings for all four statutory report templates.
/// The caller (web route or desktop view) formats amounts and dates in the
/// active locale before filling this struct; the templates receive only
/// ready-to-render strings.
pub struct StatutoryInput {
    pub org_name: String,
    pub org_address: String,
    pub org_tax_number: String,
    /// Four-digit year string, e.g. "2025".
    pub year: String,
    /// Formatted generation date, e.g. "13.02.2026".
    pub generated: String,

    // Einnahmen/Ausgaben
    pub donations: String,
    pub self_funding: String,
    pub project_expenses: String,
    pub income_total: String,
    pub expense_total: String,
    pub surplus: String,

    // Vermögen
    pub bank_balance: String,
    pub inventory_value: String,
    pub total_assets: String,

    // Narrative fields (paragraphs pre-split on blank lines)
    pub member_count: String,
    pub meeting_date: String,
    pub meeting_time_from: String,
    pub meeting_time_to: String,
    /// Outbound events that occurred in the fiscal year.
    pub outbound_events: Vec<OutboundEventLine>,
    /// Paragraphs for the Tätigkeitsbericht "Durchgeführte Maßnahmen" section.
    pub tb_activities_paras: Vec<String>,
    /// Paragraphs for the "Sonstige Tätigkeiten" section.
    pub tb_continuous_paras: Vec<String>,
    /// Paragraphs for the "Ausblick" section.
    pub tb_outlook_paras: Vec<String>,
    /// Paragraphs for the Protokoll "Planung Tätigkeiten next year" section.
    pub pk_activities_paras: Vec<String>,
    /// Paragraphs for the Protokoll "Verschiedenes" section.
    pub pk_decisions_paras: Vec<String>,
}

pub struct OutboundEventLine {
    pub date: String,
    pub recipient: String,
    pub item_count: String,
}

// ── Export functions ─────────────────────────────────────────────────────────

static EINNAHMEN_AUSGABEN: &str = include_str!("../templates/einnahmen_ausgaben.typ");
static VERMOEGEN: &str = include_str!("../templates/vermoegen.typ");
static TAETIGKEITSBERICHT: &str = include_str!("../templates/taetigkeitsbericht.typ");
static PROTOKOLL: &str = include_str!("../templates/protokoll.typ");
static JAHRESBERICHT: &str = include_str!("../templates/jahresbericht.typ");

pub fn export_einnahmen_ausgaben(
    input: &StatutoryInput,
    dest: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    run_export(EINNAHMEN_AUSGABEN, &build_dict(input), dest)
}

pub fn export_vermoegen(
    input: &StatutoryInput,
    dest: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    run_export(VERMOEGEN, &build_dict(input), dest)
}

pub fn export_taetigkeitsbericht(
    input: &StatutoryInput,
    dest: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    run_export(TAETIGKEITSBERICHT, &build_dict(input), dest)
}

pub fn export_protokoll(
    input: &StatutoryInput,
    dest: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    run_export(PROTOKOLL, &build_dict(input), dest)
}

/// Generates all four reports in a single PDF (page break between each).
pub fn export_all(input: &StatutoryInput, dest: &Path) -> Result<(), Box<dyn std::error::Error>> {
    run_export(JAHRESBERICHT, &build_dict(input), dest)
}

// ── Internal helpers ─────────────────────────────────────────────────────────

fn s(text: &str) -> Value {
    Str::from(text).into_value()
}

fn str_array(paras: &[String]) -> Value {
    let arr: Array = paras.iter().map(|p| s(p)).collect();
    arr.into_value()
}

fn build_dict(input: &StatutoryInput) -> Dict {
    let mut d = Dict::new();
    let mut ins = |k: &str, v: Value| d.insert(EcoString::from(k).into(), v);

    ins("org_name", s(&input.org_name));
    ins("org_address", s(&input.org_address));
    ins("org_tax_number", s(&input.org_tax_number));
    ins("year", s(&input.year));
    ins(
        "next_year",
        s(&(input.year.parse::<i32>().unwrap_or(0) + 1).to_string()),
    );
    ins("generated", s(&input.generated));

    ins("donations", s(&input.donations));
    ins("self_funding", s(&input.self_funding));
    ins("project_expenses", s(&input.project_expenses));
    ins("income_total", s(&input.income_total));
    ins("expense_total", s(&input.expense_total));
    ins("surplus", s(&input.surplus));

    ins("bank_balance", s(&input.bank_balance));
    ins("inventory_value", s(&input.inventory_value));
    ins("total_assets", s(&input.total_assets));

    ins("member_count", s(&input.member_count));
    ins("meeting_date", s(&input.meeting_date));
    ins("meeting_time_from", s(&input.meeting_time_from));
    ins("meeting_time_to", s(&input.meeting_time_to));

    let events: Array = input
        .outbound_events
        .iter()
        .map(|e| {
            let mut ed = Dict::new();
            ed.insert(EcoString::from("date").into(), s(&e.date));
            ed.insert(EcoString::from("recipient").into(), s(&e.recipient));
            ed.insert(EcoString::from("item_count").into(), s(&e.item_count));
            Value::Dict(ed)
        })
        .collect();
    ins("outbound_events", events.into_value());

    ins("tb_activities_paras", str_array(&input.tb_activities_paras));
    ins("tb_continuous_paras", str_array(&input.tb_continuous_paras));
    ins("tb_outlook_paras", str_array(&input.tb_outlook_paras));
    ins("pk_activities_paras", str_array(&input.pk_activities_paras));
    ins("pk_decisions_paras", str_array(&input.pk_decisions_paras));

    d
}

fn run_export(
    template: &'static str,
    inputs: &Dict,
    dest: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        do_export(template, inputs, dest)
    }))
    .unwrap_or_else(|_| {
        Err("PDF generation crashed (likely a font-index issue in the Typst renderer)".into())
    })
}

fn do_export(
    template: &'static str,
    inputs: &Dict,
    dest: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let engine = TypstEngine::builder()
        .main_file(template)
        .search_fonts_with(TypstKitFontOptions::default())
        .build();

    let result = engine.compile_with_input::<_, PagedDocument>(inputs.clone());

    for w in &result.warnings {
        eprintln!("[adm-sfa statutory pdf] typst warning: {w:?}");
    }

    let doc = result.output.map_err(|e| e.to_string())?;
    let pdf_bytes = typst_pdf::pdf(&doc, &PdfOptions::default()).map_err(|errs| {
        errs.iter()
            .map(|d| d.message.to_string())
            .collect::<Vec<_>>()
            .join("; ")
    })?;

    if let Some(parent) = dest.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(dest, pdf_bytes)?;
    Ok(())
}
