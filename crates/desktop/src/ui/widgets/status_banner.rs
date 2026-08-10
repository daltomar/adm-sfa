use eframe::egui;

/// Renders a one-shot save-outcome banner if `status` holds one — a no-op
/// otherwise, so every view can call this unconditionally on every frame
/// without its own `if let`. Generalizes the ad hoc `Option<Result<String,
/// String>>` + `colored_label` pattern `settings.rs` already used locally
/// for `screenshot_status`/`backup_status` into something the CRUD views
/// can share too.
///
/// The `Err` arm exists for symmetry with that existing convention (and
/// with `web`'s `FlashKind::Error`) — no in-scope Save action ever
/// constructs one today, since every view's existing `error: Option<String>`
/// field/`colored_label` already covers the failure case.
pub fn show(ui: &mut egui::Ui, status: &Option<Result<String, String>>) {
    let Some(status) = status else { return };
    let (color, text) = match status {
        Ok(msg) => (egui::Color32::DARK_GREEN, msg),
        Err(msg) => (egui::Color32::RED, msg),
    };
    ui.colored_label(color, text);
    ui.add_space(4.0);
}
