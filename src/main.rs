mod app;
mod backup;
mod db;
mod docs_fs;
mod format;
mod model;
mod money;
mod reports;
mod screenshot;
mod ui;

use std::path::{Path, PathBuf};

// Compile-time locale catalogues, embedded into the binary (SPEC.md §6,
// stack-plan.md "Localisation (i18n)"). English is both the source and the
// fallback locale (SPEC.md §6.6 / CLAUDE.md T5): a missing key in de/pt-BR
// falls back to its en.yml value rather than rendering blank.
rust_i18n::i18n!("locales", fallback = "en");

fn main() -> eframe::Result<()> {
    let data_dir = parse_data_dir();
    ensure_dirs(&data_dir);

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Skateboard für alle")
            .with_inner_size([1200.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native(
        "adm-sfa",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, data_dir)))),
    )
}

fn parse_data_dir() -> PathBuf {
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--data-dir" && i + 1 < args.len() {
            return PathBuf::from(&args[i + 1]);
        }
        i += 1;
    }
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("adm-sfa")
}

fn ensure_dirs(data_dir: &Path) {
    std::fs::create_dir_all(data_dir.join("documents/_deleted"))
        .expect("failed to create data directories");
}
