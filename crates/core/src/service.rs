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
    transfers as transfers_qry,
};
use crate::docs_fs;
use crate::model::outbound::OutboundEventDraft;
use crate::model::purchase::{PurchaseDraft, PurchaseStatus};
use crate::model::transfer::TransferDraft;

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
///
/// Validates `label` against `document_label` first. Both front-ends now
/// restrict this field to a picker populated from the same table (desktop's
/// `ComboBox`, web's `<select>`), so a well-behaved caller never trips this
/// — but it's the authoritative check, not a UI nicety: nothing previously
/// stopped a caller that isn't the UI (a crafted POST to `web`, bypassing
/// its `<select>` entirely) from setting arbitrary label text, since nothing
/// validated it was even one of the configured labels. Trimmed before the
/// comparison so incidental leading/trailing whitespace — which a raw POST
/// could still introduce even though a `<select>`'s value never has any —
/// doesn't cause a spurious rejection. This doesn't change filename safety
/// (that was already just an accident of `docs_fs::generate_filename`'s
/// `{date}_{record_type}-{id}_{label}` construction, not a validated
/// guarantee, and remains a separate, still-open concern), but it does
/// close the gap where an arbitrary label string could end up stored and
/// displayed as if it were a real, configured category of document.
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
    let label = label.trim();
    let known_labels = documents_qry::labels(conn).map_err(|e| e.to_string())?;
    if !known_labels.iter().any(|l| l == label) {
        return Err(format!("Unknown document label: {label:?}"));
    }
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

/// One document staged for a batch attach — a caller-owned source path plus
/// the label to file it under, not yet written anywhere.
pub struct PendingDocument<'a> {
    pub path: &'a Path,
    pub label: &'a str,
}

/// The result of attempting to attach one staged document, in the same
/// order as the `pending` slice passed to `attach_documents` — callers
/// (both front-ends) rely on `outcomes[i]` corresponding to `pending[i]` to
/// map a failure back to the staged entry it came from.
pub struct AttachmentOutcome {
    pub source_name: String,
    pub label: String,
    pub result: std::result::Result<String, String>,
}

/// Attaches every entry in `pending` to `record`, via the existing
/// single-document `attach_document` (reusing its label validation, date
/// resolution, and `docs_fs::file_document` filename/copy/cleanup logic
/// unchanged). Never stops at the first failure — every entry is attempted
/// and every outcome collected, since a caller-visible partial failure
/// (some documents attached, one didn't) is an accepted result here, not a
/// reason to abandon the rest of the batch.
///
/// Each successful filename is folded into the de-dupe list before the next
/// entry is attempted, so two staged documents that would otherwise
/// generate the identical filename (same date, same label) get
/// disambiguated within the batch instead of the second one colliding with
/// the first on `document.filename`'s `UNIQUE` constraint.
pub fn attach_documents(
    conn: &Connection,
    documents_dir: &Path,
    draft_date_input: &str,
    persisted_iso_date: Option<&str>,
    record: (&str, i64),
    pending: &[PendingDocument<'_>],
) -> Vec<AttachmentOutcome> {
    let mut existing: Vec<String> = documents_qry::list_for_record(conn, record.0, record.1)
        .unwrap_or_default()
        .into_iter()
        .map(|d| d.filename)
        .collect();

    let mut outcomes = Vec::with_capacity(pending.len());
    for doc in pending {
        let source_name = doc
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let result = attach_document(
            conn,
            documents_dir,
            doc.path,
            draft_date_input,
            persisted_iso_date,
            record,
            doc.label,
            &existing,
        );
        if let Ok(filename) = &result {
            existing.push(filename.clone());
        }
        outcomes.push(AttachmentOutcome {
            source_name,
            label: doc.label.to_string(),
            result,
        });
    }
    outcomes
}

/// A newly-created purchase and the outcome of attempting to attach every
/// document staged alongside it. `#[must_use]` because ignoring this value
/// entirely means silently dropping any document-attach failures it
/// carries — a caller must at least look at `attachments`, not just call
/// `create_purchase_with_documents` and discard the result outright.
#[must_use]
pub struct CreatedPurchase {
    pub id: i64,
    pub attachments: Vec<AttachmentOutcome>,
}

/// Creates a purchase and attaches every staged document to it in one call.
///
/// Deliberately returns `Ok` even when some (or all) of `pending` failed to
/// attach — the purchase row is real and saved either way, and there is no
/// shared transaction wrapping the insert and the file operations (file
/// copies aren't transactional, and unwinding a saved purchase because one
/// unrelated document upload failed is not the desired behavior). `Err` is
/// reserved for the purchase itself failing to save, in which case none of
/// `pending` is attempted at all. Callers must inspect
/// `CreatedPurchase::attachments` to know whether every document actually
/// made it — do not "fix" this into a `Result` that fails on partial
/// attachment failure.
pub fn create_purchase_with_documents(
    conn: &Connection,
    documents_dir: &Path,
    draft: &PurchaseDraft,
    pending: &[PendingDocument<'_>],
) -> Result<CreatedPurchase> {
    let id = create_purchase(conn, draft)?;
    let attachments = attach_documents(
        conn,
        documents_dir,
        &draft.date,
        None,
        ("purchase", id),
        pending,
    );
    Ok(CreatedPurchase { id, attachments })
}

pub fn create_transfer(conn: &Connection, draft: &TransferDraft) -> Result<i64> {
    transfers_qry::insert(conn, draft)
}

/// A newly-created transfer and the outcome of attempting to attach every
/// document staged alongside it — same shape and same `#[must_use]`
/// reasoning as `CreatedPurchase`.
#[must_use]
pub struct CreatedTransfer {
    pub id: i64,
    pub attachments: Vec<AttachmentOutcome>,
}

/// Creates a transfer and attaches every staged document to it in one call.
/// Same partial-failure contract as `create_purchase_with_documents`: `Err`
/// only if the transfer itself fails to save (an invalid date, a
/// non-positive amount/rate, or an overflowing `amount * rate` — see
/// `db::queries::transfers::insert`); a saved transfer with some documents
/// failing to attach is still `Ok`, with the per-file outcomes in
/// `CreatedTransfer::attachments`.
pub fn create_transfer_with_documents(
    conn: &Connection,
    documents_dir: &Path,
    draft: &TransferDraft,
    pending: &[PendingDocument<'_>],
) -> Result<CreatedTransfer> {
    let id = create_transfer(conn, draft)?;
    let attachments = attach_documents(
        conn,
        documents_dir,
        &draft.date,
        None,
        ("transfer", id),
        pending,
    );
    Ok(CreatedTransfer { id, attachments })
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

    fn attach_document_test_files(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let tmp = std::env::temp_dir().join(format!(
            "adm-sfa-attach-document-test-{tag}-{}",
            std::process::id()
        ));
        let documents_dir = tmp.join("documents");
        std::fs::create_dir_all(&documents_dir).unwrap();
        let src = tmp.join("chat.png");
        std::fs::write(&src, b"fake").unwrap();
        (tmp, src)
    }

    #[test]
    fn attach_document_rejects_a_label_outside_document_label() {
        let conn = test_db();
        let id = create_purchase(&conn, &purchase_draft()).unwrap();
        let (tmp, src) = attach_document_test_files("unknown-label");

        let result = attach_document(
            &conn,
            &tmp.join("documents"),
            &src,
            "2026-01-01",
            None,
            ("purchase", id),
            "../../../etc/passwd",
            &[],
        );

        assert!(result.is_err());
        assert!(documents_qry::list_for_record(&conn, "purchase", id)
            .unwrap()
            .is_empty());

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn attach_document_accepts_a_known_label() {
        let conn = test_db();
        let id = create_purchase(&conn, &purchase_draft()).unwrap();
        let (tmp, src) = attach_document_test_files("known-label");

        let result = attach_document(
            &conn,
            &tmp.join("documents"),
            &src,
            "2026-01-01",
            None,
            ("purchase", id),
            "chat",
            &[],
        );

        assert!(result.is_ok());
        assert_eq!(
            documents_qry::list_for_record(&conn, "purchase", id)
                .unwrap()
                .len(),
            1
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    fn create_with_docs_test_dir(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let tmp = std::env::temp_dir().join(format!(
            "adm-sfa-create-purchase-with-documents-test-{tag}-{}",
            std::process::id()
        ));
        let documents_dir = tmp.join("documents");
        std::fs::create_dir_all(&documents_dir).unwrap();
        (tmp, documents_dir)
    }

    #[test]
    fn create_purchase_with_documents_attaches_every_staged_file() {
        let conn = test_db();
        let (tmp, documents_dir) = create_with_docs_test_dir("attaches-every-file");
        let chat_src = tmp.join("chat.png");
        std::fs::write(&chat_src, b"fake").unwrap();
        let receipt_src = tmp.join("receipt.pdf");
        std::fs::write(&receipt_src, b"fake").unwrap();
        let pending = vec![
            PendingDocument {
                path: &chat_src,
                label: "chat",
            },
            PendingDocument {
                path: &receipt_src,
                label: "receipt",
            },
        ];

        let created =
            create_purchase_with_documents(&conn, &documents_dir, &purchase_draft(), &pending)
                .unwrap();

        assert_eq!(created.attachments.len(), 2);
        assert!(created.attachments.iter().all(|a| a.result.is_ok()));
        assert_eq!(
            documents_qry::list_for_record(&conn, "purchase", created.id)
                .unwrap()
                .len(),
            2
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn create_purchase_with_documents_deduplicates_filenames_within_one_batch() {
        let conn = test_db();
        let (tmp, documents_dir) = create_with_docs_test_dir("dedup-within-batch");
        let src_a = tmp.join("a.png");
        std::fs::write(&src_a, b"fake").unwrap();
        let src_b = tmp.join("b.png");
        std::fs::write(&src_b, b"fake").unwrap();
        // Same label, same draft date — both would generate the identical
        // base filename if the batch didn't feed each success back into the
        // de-dupe list before the next attempt.
        let pending = vec![
            PendingDocument {
                path: &src_a,
                label: "chat",
            },
            PendingDocument {
                path: &src_b,
                label: "chat",
            },
        ];

        let created =
            create_purchase_with_documents(&conn, &documents_dir, &purchase_draft(), &pending)
                .unwrap();

        let filenames: Vec<String> = created
            .attachments
            .iter()
            .map(|a| a.result.clone().unwrap())
            .collect();
        assert_eq!(filenames.len(), 2);
        assert_ne!(filenames[0], filenames[1]);
        assert!(filenames[1].contains("-2"));

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn create_purchase_with_documents_records_a_per_file_failure_without_rolling_back() {
        let conn = test_db();
        let (tmp, documents_dir) = create_with_docs_test_dir("partial-failure");
        let good_src = tmp.join("good.png");
        std::fs::write(&good_src, b"fake").unwrap();
        let bad_src = tmp.join("bad.png");
        std::fs::write(&bad_src, b"fake").unwrap();
        let pending = vec![
            PendingDocument {
                path: &good_src,
                label: "chat",
            },
            PendingDocument {
                path: &bad_src,
                label: "not-a-real-label",
            },
        ];

        let created =
            create_purchase_with_documents(&conn, &documents_dir, &purchase_draft(), &pending)
                .unwrap();

        assert!(created.attachments[0].result.is_ok());
        assert!(created.attachments[1].result.is_err());
        assert_eq!(
            documents_qry::list_for_record(&conn, "purchase", created.id)
                .unwrap()
                .len(),
            1
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn create_purchase_with_documents_with_no_attachments_creates_just_the_purchase() {
        let conn = test_db();
        let (tmp, documents_dir) = create_with_docs_test_dir("no-attachments");

        let created =
            create_purchase_with_documents(&conn, &documents_dir, &purchase_draft(), &[]).unwrap();

        assert!(created.attachments.is_empty());
        assert_eq!(
            documents_qry::list_for_record(&conn, "purchase", created.id)
                .unwrap()
                .len(),
            0
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn create_purchase_with_documents_reports_a_missing_source_file_per_file() {
        let conn = test_db();
        let (tmp, documents_dir) = create_with_docs_test_dir("missing-source");
        let missing_src = tmp.join("does-not-exist.png");
        let pending = vec![PendingDocument {
            path: &missing_src,
            label: "chat",
        }];

        let created =
            create_purchase_with_documents(&conn, &documents_dir, &purchase_draft(), &pending)
                .unwrap();

        assert!(created.attachments[0].result.is_err());
        assert_eq!(
            documents_qry::list_for_record(&conn, "purchase", created.id)
                .unwrap()
                .len(),
            0
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn create_purchase_with_documents_files_nothing_when_the_purchase_cannot_be_inserted() {
        let conn = test_db();
        let (tmp, documents_dir) = create_with_docs_test_dir("purchase-insert-fails");
        let src = tmp.join("chat.png");
        std::fs::write(&src, b"fake").unwrap();
        let pending = vec![PendingDocument {
            path: &src,
            label: "chat",
        }];
        let mut bad_draft = purchase_draft();
        bad_draft.cost_str = "not a number".to_string();

        let result = create_purchase_with_documents(&conn, &documents_dir, &bad_draft, &pending);

        assert!(result.is_err());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM document", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);

        std::fs::remove_dir_all(&tmp).ok();
    }

    fn transfer_draft() -> TransferDraft {
        TransferDraft {
            date: "2026-01-01".to_string(),
            eur_amount_sent_str: "1000.00".to_string(),
            exchange_rate_str: "5.5".to_string(),
            notes: String::new(),
        }
    }

    #[test]
    fn create_transfer_with_documents_attaches_every_staged_file() {
        let conn = test_db();
        let (tmp, documents_dir) = create_with_docs_test_dir("transfer-attaches-every-file");
        let chat_src = tmp.join("chat.png");
        std::fs::write(&chat_src, b"fake").unwrap();
        let receipt_src = tmp.join("receipt.pdf");
        std::fs::write(&receipt_src, b"fake").unwrap();
        let pending = vec![
            PendingDocument {
                path: &chat_src,
                label: "chat",
            },
            PendingDocument {
                path: &receipt_src,
                label: "receipt",
            },
        ];

        let created =
            create_transfer_with_documents(&conn, &documents_dir, &transfer_draft(), &pending)
                .unwrap();

        assert_eq!(created.attachments.len(), 2);
        assert!(created.attachments.iter().all(|a| a.result.is_ok()));
        assert_eq!(
            documents_qry::list_for_record(&conn, "transfer", created.id)
                .unwrap()
                .len(),
            2
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn create_transfer_with_documents_deduplicates_filenames_within_one_batch() {
        let conn = test_db();
        let (tmp, documents_dir) = create_with_docs_test_dir("transfer-dedup-within-batch");
        let src_a = tmp.join("a.png");
        std::fs::write(&src_a, b"fake").unwrap();
        let src_b = tmp.join("b.png");
        std::fs::write(&src_b, b"fake").unwrap();
        let pending = vec![
            PendingDocument {
                path: &src_a,
                label: "chat",
            },
            PendingDocument {
                path: &src_b,
                label: "chat",
            },
        ];

        let created =
            create_transfer_with_documents(&conn, &documents_dir, &transfer_draft(), &pending)
                .unwrap();

        let filenames: Vec<String> = created
            .attachments
            .iter()
            .map(|a| a.result.clone().unwrap())
            .collect();
        assert_eq!(filenames.len(), 2);
        assert_ne!(filenames[0], filenames[1]);
        assert!(filenames[1].contains("-2"));

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn create_transfer_with_documents_records_a_per_file_failure_without_rolling_back() {
        let conn = test_db();
        let (tmp, documents_dir) = create_with_docs_test_dir("transfer-partial-failure");
        let good_src = tmp.join("good.png");
        std::fs::write(&good_src, b"fake").unwrap();
        let bad_src = tmp.join("bad.png");
        std::fs::write(&bad_src, b"fake").unwrap();
        let pending = vec![
            PendingDocument {
                path: &good_src,
                label: "chat",
            },
            PendingDocument {
                path: &bad_src,
                label: "not-a-real-label",
            },
        ];

        let created =
            create_transfer_with_documents(&conn, &documents_dir, &transfer_draft(), &pending)
                .unwrap();

        assert!(created.attachments[0].result.is_ok());
        assert!(created.attachments[1].result.is_err());
        assert_eq!(
            documents_qry::list_for_record(&conn, "transfer", created.id)
                .unwrap()
                .len(),
            1
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn create_transfer_with_documents_with_no_attachments_creates_just_the_transfer() {
        let conn = test_db();
        let (tmp, documents_dir) = create_with_docs_test_dir("transfer-no-attachments");

        let created =
            create_transfer_with_documents(&conn, &documents_dir, &transfer_draft(), &[]).unwrap();

        assert!(created.attachments.is_empty());
        assert_eq!(
            documents_qry::list_for_record(&conn, "transfer", created.id)
                .unwrap()
                .len(),
            0
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn create_transfer_with_documents_files_nothing_when_the_transfer_cannot_be_inserted() {
        let conn = test_db();
        let (tmp, documents_dir) = create_with_docs_test_dir("transfer-insert-fails");
        let src = tmp.join("chat.png");
        std::fs::write(&src, b"fake").unwrap();
        let pending = vec![PendingDocument {
            path: &src,
            label: "chat",
        }];
        let mut bad_draft = transfer_draft();
        bad_draft.eur_amount_sent_str = "not a number".to_string();

        let result = create_transfer_with_documents(&conn, &documents_dir, &bad_draft, &pending);

        assert!(result.is_err());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM document", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);

        std::fs::remove_dir_all(&tmp).ok();
    }
}
