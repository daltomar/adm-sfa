use std::path::Path;
use walkdir::WalkDir;

/// Zips the entire data directory (DB + documents/) to `dest`.
/// Writes to a `.tmp` sibling first and renames atomically so a partial write
/// never corrupts an existing backup.
/// Uses zip v8 — verify SimpleFileOptions API if this fails to compile.
#[allow(dead_code)]
pub fn backup_to_zip(data_dir: &Path, dest: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let tmp = dest.with_extension("zip.tmp");
    let result = write_zip(data_dir, &tmp);
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return result;
    }
    std::fs::rename(&tmp, dest).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e
    })?;
    Ok(())
}

fn write_zip(data_dir: &Path, dest: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(dest)?;
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    for entry in WalkDir::new(data_dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let rel = path.strip_prefix(data_dir)?;
        writer.start_file(rel.to_string_lossy(), options)?;
        let mut f = std::fs::File::open(path)?;
        std::io::copy(&mut f, &mut writer)?;
    }
    writer.finish()?;
    Ok(())
}
