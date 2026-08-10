use axum_extra::extract::cookie::{Cookie, SameSite};
use axum_extra::extract::SignedCookieJar;

use crate::templates;

const FLASH_COOKIE: &str = "adm_sfa_flash";

/// Only needs to survive a single redirect hop (the PRG pattern), unlike
/// `auth.rs`'s 8-hour session cookie — `take_flash` also removes it on
/// read, so this `Max-Age` is belt-and-suspenders against an unread flash
/// (e.g. a save whose redirect target is never actually loaded) rather
/// than load-bearing.
const FLASH_MAX_AGE: time::Duration = time::Duration::seconds(30);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlashKind {
    Success,
    Error,
}

impl FlashKind {
    fn as_cookie_value(self) -> &'static str {
        match self {
            FlashKind::Success => "success",
            FlashKind::Error => "error",
        }
    }

    fn from_cookie_value(value: &str) -> Option<Self> {
        match value {
            "success" => Some(FlashKind::Success),
            "error" => Some(FlashKind::Error),
            _ => None,
        }
    }
}

/// Sets the flash cookie right before a save handler's redirect. Mirrors
/// `auth::set_session_cookie`'s security attributes (signed, `HttpOnly`,
/// `SameSite=Strict`, path `/`) but with a much shorter `Max-Age`.
pub fn set_flash(jar: SignedCookieJar, kind: FlashKind) -> SignedCookieJar {
    let cookie = Cookie::build((FLASH_COOKIE, kind.as_cookie_value()))
        .http_only(true)
        .same_site(SameSite::Strict)
        .path("/")
        .max_age(FLASH_MAX_AGE)
        .build();
    jar.add(cookie)
}

/// Reads and immediately clears the flash cookie — read-once semantics, so
/// a manual page refresh doesn't re-show a stale banner (this is what makes
/// "persistent until navigation, no auto-dismiss timer" correct: the data
/// disappears after one read, not after a clock).
///
/// The removal cookie must repeat `set_flash`'s explicit `path("/")` — a
/// `Cookie:` request header (which is all `SignedCookieJar::from_headers`
/// has to go on) carries no path/domain attributes at all, so a bare
/// `Cookie::from(FLASH_COOKIE)` removal defaults to the *request URI's*
/// directory (RFC 6265) rather than `/`. Confirmed live: consuming the
/// flash on a nested route like `/purchases/1/edit` produced a deletion
/// `Set-Cookie` scoped to `/purchases/1`, which a real browser (and curl)
/// correctly leaves the original `Path=/` cookie alone for — so it survived
/// and re-appeared on the next unrelated `/purchases` load a moment later,
/// silently defeating the read-once guarantee this function exists for.
pub fn take_flash(jar: SignedCookieJar) -> (SignedCookieJar, Option<FlashKind>) {
    let kind = jar
        .get(FLASH_COOKIE)
        .and_then(|c| FlashKind::from_cookie_value(c.value()));
    let jar = jar.remove(Cookie::build(FLASH_COOKIE).path("/"));
    (jar, kind)
}

/// Resolves a `FlashKind` into the translated, render-ready `Flash` a
/// template can display — static banner keys go through the same `t!()`
/// convention as every other explicit-locale string in this crate.
pub fn flash_for_template(kind: Option<FlashKind>, locale: &str) -> Option<templates::Flash> {
    kind.map(|kind| match kind {
        FlashKind::Success => templates::Flash {
            css_class: "flash-success",
            message: rust_i18n::t!("common.status.save_success", locale = locale).into_owned(),
        },
        FlashKind::Error => templates::Flash {
            css_class: "flash-error",
            message: rust_i18n::t!("common.status.save_failed", locale = locale).into_owned(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum_extra::extract::cookie::Key;

    #[test]
    fn set_flash_has_the_expected_security_attributes() {
        let jar = SignedCookieJar::new(Key::generate());
        let jar = set_flash(jar, FlashKind::Success);
        let cookie = jar
            .get(FLASH_COOKIE)
            .expect("set_flash should have added the flash cookie");

        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(cookie.same_site(), Some(SameSite::Strict));
        assert_eq!(cookie.path(), Some("/"));
        assert_eq!(cookie.max_age(), Some(FLASH_MAX_AGE));
        assert_eq!(cookie.value(), "success");
    }

    /// Regression test for a real bug caught only by manual, live-server
    /// testing (not by the in-memory-jar tests above): a removal cookie
    /// missing an explicit `Path=/` doesn't actually delete the original
    /// cookie in a real browser or `curl`, since a `Set-Cookie` with no
    /// path defaults to the *request URI's* directory (RFC 6265), not `/`.
    /// Confirmed live — consuming the flash on `/purchases/1/edit` used to
    /// emit a deletion scoped to `/purchases/1`, which a real cookie store
    /// correctly leaves the original `Path=/` cookie alone for, so it
    /// reappeared on the very next `/purchases` list load. The in-memory
    /// `jar.get()` assertions above don't model this — a real client's
    /// path-matching identity — so this asserts against the actual rendered
    /// `Set-Cookie` header text instead, the only way to see what a browser
    /// would.
    #[test]
    fn take_flash_clears_the_cookie_with_a_path_matching_set_flash() {
        use axum::response::IntoResponse;

        // Reproduces the real two-request lifecycle, not a single in-memory
        // jar object — this distinction is exactly what the first version of
        // this test got wrong (it passed against the bug). The underlying
        // `cookie` crate's `CookieJar::remove` only emits a real removal
        // `Set-Cookie` when the cookie being removed is registered as an
        // *original* cookie (one seeded from an incoming `Cookie:` request
        // header via `from_headers`/`add_original`) — if it was only ever
        // `.add()`-ed within the same jar object, `remove` just cancels the
        // pending add with no header at all. So: render `set_flash`'s
        // response for real, extract its `Set-Cookie` value the way a
        // browser would store it, and feed *that* back in as a fresh
        // request's `Cookie:` header before calling `take_flash` — only
        // then does `remove` take the `original_cookies` branch and
        // actually exercise the `Path` this test is checking.
        let key = Key::generate();
        let jar = SignedCookieJar::new(key.clone());
        let jar = set_flash(jar, FlashKind::Success);
        let set_cookie_response = (jar, axum::http::StatusCode::OK).into_response();
        let set_cookie = set_cookie_response
            .headers()
            .get("set-cookie")
            .expect("set_flash should have produced a Set-Cookie header")
            .to_str()
            .unwrap();
        let cookie_header = set_cookie
            .split(';')
            .next()
            .expect("Set-Cookie header was empty")
            .to_string();

        let mut headers = axum::http::HeaderMap::new();
        headers.insert("cookie", cookie_header.parse().unwrap());
        let incoming_jar = SignedCookieJar::from_headers(&headers, key);
        let (jar, kind) = take_flash(incoming_jar);
        assert_eq!(kind, Some(FlashKind::Success));

        let removal_response = (jar, axum::http::StatusCode::OK).into_response();
        let removal = removal_response
            .headers()
            .get_all("set-cookie")
            .iter()
            .map(|v| v.to_str().unwrap())
            .find(|h| h.starts_with(FLASH_COOKIE))
            .expect("take_flash should emit a removal Set-Cookie for the flash cookie")
            .to_string();
        assert!(
            removal.contains("Path=/"),
            "removal cookie must repeat the original's Path=/ or a real browser won't delete it: {removal}"
        );
    }

    #[test]
    fn take_flash_round_trips_success_and_clears_the_cookie() {
        let jar = SignedCookieJar::new(Key::generate());
        let jar = set_flash(jar, FlashKind::Success);

        let (jar, kind) = take_flash(jar);

        assert_eq!(kind, Some(FlashKind::Success));
        assert!(jar.get(FLASH_COOKIE).is_none());
    }

    #[test]
    fn take_flash_round_trips_error() {
        let jar = SignedCookieJar::new(Key::generate());
        let jar = set_flash(jar, FlashKind::Error);

        let (_, kind) = take_flash(jar);

        assert_eq!(kind, Some(FlashKind::Error));
    }

    #[test]
    fn take_flash_returns_none_when_no_cookie_is_present() {
        let jar = SignedCookieJar::new(Key::generate());

        let (_, kind) = take_flash(jar);

        assert_eq!(kind, None);
    }

    #[test]
    fn take_flash_returns_none_for_a_tampered_cookie() {
        // A garbage `Cookie:` header can never be found by `SignedCookieJar`
        // (the signature check fails), so this exercises the same
        // "unsigned/tampered cookie is silently absent" behavior
        // `auth.rs::require_auth_redirects_when_the_cookie_is_unsigned_garbage`
        // relies on.
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "cookie",
            "adm_sfa_flash=not-a-valid-signature".parse().unwrap(),
        );
        let jar = SignedCookieJar::from_headers(&headers, Key::generate());

        let (_, kind) = take_flash(jar);

        assert_eq!(kind, None);
    }

    #[test]
    fn flash_for_template_resolves_success_and_error_text() {
        let success = flash_for_template(Some(FlashKind::Success), "en").unwrap();
        assert_eq!(success.css_class, "flash-success");
        assert_eq!(success.message, "Saved successfully.");

        let error = flash_for_template(Some(FlashKind::Error), "en").unwrap();
        assert_eq!(error.css_class, "flash-error");
        assert_eq!(error.message, "Save failed. Please try again.");

        assert!(flash_for_template(None, "en").is_none());
    }
}
