use eframe::egui;
use std::path::PathBuf;

use crate::ui;

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
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>, data_dir: PathBuf) -> Self {
        let db = crate::db::open_db(&data_dir).expect("failed to open database");
        Self { section: Section::Dashboard, db, data_dir }
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
                Section::Donors     => ui::views::donors::show(ui),
                Section::EurLedger  => ui::views::eur_ledger::show(ui),
                Section::BrlLedger  => ui::views::brl_ledger::show(ui),
                Section::Purchases  => ui::views::purchases::show(ui),
                Section::Transfers  => ui::views::transfers::show(ui),
                Section::Inventory  => ui::views::inventory::show(ui),
                Section::Outbound   => ui::views::outbound::show(ui),
                Section::Reports    => ui::views::reports::show(ui),
                Section::Settings   => ui::views::settings::show(ui),
            }
        });
    }
}
