use std::path::Path;

/// Write a CSV file with the given headers and rows.
///
/// Always uses `;` as the field delimiter (SPEC.md §6.4/T6) — CSV output is
/// German-format and locale-independent, since `,` is the decimal separator
/// German/Brazilian spreadsheet software expects, which would collide with a
/// `,` field delimiter. Not affected by the active UI language.
pub fn write(
    path: &Path,
    headers: &[String],
    rows: &[Vec<String>],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut wtr = csv::WriterBuilder::new().delimiter(b';').from_path(path)?;
    wtr.write_record(headers)?;
    for row in rows {
        wtr.write_record(row)?;
    }
    wtr.flush()?;
    Ok(())
}
