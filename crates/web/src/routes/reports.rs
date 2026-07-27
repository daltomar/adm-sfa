use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use rust_decimal::Decimal;
use serde::Deserialize;

use adm_sfa_core::db::queries::{
    brl_ledger as brl_qry, documents as documents_qry, donors as donors_qry, eur_ledger as eur_qry,
    inventory as inventory_qry, outbound as outbound_qry,
};
use adm_sfa_core::format;
use adm_sfa_core::model::donor::{Donor, PhysicalDonation};
use adm_sfa_core::model::inventory::{InventoryItemRow, SourceType};
use adm_sfa_core::model::outbound::{OutboundEventRow, RecipientProject};
use adm_sfa_core::model::transaction::{BrlTxRow, EurTxRow};
use adm_sfa_core::reporting::{
    brl_summary, brl_tx_description, build_audit_entries, build_donor_rows, eur_summary,
    eur_tx_description, in_range, outbound_audit_text,
};

use crate::state::AppState;
use crate::templates::{HtmlTemplate, ReportsTemplate, SummaryLine, TabLink};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/reports", get(index))
        .route("/reports/export.csv", get(export_csv))
        .route("/reports/export.pdf", get(export_pdf))
}

/// Distinguishes concurrent exports landing on the same temp path — same
/// pattern (and same reason) as `purchases.rs`'s `UPLOAD_COUNTER`.
static EXPORT_COUNTER: AtomicU64 = AtomicU64::new(0);

const TABS: &[(&str, &str)] = &[
    ("donors", "Donor breakdown"),
    ("eur", "EUR ledger"),
    ("brl", "BRL ledger"),
    ("inventory", "Inventory"),
    ("outbound", "Outbound"),
    ("audit", "Audit trail"),
];

/// Everything the report tables/summaries are built from — one query pass
/// per request, mirroring desktop's `ReportsView::reload`. `web` has no
/// long-lived view state to cache this in between requests (unlike
/// desktop's `needs_reload` flag), so this just runs fresh every time; at
/// this data scale that's the same tradeoff already accepted throughout
/// this crate's other sections (re-fetching full lists per request).
struct ReportsData {
    donors: Vec<Donor>,
    donations: Vec<PhysicalDonation>,
    eur_rows: Vec<EurTxRow>,
    brl_rows: Vec<BrlTxRow>,
    inventory_rows: Vec<InventoryItemRow>,
    outbound_rows: Vec<OutboundEventRow>,
    recipient_projects: Vec<RecipientProject>,
    doc_counts: HashMap<(String, i64), i64>,
}

fn load(conn: &rusqlite::Connection) -> ReportsData {
    ReportsData {
        donors: donors_qry::list(conn).unwrap_or_default(),
        donations: donors_qry::list_donations(conn).unwrap_or_default(),
        eur_rows: eur_qry::list(conn).unwrap_or_default(),
        brl_rows: brl_qry::list(conn).unwrap_or_default(),
        inventory_rows: inventory_qry::list(conn).unwrap_or_default(),
        outbound_rows: outbound_qry::list(conn).unwrap_or_default(),
        recipient_projects: outbound_qry::list_recipient_projects(conn).unwrap_or_default(),
        doc_counts: documents_qry::counts_by_record(conn).unwrap_or_default(),
    }
}

fn donors_table(
    data: &ReportsData,
    from: &str,
    to: &str,
    locale: &str,
    for_csv: bool,
) -> (Vec<String>, Vec<Vec<String>>) {
    let headers = vec![
        "Donor".to_string(),
        "Cash gifts".to_string(),
        "Cash total".to_string(),
        "Items donated".to_string(),
    ];
    let rows = build_donor_rows(&data.donors, &data.eur_rows, &data.donations, from, to)
        .into_iter()
        .map(|r| {
            let cash_total = if for_csv {
                format::csv_amount(r.cash_total)
            } else {
                format::amount_in(r.cash_total, locale)
            };
            vec![
                r.name,
                r.cash_count.to_string(),
                cash_total,
                r.item_count.to_string(),
            ]
        })
        .collect();
    (headers, rows)
}

fn eur_table(
    data: &ReportsData,
    from: &str,
    to: &str,
    locale: &str,
    for_csv: bool,
) -> (Vec<String>, Vec<Vec<String>>) {
    let headers = vec![
        "Date".to_string(),
        "Type".to_string(),
        "Description".to_string(),
        "Amount".to_string(),
    ];
    let rows = data
        .eur_rows
        .iter()
        .filter(|r| in_range(&r.date, from, to))
        .map(|r| {
            let description = eur_tx_description(r);
            let sign = if r.tx_type.is_inflow() { "" } else { "-" };
            // `r.tx_type.label()` below resolves through `rust_i18n`'s
            // ambient locale, not the `locale` parameter — same disclosed,
            // pre-existing scope limit as `inventory_table`'s status/
            // location labels and `reporting::AuditEntry.ledger_kind`
            // (see its doc comment in crates/core/src/reporting.rs). A
            // `locale=de` export's Date/Amount columns follow `locale`
            // correctly; the Type column follows whatever locale happens
            // to be compiled in, since `web` never calls
            // `rust_i18n::set_locale`. Not introduced by this port —
            // desktop has the identical gap — just flagging it here too
            // since nothing previously disclosed it for this call site.
            let date = if for_csv {
                r.date.clone()
            } else {
                format::date_in(&r.date, locale)
            };
            let amount = if for_csv {
                format::csv_amount(r.amount)
            } else {
                format::amount_in(r.amount, locale)
            };
            vec![
                date,
                r.tx_type.label(),
                description,
                format!("{sign}{amount}"),
            ]
        })
        .collect();
    (headers, rows)
}

fn brl_table(
    data: &ReportsData,
    from: &str,
    to: &str,
    locale: &str,
    for_csv: bool,
) -> (Vec<String>, Vec<Vec<String>>) {
    let headers = vec![
        "Date".to_string(),
        "Type".to_string(),
        "Description".to_string(),
        "Amount".to_string(),
    ];
    let rows = data
        .brl_rows
        .iter()
        .filter(|r| in_range(&r.date, from, to))
        .map(|r| {
            let description = brl_tx_description(r);
            let sign = if r.tx_type.is_inflow() { "" } else { "-" };
            // See eur_table's comment above — r.tx_type.label() has the
            // same disclosed ambient-locale limitation here.
            let date = if for_csv {
                r.date.clone()
            } else {
                format::date_in(&r.date, locale)
            };
            let amount = if for_csv {
                format::csv_amount(r.amount)
            } else {
                format::amount_in(r.amount, locale)
            };
            vec![
                date,
                r.tx_type.label(),
                description,
                format!("{sign}{amount}"),
            ]
        })
        .collect();
    (headers, rows)
}

fn inventory_table(data: &ReportsData, locale: &str) -> (Vec<String>, Vec<Vec<String>>) {
    let headers = vec![
        "Name".to_string(),
        "Category".to_string(),
        "Status".to_string(),
        "Location".to_string(),
        "Source type".to_string(),
        "Source".to_string(),
    ];
    let _ = locale; // status/location labels aren't locale-threaded in core yet, same as desktop.
    let rows = data
        .inventory_rows
        .iter()
        .map(|item| {
            let source_type = match item.source_type {
                SourceType::Donation => "Donation",
                SourceType::Purchase => "Purchase",
            };
            vec![
                item.name.clone(),
                item.category_name.clone(),
                item.status.label(),
                item.location.label(),
                source_type.to_string(),
                item.source_desc.clone(),
            ]
        })
        .collect();
    (headers, rows)
}

fn outbound_table(
    data: &ReportsData,
    from: &str,
    to: &str,
    recipient_filter: Option<i64>,
    locale: &str,
    for_csv: bool,
) -> (Vec<String>, Vec<Vec<String>>) {
    let headers = vec![
        "Date".to_string(),
        "Recipient".to_string(),
        "Items".to_string(),
        "Cash".to_string(),
    ];
    let rows = data
        .outbound_rows
        .iter()
        .filter(|e| in_range(&e.date, from, to))
        .filter(|e| recipient_filter.is_none_or(|rid| e.recipient_project_id == rid))
        .map(|e| {
            let date = if for_csv {
                e.date.clone()
            } else {
                format::date_in(&e.date, locale)
            };
            let cash = e
                .cash_amount_brl
                .map(|c| {
                    if for_csv {
                        format::csv_amount(c)
                    } else {
                        format::amount_in(c, locale)
                    }
                })
                .unwrap_or_default();
            vec![
                date,
                e.recipient_name.clone(),
                e.item_count.to_string(),
                cash,
            ]
        })
        .collect();
    (headers, rows)
}

fn audit_table(
    data: &ReportsData,
    from: &str,
    to: &str,
    recipient_filter: Option<i64>,
    locale: &str,
    for_csv: bool,
) -> (Vec<String>, Vec<Vec<String>>) {
    let headers = vec![
        "Date".to_string(),
        "Ledger".to_string(),
        "Type".to_string(),
        "Description".to_string(),
        "Amount".to_string(),
        "Docs".to_string(),
    ];
    let fmt_amount = |v: Decimal| {
        if for_csv {
            format::csv_amount(v)
        } else {
            format::amount_in(v, locale)
        }
    };
    let entries = build_audit_entries(
        &data.eur_rows,
        &data.brl_rows,
        &data.outbound_rows,
        &data.doc_counts,
        from,
        to,
        recipient_filter,
    );
    let rows = entries
        .into_iter()
        .map(|e| {
            let date = if for_csv {
                e.date
            } else {
                format::date_in(&e.date, locale)
            };
            let (ledger, kind, description) = match &e.outbound {
                Some(info) => outbound_audit_text(info, locale, fmt_amount),
                None => {
                    let (ledger, kind) = e.ledger_kind.unwrap_or_default();
                    (ledger, kind, e.description)
                }
            };
            vec![
                date,
                ledger,
                kind,
                description,
                e.amount
                    .map(|a| format!("{}{}{}", a.sign, a.symbol, fmt_amount(a.value)))
                    .unwrap_or_default(),
                if e.docs > 0 {
                    e.docs.to_string()
                } else {
                    String::new()
                },
            ]
        })
        .collect();
    (headers, rows)
}

fn eur_summary_lines(data: &ReportsData, from: &str, to: &str, locale: &str) -> Vec<SummaryLine> {
    let s = eur_summary(&data.eur_rows, from, to);
    let fmt = |v: Decimal| format::amount_in(v, locale);
    vec![
        SummaryLine {
            label: "Starting balance".to_string(),
            value: format!("\u{20ac} {}", fmt(s.starting_balance)),
        },
        SummaryLine {
            label: format!("Donations ({})", s.donation_count),
            value: format!("\u{20ac} {}", fmt(s.donation_total)),
        },
        SummaryLine {
            label: format!("Self-funding ({})", s.self_funding_count),
            value: format!("\u{20ac} {}", fmt(s.self_funding_total)),
        },
        SummaryLine {
            label: format!("Purchases ({})", s.purchase_count),
            value: format!("-\u{20ac} {}", fmt(s.purchase_total)),
        },
        SummaryLine {
            label: format!("Transfers to BRL ({})", s.transfer_count),
            value: format!("-\u{20ac} {}", fmt(s.transfer_total)),
        },
        SummaryLine {
            label: "Net".to_string(),
            value: format!("\u{20ac} {}", fmt(s.net)),
        },
        SummaryLine {
            label: "Ending balance".to_string(),
            value: format!("\u{20ac} {}", fmt(s.ending_balance)),
        },
    ]
}

fn brl_summary_lines(data: &ReportsData, from: &str, to: &str, locale: &str) -> Vec<SummaryLine> {
    let s = brl_summary(&data.brl_rows, from, to);
    let fmt = |v: Decimal| format::amount_in(v, locale);
    vec![
        SummaryLine {
            label: "Starting balance".to_string(),
            value: format!("R$ {}", fmt(s.starting_balance)),
        },
        SummaryLine {
            label: format!("Transfers in ({})", s.transfer_in_count),
            value: format!("R$ {}", fmt(s.transfer_in_total)),
        },
        SummaryLine {
            label: format!("Purchases ({})", s.purchase_count),
            value: format!("-R$ {}", fmt(s.purchase_total)),
        },
        SummaryLine {
            label: format!("Cash gifts ({})", s.gift_count),
            value: format!("-R$ {}", fmt(s.gift_total)),
        },
        SummaryLine {
            label: "Net".to_string(),
            value: format!("R$ {}", fmt(s.net)),
        },
        SummaryLine {
            label: "Ending balance".to_string(),
            value: format!("R$ {}", fmt(s.ending_balance)),
        },
    ]
}

/// All six sections, in the fixed order `build_pdf_sections` (desktop)
/// uses — the PDF export always includes everything regardless of which
/// tab is active on screen, matching desktop's own "PDF exports the whole
/// report" behavior (only CSV export is scoped to the current tab).
fn all_sections(
    data: &ReportsData,
    from: &str,
    to: &str,
    recipient_filter: Option<i64>,
    locale: &str,
) -> Vec<reports::pdf::PdfSection> {
    let (dh, dr) = donors_table(data, from, to, locale, false);
    let (eh, er) = eur_table(data, from, to, locale, false);
    let (bh, br) = brl_table(data, from, to, locale, false);
    let (ih, ir) = inventory_table(data, locale);
    let (oh, or_) = outbound_table(data, from, to, recipient_filter, locale, false);
    let (ah, ar) = audit_table(data, from, to, recipient_filter, locale, false);
    vec![
        reports::pdf::PdfSection {
            title: "Donor breakdown".to_string(),
            headers: dh,
            rows: dr,
        },
        reports::pdf::PdfSection {
            title: "EUR ledger".to_string(),
            headers: eh,
            rows: er,
        },
        reports::pdf::PdfSection {
            title: "BRL ledger".to_string(),
            headers: bh,
            rows: br,
        },
        reports::pdf::PdfSection {
            title: "Inventory".to_string(),
            headers: ih,
            rows: ir,
        },
        reports::pdf::PdfSection {
            title: "Outbound".to_string(),
            headers: oh,
            rows: or_,
        },
        reports::pdf::PdfSection {
            title: "Audit trail".to_string(),
            headers: ah,
            rows: ar,
        },
    ]
}

#[derive(Deserialize)]
struct ReportsQuery {
    #[serde(default)]
    tab: String,
    #[serde(default)]
    from: String,
    #[serde(default)]
    to: String,
    #[serde(default)]
    recipient: String,
}

fn parsed_recipient(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        s.parse().ok()
    }
}

fn query_string(tab: &str, from: &str, to: &str, recipient: Option<i64>) -> String {
    let mut s = form_urlencoded::Serializer::new(String::new());
    s.append_pair("tab", tab);
    s.append_pair("from", from);
    s.append_pair("to", to);
    if let Some(r) = recipient {
        s.append_pair("recipient", &r.to_string());
    }
    s.finish()
}

async fn index(State(state): State<AppState>, Query(q): Query<ReportsQuery>) -> impl IntoResponse {
    let conn = state.conn();
    let data = load(&conn);

    let tab = if TABS.iter().any(|(slug, _)| *slug == q.tab) {
        q.tab.as_str()
    } else {
        "donors"
    };
    let recipient_id = parsed_recipient(&q.recipient);

    let (summary, headers, rows) = match tab {
        "eur" => {
            let (h, r) = eur_table(&data, &q.from, &q.to, "en", false);
            (eur_summary_lines(&data, &q.from, &q.to, "en"), h, r)
        }
        "brl" => {
            let (h, r) = brl_table(&data, &q.from, &q.to, "en", false);
            (brl_summary_lines(&data, &q.from, &q.to, "en"), h, r)
        }
        "inventory" => {
            let (h, r) = inventory_table(&data, "en");
            (Vec::new(), h, r)
        }
        "outbound" => {
            let (h, r) = outbound_table(&data, &q.from, &q.to, recipient_id, "en", false);
            (Vec::new(), h, r)
        }
        "audit" => {
            let (h, r) = audit_table(&data, &q.from, &q.to, recipient_id, "en", false);
            (Vec::new(), h, r)
        }
        _ => {
            let (h, r) = donors_table(&data, &q.from, &q.to, "en", false);
            (Vec::new(), h, r)
        }
    };

    let tabs = TABS
        .iter()
        .map(|(slug, label)| TabLink {
            label,
            href: format!(
                "/reports?{}",
                query_string(slug, &q.from, &q.to, recipient_id)
            ),
            active: *slug == tab,
        })
        .collect();

    let recipients = data
        .recipient_projects
        .iter()
        .map(|p| (p.id, p.name.clone(), recipient_id == Some(p.id)))
        .collect();

    HtmlTemplate(ReportsTemplate {
        tabs,
        active_tab: TABS
            .iter()
            .find(|(s, _)| *s == tab)
            .map(|(s, _)| *s)
            .unwrap_or("donors"),
        date_from: q.from.clone(),
        date_to: q.to.clone(),
        recipient_id,
        recipients,
        summary,
        headers,
        rows,
        error: None,
    })
}

#[derive(Deserialize)]
struct ExportQuery {
    #[serde(default)]
    tab: String,
    #[serde(default)]
    from: String,
    #[serde(default)]
    to: String,
    #[serde(default)]
    recipient: String,
    #[serde(default = "default_locale")]
    locale: String,
}

fn default_locale() -> String {
    "en".to_string()
}

async fn export_csv(State(state): State<AppState>, Query(q): Query<ExportQuery>) -> Response {
    // Same allowlist check `index` already does — `tab_slug` below feeds
    // straight into a filesystem path, so an unvalidated value here would
    // let a `tab=../../../whatever` query param steer `reports::csv::write`
    // outside the temp dir (PathBuf::join does not neutralize `..`
    // segments). Defaulting an unrecognized tab to "donors" rather than
    // rejecting the request matches `index`'s own leniency.
    let tab_slug = TABS
        .iter()
        .map(|(slug, _)| *slug)
        .find(|slug| *slug == q.tab)
        .unwrap_or("donors");

    let conn = state.conn();
    let data = load(&conn);
    let recipient_id = parsed_recipient(&q.recipient);

    let (headers, rows) = match tab_slug {
        "eur" => eur_table(&data, &q.from, &q.to, &q.locale, true),
        "brl" => brl_table(&data, &q.from, &q.to, &q.locale, true),
        "inventory" => inventory_table(&data, &q.locale),
        "outbound" => outbound_table(&data, &q.from, &q.to, recipient_id, &q.locale, true),
        "audit" => audit_table(&data, &q.from, &q.to, recipient_id, &q.locale, true),
        _ => donors_table(&data, &q.from, &q.to, &q.locale, true),
    };

    let unique = EXPORT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dest = std::env::temp_dir().join(format!(
        "adm-sfa-web-report-{tab_slug}-{}-{unique}.csv",
        std::process::id()
    ));
    if let Err(e) = reports::csv::write(&dest, &headers, &rows) {
        let _ = std::fs::remove_file(&dest);
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    let result = std::fs::read(&dest);
    let _ = std::fs::remove_file(&dest);
    let bytes = match result {
        Ok(b) => b,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{tab_slug}.csv\""),
            ),
        ],
        bytes,
    )
        .into_response()
}

async fn export_pdf(State(state): State<AppState>, Query(q): Query<ExportQuery>) -> Response {
    let conn = state.conn();
    let data = load(&conn);
    let recipient_id = parsed_recipient(&q.recipient);

    let generated_iso = chrono::Local::now().format("%Y-%m-%d").to_string();
    let generated = format::date_in(&generated_iso, &q.locale);
    let from_display = format::date_in(&q.from, &q.locale);
    let to_display = format::date_in(&q.to, &q.locale);
    let sections = all_sections(&data, &q.from, &q.to, recipient_id, &q.locale);

    let unique = EXPORT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dest = std::env::temp_dir().join(format!(
        "adm-sfa-web-report-{}-{unique}.pdf",
        std::process::id()
    ));
    if let Err(e) = reports::pdf::export(&dest, &generated, &from_display, &to_display, &sections) {
        let _ = std::fs::remove_file(&dest);
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    let result = std::fs::read(&dest);
    let _ = std::fs::remove_file(&dest);
    let bytes = match result {
        Ok(b) => b,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    (
        [
            (header::CONTENT_TYPE, "application/pdf".to_string()),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"adm-sfa-report.pdf\"".to_string(),
            ),
        ],
        bytes,
    )
        .into_response()
}
