use eframe::egui;
use rust_i18n::t;

pub fn show(ui: &mut egui::Ui) {
    ui.heading(t!("dashboard.heading").as_ref());
}
