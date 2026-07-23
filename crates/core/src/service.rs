//! Operation-shaped functions — the entry points `desktop` (and, later,
//! `web`) call instead of reaching into `db::queries` directly, per
//! CLAUDE.md phase 4. Each one wraps whatever orchestration a single
//! business operation actually needs (a multi-step sequence, a validation
//! rule, or just a named, discoverable seam) so both front-ends get
//! identical behavior without re-implementing it themselves.

use std::path::Path;

use rusqlite::{Connection, Result};

use crate::db::queries::{
    documents as documents_qry, outbound as outbound_qry, purchases as purchases_qry,
};
use crate::docs_fs;
use crate::model::outbound::OutboundEventDraft;
use crate::model::purchase::{PurchaseDraft, PurchaseStatus};

pub fn create_purchase(conn: &Connection, draft: &PurchaseDraft) -> Result<i64> {
    purchases_qry::insert(conn, draft)
}

/// Transitions a negotiating purchase to bought, saving whatever else is
/// currently in `draft` along with it — the desktop form's "Mark as
/// bought" button acts on the same in-progress edit as the rest of the
/// form, not just a bare status flip, so this takes the full draft rather
/// than re-fetching the persisted row. `purchases_qry::update`'s own
/// terminal guard (bought can never revert) still applies underneath.
pub fn mark_purchase_bought(conn: &Connection, id: i64, draft: &PurchaseDraft) -> Result<()> {
    let mut bought = draft.clone();
    bought.status = PurchaseStatus::Bought;
    purchases_qry::update(conn, id, &bought)
}

/// Hard-deletes a negotiating purchase, soft-deleting any attached
/// documents first (SPEC.md §3.6) — never orphaned or hard-deleted
/// alongside the purchase row. Stops at the first failure (matching the
/// behavior this was extracted from), but each document removal now goes
/// through `docs_fs::remove_document`'s safer file-then-DB ordering
/// instead of the DB-then-file order every call site used to duplicate
/// inline.
pub fn drop_negotiating_purchase(
    conn: &Connection,
    documents_dir: &Path,
    purchase_id: i64,
) -> std::result::Result<(), String> {
    let docs =
        documents_qry::list_for_record(conn, "purchase", purchase_id).map_err(|e| e.to_string())?;
    for doc in docs {
        docs_fs::remove_document(conn, documents_dir, doc.id, &doc.filename)?;
    }
    purchases_qry::delete(conn, purchase_id).map_err(|e| e.to_string())
}

/// Resolves which ISO date to file a document under: the currently-typed
/// draft date if it parses, else the record's own persisted date (covers
/// attaching a document mid-edit while the date field is momentarily
/// unparseable), else today. Filenames must stay ISO-sortable (T4)
/// regardless of what the user has typed — this is the shared
/// implementation behind what used to be two verbatim-duplicated 3-tier
/// fallbacks (`purchases.rs`, `transfers.rs`).
pub fn resolve_filing_date(draft_date_input: &str, persisted_iso_date: Option<&str>) -> String {
    crate::date::parse_date_input(draft_date_input)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .or_else(|| persisted_iso_date.map(|s| s.to_string()))
        .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string())
}

/// Attaches a document to a record, resolving the filing date the same way
/// regardless of caller (see `resolve_filing_date`) before delegating to
/// `docs_fs::file_document`.
#[allow(clippy::too_many_arguments)]
pub fn attach_document(
    conn: &Connection,
    documents_dir: &Path,
    src: &Path,
    draft_date_input: &str,
    persisted_iso_date: Option<&str>,
    record: (&str, i64),
    label: &str,
    existing: &[String],
) -> std::result::Result<String, String> {
    let date = resolve_filing_date(draft_date_input, persisted_iso_date);
    docs_fs::file_document(conn, documents_dir, src, &date, record, label, existing)
}

/// Records an outbound donation event — one or more inventory items and/or
/// a cash gift to a recipient project. `outbound_qry::insert` enforces "at
/// least one item or a cash amount" and the item-availability guard
/// authoritatively (see CLAUDE.md phase 2); this wrapper exists so callers
/// use one named operation instead of reaching into `db::queries` directly.
pub fn donate_items(
    conn: &Connection,
    draft: &OutboundEventDraft,
    item_ids: &[i64],
) -> Result<i64> {
    outbound_qry::insert(conn, draft, item_ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::purchase::Currency;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../schema.sql")).unwrap();
        conn
    }

    fn purchase_draft() -> PurchaseDraft {
        PurchaseDraft {
            date: "2026-01-01".to_string(),
            currency: Currency::Eur,
            cost_str: "50.00".to_string(),
            channel: "Kleinanzeigen".to_string(),
            seller_info: String::new(),
            multiple_items: false,
            status: PurchaseStatus::Negotiating,
        }
    }

    #[test]
    fn mark_purchase_bought_forces_status_and_keeps_other_edits() {
        let conn = test_db();
        let id = create_purchase(&conn, &purchase_draft()).unwrap();

        let mut edited = purchase_draft();
        edited.channel = "Updated channel".to_string();
        mark_purchase_bought(&conn, id, &edited).unwrap();

        let (status, channel): (String, String) = conn
            .query_row(
                "SELECT status, channel FROM purchase WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "bought");
        assert_eq!(channel, "Updated channel");
    }

    #[test]
    fn drop_negotiating_purchase_soft_deletes_docs_then_hard_deletes_the_row() {
        let conn = test_db();
        let id = create_purchase(&conn, &purchase_draft()).unwrap();

        let tmp = std::env::temp_dir().join(format!(
            "adm-sfa-drop-negotiating-test-{}",
            std::process::id()
        ));
        let documents_dir = tmp.join("documents");
        std::fs::create_dir_all(documents_dir.join("_deleted")).unwrap();
        let src = tmp.join("chat.png");
        std::fs::write(&src, b"fake").unwrap();
        docs_fs::file_document(
            &conn,
            &documents_dir,
            &src,
            "2026-01-01",
            ("purchase", id),
            "chat",
            &[],
        )
        .unwrap();

        drop_negotiating_purchase(&conn, &documents_dir, id).unwrap();

        let purchase_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM purchase WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(purchase_count, 0);
        let active_docs = documents_qry::list_for_record(&conn, "purchase", id).unwrap();
        assert!(active_docs.is_empty());

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn resolve_filing_date_prefers_the_parsed_draft_date() {
        assert_eq!(
            resolve_filing_date("16.07.2026", Some("2020-01-01")),
            "2026-07-16"
        );
    }

    #[test]
    fn resolve_filing_date_falls_back_to_the_persisted_date_when_draft_is_unparseable() {
        assert_eq!(
            resolve_filing_date("not a date", Some("2020-01-01")),
            "2020-01-01"
        );
    }

    #[test]
    fn resolve_filing_date_falls_back_to_today_when_nothing_else_is_available() {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        assert_eq!(resolve_filing_date("", None), today);
    }

    #[test]
    fn donate_items_rejects_an_empty_gift() {
        let conn = test_db();
        let rp_id = outbound_qry::insert_recipient_project(
            &conn,
            &crate::model::outbound::RecipientProjectDraft {
                name: "Test Project".to_string(),
                contact_info: String::new(),
                location: String::new(),
                active: true,
            },
        )
        .unwrap();
        let draft = OutboundEventDraft {
            date: "2026-01-01".to_string(),
            recipient_project_id: Some(rp_id),
            cash_amount_brl_str: String::new(),
            notes: String::new(),
        };
        assert!(donate_items(&conn, &draft, &[]).is_err());
    }
}
