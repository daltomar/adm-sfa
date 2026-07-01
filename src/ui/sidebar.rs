use crate::app::Section;
use eframe::egui;

const SECTIONS: &[(Section, &str)] = &[
    (Section::Dashboard, "Dashboard"),
    (Section::Donors,    "Donors"),
    (Section::EurLedger, "EUR Ledger"),
    (Section::BrlLedger, "BRL Ledger"),
    (Section::Purchases, "Purchases"),
    (Section::Transfers, "Transfers"),
    (Section::Inventory, "Inventory"),
    (Section::Outbound,  "Outbound"),
    (Section::Reports,   "Reports"),
    (Section::Settings,  "Settings"),
];

pub fn show(ui: &mut egui::Ui, current: &mut Section) {
    ui.heading("adm-sfa");
    ui.separator();
    for &(section, label) in SECTIONS {
        if ui.selectable_label(*current == section, label).clicked() {
            *current = section;
        }
    }
}
