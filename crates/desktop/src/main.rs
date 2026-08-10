mod app;
mod screenshot;
mod ui;

// Compile-time locale catalogues, embedded into the binary (SPEC.md §6,
// stack-plan.md "Localisation (i18n)"). English is both the source and the
// fallback locale (SPEC.md §6.6 / CLAUDE.md T5): a missing key in de/pt-BR
// falls back to its en.yml value rather than rendering blank. `locales/`
// lives at the workspace root (not inside this crate) so a future `web`
// crate can embed the same catalogue via an equivalent relative path.
rust_i18n::i18n!("../../locales", fallback = "en");

fn main() -> eframe::Result<()> {
    let data_dir = adm_sfa_core::config::parse_data_dir(std::env::args());
    adm_sfa_core::config::ensure_dirs(&data_dir);

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
