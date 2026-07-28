pub mod brl_ledger;
pub mod categories;
pub mod documents;
pub mod donors;
pub mod eur_ledger;
pub mod inventory;
pub mod outbound;
pub mod purchases;
pub mod settings;
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

/// Authoritative guard for name-like fields (category, document label, donor
/// name) that a UI form marks `required` but that a raw POST can still send
/// blank. `field` names the field in the error message (e.g. "category
/// name") so callers don't need their own wording.
fn require_name<'a>(field: &str, s: &'a str) -> rusqlite::Result<&'a str> {
    let t = s.trim();
    if t.is_empty() {
        return Err(rusqlite::Error::ToSqlConversionFailure(
            format!("{field} must not be empty").into(),
        ));
    }
    Ok(t)
}
