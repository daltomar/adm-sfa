use rust_decimal::Decimal;
use std::str::FromStr;

/// Parses a user-typed decimal amount, tolerating a comma as the decimal
/// separator (e.g. "50,00") and a leading/trailing currency symbol
/// (R$, €, $) — natural input in this app's EUR/BRL, German/Brazilian
/// context, which `Decimal::from_str` alone rejects.
///
/// Mixed-format input (e.g. "1.234,56") is not handled — comma is only
/// treated as the decimal separator when the string contains no period.
pub fn parse_amount_input(s: &str) -> Option<Decimal> {
    let s = s.trim();
    let s = s
        .strip_prefix("R$")
        .or_else(|| s.strip_prefix('€'))
        .or_else(|| s.strip_prefix('$'))
        .unwrap_or(s)
        .trim();
    let s = s
        .strip_suffix('€')
        .or_else(|| s.strip_suffix('$'))
        .unwrap_or(s)
        .trim();

    if s.is_empty() {
        return None;
    }

    if s.contains(',') && !s.contains('.') {
        Decimal::from_str(&s.replace(',', ".")).ok()
    } else {
        Decimal::from_str(s).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_period_decimal() {
        assert_eq!(parse_amount_input("50.00"), Some(Decimal::new(5000, 2)));
    }

    #[test]
    fn parses_comma_decimal() {
        assert_eq!(parse_amount_input("50,00"), Some(Decimal::new(5000, 2)));
    }

    #[test]
    fn strips_leading_currency_symbol() {
        assert_eq!(parse_amount_input("R$ 50,00"), Some(Decimal::new(5000, 2)));
        assert_eq!(parse_amount_input("€ 50.00"), Some(Decimal::new(5000, 2)));
        assert_eq!(parse_amount_input("$50.00"), Some(Decimal::new(5000, 2)));
    }

    #[test]
    fn negative_amounts_parse_positivity_is_callers_job() {
        assert_eq!(parse_amount_input("-50,00"), Some(Decimal::new(-5000, 2)));
    }

    #[test]
    fn mixed_separators_are_rejected_not_misparsed() {
        // Ambiguous four-figure formats aren't guessed at — comma is only
        // treated as a decimal separator when there's no period present.
        assert_eq!(parse_amount_input("1.234,56"), None);
        assert_eq!(parse_amount_input("1,234.56"), None);
    }

    #[test]
    fn malformed_comma_input_is_rejected() {
        assert_eq!(parse_amount_input("50,,00"), None);
    }

    #[test]
    fn rejects_empty_and_garbage() {
        assert_eq!(parse_amount_input(""), None);
        assert_eq!(parse_amount_input("   "), None);
        assert_eq!(parse_amount_input("abc"), None);
    }
}
