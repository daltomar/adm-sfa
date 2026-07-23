use std::path::Path;
use walkdir::WalkDir;

/// Zips the entire data directory (DB + documents/) to `dest`.
/// Writes to a `.tmp` sibling first and renames atomically so a partial write
/// never corrupts an existing backup.
/// Uses zip v8 — verify SimpleFileOptions API if this fails to compile.
pub fn backup_to_zip(data_dir: &Path, dest: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let tmp = dest.with_extension("zip.tmp");
    let result = write_zip(data_dir, &tmp);
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return result;
    }
    std::fs::rename(&tmp, dest).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zips_db_and_documents_and_is_readable() {
        let tmp = std::env::temp_dir().join(format!("adm-sfa-backup-test-{}", std::process::id()));
        let data_dir = tmp.join("data");
        std::fs::create_dir_all(data_dir.join("documents")).unwrap();
        std::fs::write(data_dir.join("adm-sfa.sqlite"), b"fake db").unwrap();
        std::fs::write(
            data_dir.join("documents/2026-01-01_purchase-1_ad.jpg"),
            b"fake photo",
        )
        .unwrap();

        let dest = tmp.join("out.zip");
        backup_to_zip(&data_dir, &dest).unwrap();

        assert!(dest.exists());
        let file = std::fs::File::open(&dest).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec!["adm-sfa.sqlite", "documents/2026-01-01_purchase-1_ad.jpg"]
        );

        std::fs::remove_dir_all(&tmp).ok();
    }
}
