use std::path::Path;

/// Write a CSV file with the given headers and rows.
pub fn write(
    path: &Path,
    headers: &[String],
    rows: &[Vec<String>],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut wtr = csv::Writer::from_path(path)?;
    wtr.write_record(headers)?;
    for row in rows {
        wtr.write_record(row)?;
    }
    wtr.flush()?;
    Ok(())
}
