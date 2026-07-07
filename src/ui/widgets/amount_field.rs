use eframe::egui;
use rust_decimal::Decimal;

/// Decimal text input that validates on focus-loss.
/// Returns the parsed value if the current text is valid.
#[allow(dead_code)]
pub struct AmountField {
    text: String,
}

#[allow(dead_code)]
impl AmountField {
    pub fn new(initial: &Decimal) -> Self {
        Self { text: initial.to_string() }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, label: &str) -> Option<Decimal> {
        ui.horizontal(|ui| {
            ui.label(label);
            ui.text_edit_singleline(&mut self.text);
        });
        self.text.trim().parse().ok()
    }
}
