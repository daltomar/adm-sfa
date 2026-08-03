pub mod brl_ledger;
pub mod donors;
pub mod eur_ledger;
pub mod inventory;
pub mod login;
pub mod outbound;
pub mod purchases;
pub mod reports;
pub mod settings;
pub mod transfers;

/// A same-origin, root-relative path — rejects absolute URLs (`https://...`)
/// and protocol-relative ones (`//evil.example`) so a crafted `return_to`
/// can't be used as an open redirect. Shared by every `return_to`-carrying
/// "create X, come back to where I was" flow (`donors::create`,
/// `inventory::create_donation`, …) rather than duplicated per route module
/// — this is security-relevant logic, and CLAUDE.md's "business rules live
/// in one place" principle applies just as much to a redirect-safety check
/// as to a domain rule.
///
/// A bare `starts_with('/') && !starts_with("//")` isn't enough on its own:
/// WHATWG URL parsing (what a real browser applies to a `Location` header)
/// normalizes backslashes to forward slashes and strips ASCII tab/CR/LF
/// *before* resolving a relative reference, so e.g. `/\evil.example` or
/// `/\t/evil.example` both pass that check yet still resolve to an external
/// origin. Reject any backslash or control character outright rather than
/// trying to enumerate every such normalization individually.
pub(crate) fn safe_return_to(s: &str) -> bool {
    s.starts_with('/') && !s.starts_with("//") && !s.chars().any(|c| c == '\\' || c.is_control())
}
