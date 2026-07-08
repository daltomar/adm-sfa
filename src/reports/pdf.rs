use std::path::Path;

use ecow::EcoString;
use typst::foundations::{Array, Dict, IntoValue, Str, Value};
use typst_as_lib::TypstEngine;
use typst_as_lib::typst_kit_options::TypstKitFontOptions;
use typst_layout::PagedDocument;
use typst_pdf::PdfOptions;

static TEMPLATE: &str = include_str!("../../templates/report.typ");

/// A single named table section to render in the PDF.
pub struct PdfSection {
    pub title: &'static str,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// Render all report sections to a PDF file at `dest`.
pub fn export(
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

    let doc: PagedDocument = engine
        .compile_with_input(inputs)
        .output
        .map_err(|e| format!("{e:?}"))?;

    let pdf_bytes =
        typst_pdf::pdf(&doc, &PdfOptions::default()).map_err(|e| format!("{e:?}"))?;

    std::fs::write(dest, pdf_bytes)?;
    Ok(())
}

fn s(text: &str) -> Value {
    Str::from(text).into_value()
}

fn build_inputs(
    generated: &str,
    date_from: &str,
    date_to: &str,
    sections: &[PdfSection],
) -> Dict {
    let section_values: Array = sections.iter().map(|sec| {
        let mut d = Dict::new();
        d.insert(EcoString::from("title").into(), s(sec.title));

        let headers: Array = sec.headers.iter().map(|h| s(h)).collect();
        d.insert(EcoString::from("headers").into(), headers.into_value());

        let rows: Array = sec.rows.iter().map(|row| {
            let cells: Array = row.iter().map(|c| s(c)).collect();
            cells.into_value()
        }).collect();
        d.insert(EcoString::from("rows").into(), rows.into_value());

        Value::Dict(d)
    }).collect();

    let mut inputs = Dict::new();
    inputs.insert(EcoString::from("generated").into(), s(generated));
    inputs.insert(EcoString::from("date_from").into(), s(date_from));
    inputs.insert(EcoString::from("date_to").into(), s(date_to));
    inputs.insert(EcoString::from("sections").into(), section_values.into_value());
    inputs
}
