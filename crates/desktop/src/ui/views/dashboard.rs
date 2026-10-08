use eframe::egui;
use rusqlite::Connection;
use rust_i18n::t;

use adm_sfa_core::dashboard::DashboardData;
use adm_sfa_core::format;

pub enum DashboardNavTarget {
    EurLedger,
    BrlLedger,
    Inventory,
    Outbound,
    Purchases,
}

#[derive(Default)]
pub struct DashboardView {
    data: Option<DashboardData>,
    nav_request: Option<DashboardNavTarget>,
}

impl DashboardView {
    pub fn invalidate(&mut self) {
        self.data = None;
    }

    pub fn take_nav_request(&mut self) -> Option<DashboardNavTarget> {
        self.nav_request.take()
    }

    pub fn show(&mut self, ui: &mut egui::Ui, db: &Connection) {
        let data = self.data.get_or_insert_with(|| {
            adm_sfa_core::dashboard::load(db).unwrap_or_else(|e| {
                eprintln!("dashboard: failed to load data: {e}");
                DashboardData::default()
            })
        });

        ui.heading(t!("dashboard.heading").as_ref());
        ui.add_space(12.0);

        ui.columns(2, |cols| {
            // ── Left column ────────────────────────────────────────────
            let left = &mut cols[0];

            // Balances — link heading navigates to EUR Ledger / BRL Ledger
            egui::Frame::group(left.style()).show(left, |ui| {
                if ui.link(t!("dashboard.eur_balance").as_ref()).clicked() {
                    self.nav_request = Some(DashboardNavTarget::EurLedger);
                }
                ui.label(format!("€ {}", format::amount(data.eur_balance)));
                ui.add_space(4.0);
                if ui.link(t!("dashboard.brl_balance").as_ref()).clicked() {
                    self.nav_request = Some(DashboardNavTarget::BrlLedger);
                }
                ui.label(format!("R$ {}", format::amount(data.brl_balance)));
            });

            left.add_space(10.0);

            // Inventory snapshot — link heading navigates to Inventory
            egui::Frame::group(left.style()).show(left, |ui| {
                if ui
                    .link(t!("dashboard.inventory.heading").as_ref())
                    .clicked()
                {
                    self.nav_request = Some(DashboardNavTarget::Inventory);
                }
                ui.add_space(4.0);
                egui::Grid::new("dashboard_inventory_grid")
                    .num_columns(2)
                    .spacing([12.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(t!("dashboard.inventory.available").as_ref());
                        ui.label(data.inventory_available.to_string());
                        ui.end_row();
                        ui.label(t!("dashboard.inventory.reserved").as_ref());
                        ui.label(data.inventory_reserved.to_string());
                        ui.end_row();
                        ui.label(t!("dashboard.inventory.donated").as_ref());
                        ui.label(data.inventory_donated.to_string());
                        ui.end_row();
                    });
            });

            // ── Right column ───────────────────────────────────────────
            let right = &mut cols[1];

            // Outbound this year — link heading navigates to Outbound
            egui::Frame::group(right.style()).show(right, |ui| {
                if ui.link(t!("dashboard.outbound.heading").as_ref()).clicked() {
                    self.nav_request = Some(DashboardNavTarget::Outbound);
                }
                ui.add_space(4.0);
                egui::Grid::new("dashboard_outbound_grid")
                    .num_columns(2)
                    .spacing([12.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(t!("dashboard.outbound.items").as_ref());
                        ui.label(data.outbound_items_this_year.to_string());
                        ui.end_row();
                        ui.label(t!("dashboard.outbound.cash").as_ref());
                        ui.label(format!(
                            "R$ {}",
                            format::amount(data.outbound_cash_this_year)
                        ));
                        ui.end_row();
                    });
            });

            right.add_space(10.0);

            // Open negotiations — link heading navigates to Purchases
            egui::Frame::group(right.style()).show(right, |ui| {
                if ui
                    .link(t!("dashboard.negotiations.heading").as_ref())
                    .clicked()
                {
                    self.nav_request = Some(DashboardNavTarget::Purchases);
                }
                ui.add_space(4.0);
                let label = if data.negotiating_count == 0 {
                    t!("dashboard.negotiations.none").into_owned()
                } else if data.negotiating_count == 1 {
                    t!("dashboard.negotiations.count.one").into_owned()
                } else {
                    t!(
                        "dashboard.negotiations.count.other",
                        count = data.negotiating_count
                    )
                    .into_owned()
                };
                ui.label(label);
            });
        });
    }
}
