use eframe::egui;
use std::path::PathBuf;

use crate::ui;
use crate::ui::views::donors::DonorsView;
use crate::ui::views::brl_ledger::BrlLedgerView;
use crate::ui::views::eur_ledger::EurLedgerView;
use crate::ui::views::purchases::PurchasesView;
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
    pub db: rusqlite::Connection,
    pub data_dir: PathBuf,
    donors_view: DonorsView,
    purchases_view: PurchasesView,
    eur_ledger_view: EurLedgerView,
    brl_ledger_view: BrlLedgerView,
    transfers_view: TransfersView,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>, data_dir: PathBuf) -> Self {
        let db = crate::db::open_db(&data_dir).expect("failed to open database");
        Self {
            section: Section::Dashboard,
            db,
            data_dir,
            donors_view: DonorsView::default(),
            purchases_view: PurchasesView::default(),
            eur_ledger_view: EurLedgerView::default(),
            brl_ledger_view: BrlLedgerView::default(),
            transfers_view: TransfersView::default(),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::left("nav").show(ui, |ui| {
            ui::sidebar::show(ui, &mut self.section);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            match self.section {
                Section::Dashboard  => ui::views::dashboard::show(ui),
                Section::Donors     => self.donors_view.show(ui, &self.db),
                Section::EurLedger  => self.eur_ledger_view.show(ui, &self.db),
                Section::BrlLedger  => self.brl_ledger_view.show(ui, &self.db),
                Section::Purchases  => self.purchases_view.show(ui, &self.db, &self.data_dir),
                Section::Transfers  => self.transfers_view.show(ui, &self.db, &self.data_dir),
                Section::Inventory  => ui::views::inventory::show(ui),
                Section::Outbound   => ui::views::outbound::show(ui),
                Section::Reports    => ui::views::reports::show(ui),
                Section::Settings   => ui::views::settings::show(ui),
            }
        });
    }
}
