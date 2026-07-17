use eframe::egui;
use std::path::PathBuf;

use crate::ui;
use crate::ui::views::brl_ledger::BrlLedgerView;
use crate::ui::views::donors::DonorsView;
use crate::ui::views::eur_ledger::EurLedgerView;
use crate::ui::views::inventory::InventoryView;
use crate::ui::views::outbound::OutboundView;
use crate::ui::views::purchases::PurchasesView;
use crate::ui::views::reports::ReportsView;
use crate::ui::views::settings::SettingsView;
use crate::ui::views::transfers::TransfersView;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Dashboard,
    Donors,
    EurLedger,
    BrlLedger,
    Purchases,
    Transfers,
    Inventory,
    Outbound,
    Reports,
    Settings,
}

pub struct App {
    pub section: Section,
    prev_section: Section,
    pub db: rusqlite::Connection,
    pub data_dir: PathBuf,
    donors_view: DonorsView,
    purchases_view: PurchasesView,
    eur_ledger_view: EurLedgerView,
    brl_ledger_view: BrlLedgerView,
    transfers_view: TransfersView,
    inventory_view: InventoryView,
    outbound_view: OutboundView,
    reports_view: ReportsView,
    settings_view: SettingsView,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>, data_dir: PathBuf) -> Self {
        let db = crate::db::open_db(&data_dir).expect("failed to open database");

        // Apply the saved UI language at startup (SPEC.md §6.1/§6.2) — seeded
        // to "en" by seed_default_settings if never set. Live switching
        // (rust_i18n::set_locale) also happens from the Settings selector
        // without restarting; this just makes the choice persist across runs.
        if let Ok(Some(locale)) = crate::db::queries::settings::get(&db, "ui_locale") {
            rust_i18n::set_locale(&locale);
        }

        Self {
            section: Section::Dashboard,
            prev_section: Section::Dashboard,
            db,
            data_dir,
            donors_view: DonorsView::default(),
            purchases_view: PurchasesView::default(),
            eur_ledger_view: EurLedgerView::default(),
            brl_ledger_view: BrlLedgerView::default(),
            transfers_view: TransfersView::default(),
            inventory_view: InventoryView::default(),
            outbound_view: OutboundView::default(),
            reports_view: ReportsView::default(),
            settings_view: SettingsView::default(),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::left("nav").show(ui, |ui| {
            ui::sidebar::show(ui, &mut self.section);
        });

        if self.section != self.prev_section {
            match self.section {
                Section::Donors => self.donors_view.invalidate(),
                Section::EurLedger => self.eur_ledger_view.invalidate(),
                Section::BrlLedger => self.brl_ledger_view.invalidate(),
                Section::Purchases => self.purchases_view.invalidate(),
                Section::Transfers => self.transfers_view.invalidate(),
                Section::Inventory => self.inventory_view.invalidate(),
                Section::Outbound => self.outbound_view.invalidate(),
                Section::Reports => self.reports_view.invalidate(),
                Section::Settings => self.settings_view.invalidate(),
                Section::Dashboard => {}
            }
            self.prev_section = self.section;
        }

        egui::CentralPanel::default().show(ui, |ui| match self.section {
            Section::Dashboard => ui::views::dashboard::show(ui),
            Section::Donors => self.donors_view.show(ui, &self.db),
            Section::EurLedger => self.eur_ledger_view.show(ui, &self.db),
            Section::BrlLedger => self.brl_ledger_view.show(ui, &self.db),
            Section::Purchases => self.purchases_view.show(ui, &self.db, &self.data_dir),
            Section::Transfers => self.transfers_view.show(ui, &self.db, &self.data_dir),
            Section::Inventory => self.inventory_view.show(ui, &self.db, &self.data_dir),
            Section::Outbound => self.outbound_view.show(ui, &self.db),
            Section::Reports => self.reports_view.show(ui, &self.db),
            Section::Settings => self.settings_view.show(ui, &self.db, &self.data_dir),
        });
    }
}
