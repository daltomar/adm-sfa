//! Locale-aware *display* formatting for dates and money amounts
//! (SPEC.md §6.5, stack-plan.md "Localisation (i18n)"). Never touches
//! storage or parsing — dates stay ISO `YYYY-MM-DD` in the DB and in date
//! *input* fields, and amount input parsing (`money::parse_amount_input`)
//! stays locale-independent per T7. Currency *symbol* is a property of the
//! ledger, not the locale (T-currency-symbol / SPEC.md §6.5) — this module
//! only ever formats the bare number; callers place the symbol themselves,
//! and the symbol's position relative to the number is a translation
//! concern (the `%{symbol}`/`%{amount}` ordering inside each locale's own
//! `t!()` template), not something this module decides.

use rust_decimal::Decimal;

/// The three supported UI/report languages (SPEC.md §6), as (code, endonym)
/// pairs — the name is shown in its own language ("Deutsch", not a
/// translation of "German"), the universal convention for language pickers,
/// so it's a fixed literal here rather than an i18n key.
pub const LOCALES: &[(&str, &str)] = &[
    ("en", "English"),
    ("de", "Deutsch"),
    ("pt-BR", "Português (Brasil)"),
];

/// Reformats a raw ISO `YYYY-MM-DD` date for on-screen/PDF display in the
/// currently active locale. Falls back to the ISO string unchanged if it
/// doesn't parse as `YYYY-MM-DD` (defensive only — every date passed in
/// should already be one, per the DB schema).
pub fn date(iso: &str) -> String {
    date_for_locale(iso, rust_i18n::locale().as_ref())
}

/// Same as `date()`, but for an explicitly chosen locale rather than the
/// ambient UI locale — for report export (SPEC.md §6.3), where the target
/// language is a parameter the operator picks in the export dialog, never
/// implicitly read from the UI.
pub fn date_in(iso: &str, locale: &str) -> String {
    date_for_locale(iso, locale)
}

fn date_for_locale(iso: &str, locale: &str) -> String {
    let mut parts = iso.splitn(3, '-');
    let (Some(y), Some(m), Some(d)) = (parts.next(), parts.next(), parts.next()) else {
        return iso.to_string();
    };
    if d.len() != 2 || m.len() != 2 || y.len() != 4 {
        return iso.to_string();
    }
    match locale {
        "de" => format!("{d}.{m}.{y}"),
        "pt-BR" => format!("{d}/{m}/{y}"),
        _ => iso.to_string(),
    }
}

/// Formats a money amount to 2 decimal places with the currently active
/// locale's decimal/thousands separators. Never includes a currency symbol.
pub fn amount(value: Decimal) -> String {
    number(value, 2)
}

/// Same as `amount()`, but for an explicitly chosen locale rather than the
/// ambient UI locale — for report export (SPEC.md §6.3). Not used by CSV
/// export, which always uses `csv_amount()` regardless of the chosen
/// report language (SPEC.md §6.4/T6).
pub fn amount_in(value: Decimal, locale: &str) -> String {
    number_for_locale(value, 2, locale)
}

/// Formats a plain decimal number (e.g. an exchange rate, not a money
/// amount) to `decimals` places with the currently active locale's
/// decimal/thousands separators.
pub fn number(value: Decimal, decimals: usize) -> String {
    number_for_locale(value, decimals, rust_i18n::locale().as_ref())
}

/// Formats a money amount to 2 decimal places in the fixed German/Brazilian
/// style (`,` decimal, `.` thousands) used by CSV export (SPEC.md §6.4/T6).
/// Deliberately ignores the active UI locale — CSV output never varies with
/// it, unlike `amount()`/`number()` above.
pub fn csv_amount(value: Decimal) -> String {
    number_for_locale(value, 2, "de")
}

fn number_for_locale(value: Decimal, decimals: usize, locale: &str) -> String {
    let raw = format!("{value:.decimals$}");
    let (sign, digits) = raw
        .strip_prefix('-')
        .map_or(("", raw.as_str()), |d| ("-", d));
    let (int_part, frac_part) = digits.split_once('.').unwrap_or((digits, ""));

    let (decimal_sep, thousands_sep) = match locale {
        "de" | "pt-BR" => (',', '.'),
        _ => ('.', ','),
    };

    let grouped = int_part
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|chunk| std::str::from_utf8(chunk).expect("ASCII digits"))
        .collect::<Vec<_>>()
        .join(&thousands_sep.to_string());

    if frac_part.is_empty() {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped}{decimal_sep}{frac_part}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn date_stays_iso_for_english() {
        assert_eq!(date_for_locale("2026-07-16", "en"), "2026-07-16");
    }

    #[test]
    fn date_is_dotted_dmy_for_german() {
        assert_eq!(date_for_locale("2026-07-16", "de"), "16.07.2026");
    }

    #[test]
    fn date_is_slashed_dmy_for_portuguese() {
        assert_eq!(date_for_locale("2026-07-16", "pt-BR"), "16/07/2026");
    }

    #[test]
    fn date_falls_back_unchanged_when_not_iso_shaped() {
        assert_eq!(date_for_locale("not-a-date", "de"), "not-a-date");
    }

    #[test]
    fn amount_uses_comma_decimal_for_english() {
        assert_eq!(number_for_locale(dec!(1234.5), 2, "en"), "1,234.50");
    }

    #[test]
    fn amount_uses_period_thousands_for_german() {
        assert_eq!(number_for_locale(dec!(1234.56), 2, "de"), "1.234,56");
    }

    #[test]
    fn amount_matches_german_for_portuguese() {
        assert_eq!(number_for_locale(dec!(1234.56), 2, "pt-BR"), "1.234,56");
    }

    #[test]
    fn amount_handles_negative_sign() {
        assert_eq!(number_for_locale(dec!(-1234.56), 2, "de"), "-1.234,56");
    }

    #[test]
    fn amount_handles_small_values_with_no_grouping() {
        assert_eq!(number_for_locale(dec!(5), 2, "en"), "5.00");
    }

    #[test]
    fn amount_groups_large_values_correctly() {
        assert_eq!(
            number_for_locale(dec!(12345678.9), 2, "en"),
            "12,345,678.90"
        );
    }

    #[test]
    fn csv_amount_is_always_german_style_regardless_of_active_locale() {
        assert_eq!(csv_amount(dec!(1234.5)), "1.234,50");
    }

    #[test]
    fn amount_in_and_date_in_use_the_given_locale_not_the_ambient_one() {
        assert_eq!(amount_in(dec!(1234.5), "de"), "1.234,50");
        assert_eq!(date_in("2026-07-16", "pt-BR"), "16/07/2026");
    }

    #[test]
    fn number_supports_non_money_precision_like_exchange_rates() {
        assert_eq!(number_for_locale(dec!(5.4321), 4, "en"), "5.4321");
        assert_eq!(number_for_locale(dec!(5.4321), 4, "de"), "5,4321");
    }

    #[test]
    fn number_with_zero_decimals_has_no_separator() {
        assert_eq!(number_for_locale(dec!(1234), 0, "de"), "1.234");
    }
}
