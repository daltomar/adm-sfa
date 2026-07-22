use crate::app::Section;
use eframe::egui;
use rust_i18n::t;

// i18n keys, not literal labels — resolved via t!() at render time so a live
// locale switch (SPEC.md §6.1/§8.3) takes effect on the very next frame.
const SECTIONS: &[(Section, &str)] = &[
    (Section::Dashboard, "sidebar.dashboard"),
    (Section::Donors, "sidebar.donors"),
    (Section::EurLedger, "sidebar.eur_ledger"),
    (Section::BrlLedger, "sidebar.brl_ledger"),
    (Section::Purchases, "sidebar.purchases"),
    (Section::Transfers, "sidebar.transfers"),
    (Section::Inventory, "sidebar.inventory"),
    (Section::Outbound, "sidebar.outbound"),
    (Section::Reports, "sidebar.reports"),
    (Section::Settings, "sidebar.settings"),
];

pub fn show(ui: &mut egui::Ui, current: &mut Section) {
    // "adm-sfa" is the app's internal short name, not translatable UI chrome
    // — same treatment as the "Skateboard für alle" window title (main.rs).
    ui.heading("adm-sfa");
    ui.separator();
    for &(section, key) in SECTIONS {
        let label = t!(key);
        if ui
            .selectable_label(*current == section, label.as_ref())
            .clicked()
        {
            *current = section;
        }
    }
}
