use std::path::Path;

use ecow::EcoString;
use typst::foundations::{Array, Dict, IntoValue, Str, Value};
use typst_as_lib::typst_kit_options::TypstKitFontOptions;
use typst_as_lib::TypstEngine;
use typst_layout::PagedDocument;
use typst_pdf::PdfOptions;

static TEMPLATE: &str = include_str!("../../templates/report.typ");

/// A single named table section to render in the PDF.
///
/// All rows must have exactly `headers.len()` cells; mismatched lengths
/// will silently misalign table columns in the Typst output.
pub struct PdfSection {
    pub title: String,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// Render all report sections to a PDF file at `dest`.
///
/// Creates the parent directory if it does not exist.
/// Wraps compilation in `catch_unwind` to guard against panics in the
/// upstream `typst-as-lib` font-index code path.
pub fn export(
    dest: &Path,
    generated: &str,
    date_from: &str,
    date_to: &str,
    sections: &[PdfSection],
) -> Result<(), Box<dyn std::error::Error>> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_export(dest, generated, date_from, date_to, sections)
    }))
    .unwrap_or_else(|_| {
        Err("PDF generation crashed (likely a font-index issue in the Typst renderer)".into())
    })
}

fn run_export(
    dest: &Path,
    generated: &str,
    date_from: &str,
    date_to: &str,
    sections: &[PdfSection],
) -> Result<(), Box<dyn std::error::Error>> {
    let engine = TypstEngine::builder()
        .main_file(TEMPLATE)
        .search_fonts_with(TypstKitFontOptions::default())
        .build();

    let inputs = build_inputs(generated, date_from, date_to, sections);

    let result = engine.compile_with_input::<_, PagedDocument>(inputs);

    if !result.warnings.is_empty() {
        for w in &result.warnings {
            eprintln!("[adm-sfa pdf] typst warning: {w:?}");
        }
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

fn s(text: &str) -> Value {
    Str::from(text).into_value()
}

fn build_inputs(generated: &str, date_from: &str, date_to: &str, sections: &[PdfSection]) -> Dict {
    let section_values: Array = sections
        .iter()
        .map(|sec| {
            let mut d = Dict::new();
            d.insert(EcoString::from("title").into(), s(&sec.title));

            let headers: Array = sec.headers.iter().map(|h| s(h)).collect();
            d.insert(EcoString::from("headers").into(), headers.into_value());

            let rows: Array = sec
                .rows
                .iter()
                .map(|row| {
                    let cells: Array = row.iter().map(|c| s(c)).collect();
                    cells.into_value()
                })
                .collect();
            d.insert(EcoString::from("rows").into(), rows.into_value());

            Value::Dict(d)
        })
        .collect();

    let mut inputs = Dict::new();
    inputs.insert(EcoString::from("generated").into(), s(generated));
    inputs.insert(EcoString::from("date_from").into(), s(date_from));
    inputs.insert(EcoString::from("date_to").into(), s(date_to));
    inputs.insert(
        EcoString::from("sections").into(),
        section_values.into_value(),
    );
    inputs
}
