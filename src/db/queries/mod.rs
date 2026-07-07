pub mod brl_ledger;
pub mod categories;
pub mod documents;
pub mod donors;
pub mod eur_ledger;
pub mod inventory;
pub mod outbound;
pub mod purchases;
pub mod transfers;

/// Converts an empty-or-whitespace string to `None`; non-empty to `Some`.
/// Used by query modules when mapping draft string fields to nullable DB columns.
fn opt(s: &str) -> Option<&str> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}
