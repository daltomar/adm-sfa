#![allow(dead_code, unused_imports, unused_variables)]

mod app;
mod backup;
mod db;
mod docs_fs;
mod model;
mod reports;
mod ui;

use std::path::{Path, PathBuf};

fn main() -> eframe::Result<()> {
    let data_dir = parse_data_dir();
    ensure_dirs(&data_dir);

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("adm-sfa")
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
