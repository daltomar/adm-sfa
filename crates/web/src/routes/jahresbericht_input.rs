use adm_sfa_core::format;
use adm_sfa_core::statutory::StatutoryReportData;
use reports::statutory::{OutboundEventLine, StatutoryInput};

/// Converts a `StatutoryReportData` (from core) into a `StatutoryInput`
/// (for the reports crate) with all amounts and dates pre-formatted in
/// the given locale. Shared between the web route and can be used from
/// the desktop view.
pub fn build_input(data: &StatutoryReportData, locale: &str) -> StatutoryInput {
    let ea = &data.einnahmen_ausgaben;
    let v = &data.vermoegen;
    let d = &data.draft;

    StatutoryInput {
        org_name: data.org_name.clone(),
        org_address: data.org_address.clone(),
        org_tax_number: data.org_tax_number.clone(),
        year: data.year.to_string(),
        generated: chrono::Local::now().format("%d.%m.%Y").to_string(),
        donations: format::amount_in(ea.donations, locale),
        self_funding: format::amount_in(ea.other_income, locale),
        project_expenses: format::amount_in(ea.project_expenses, locale),
        income_total: format::amount_in(ea.income_total, locale),
        expense_total: format::amount_in(ea.expense_total, locale),
        surplus: format::amount_in(ea.surplus, locale),
        bank_balance: format::amount_in(v.bank_balance, locale),
        inventory_value: format::amount_in(v.inventory_value, locale),
        total_assets: format::amount_in(v.total_assets, locale),
        member_count: d.member_count.to_string(),
        meeting_date: d.meeting_date.clone(),
        meeting_time_from: d.meeting_time_from.clone(),
        meeting_time_to: d.meeting_time_to.clone(),
        outbound_events: data
            .outbound_events
            .iter()
            .map(|e| OutboundEventLine {
                date: format::date_in(&e.date, locale),
                recipient: e.recipient_name.clone(),
                item_count: e.item_count.to_string(),
            })
            .collect(),
        tb_activities_paras: split_paragraphs(&d.tb_activities),
        tb_continuous_paras: split_paragraphs(&d.tb_continuous),
        tb_outlook_paras: split_paragraphs(&d.tb_outlook),
        pk_activities_paras: split_paragraphs(&d.pk_activities_next_year),
        pk_decisions_paras: split_paragraphs(&d.pk_decisions),
    }
}

fn split_paragraphs(text: &str) -> Vec<String> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    text.split("\n\n")
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}
