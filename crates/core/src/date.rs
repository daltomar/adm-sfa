use chrono::NaiveDate;

/// Parses a user-typed date, accepting the app's canonical "D.M.YYYY" input
/// format (leading zeros optional, e.g. "1.7.2026" or "01.07.2026") and,
/// as a fallback, ISO "YYYY-MM-DD" (a strict superset — the different
/// separator makes the two formats visually unambiguous, and this keeps
/// any already-ISO data, including hardcoded test fixtures, parsing
/// unchanged).
///
/// The year must be exactly 4 digits in either format — two-digit years
/// aren't guessed at, same "don't guess ambiguous input" philosophy as
/// `money::parse_amount_input`. Slash-separated input ("DD/MM/YYYY") is
/// not accepted; that display style has been retired in favor of a single
/// dotted format everywhere.
pub fn parse_date_input(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    if let Some(date) = parse_dotted(s) {
        return Some(date);
    }
    parse_iso(s)
}

fn parse_dotted(s: &str) -> Option<NaiveDate> {
    let mut parts = s.splitn(4, '.');
    let (Some(d), Some(m), Some(y), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    if y.len() != 4 {
        return None;
    }
    let (d, m, y) = (d.parse().ok()?, m.parse().ok()?, y.parse().ok()?);
    NaiveDate::from_ymd_opt(y, m, d)
}

fn parse_iso(s: &str) -> Option<NaiveDate> {
    let mut parts = s.splitn(4, '-');
    let (Some(y), Some(m), Some(d), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    if y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return None;
    }
    let (y, m, d) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    NaiveDate::from_ymd_opt(y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn parses_dotted_zero_padded() {
        assert_eq!(parse_date_input("16.07.2026"), Some(ymd(2026, 7, 16)));
    }

    #[test]
    fn parses_dotted_single_digit_day_and_month() {
        assert_eq!(parse_date_input("1.7.2026"), Some(ymd(2026, 7, 1)));
    }

    #[test]
    fn accepts_iso_as_a_fallback() {
        assert_eq!(parse_date_input("2026-07-16"), Some(ymd(2026, 7, 16)));
    }

    #[test]
    fn rejects_two_digit_years() {
        assert_eq!(parse_date_input("16.07.26"), None);
        assert_eq!(parse_date_input("26-07-16"), None);
    }

    #[test]
    fn rejects_invalid_calendar_dates() {
        assert_eq!(parse_date_input("31.02.2026"), None);
        assert_eq!(parse_date_input("29.02.2025"), None); // not a leap year
    }

    #[test]
    fn accepts_leap_day() {
        assert_eq!(parse_date_input("29.02.2024"), Some(ymd(2024, 2, 29)));
    }

    #[test]
    fn rejects_slash_format() {
        assert_eq!(parse_date_input("16/07/2026"), None);
    }

    #[test]
    fn rejects_empty_and_garbage() {
        assert_eq!(parse_date_input(""), None);
        assert_eq!(parse_date_input("   "), None);
        assert_eq!(parse_date_input("not a date"), None);
    }

    #[test]
    fn trims_whitespace() {
        assert_eq!(parse_date_input("  16.07.2026  "), Some(ymd(2026, 7, 16)));
    }
}
