use axum::extract::{Path, Query, RawForm, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Form;
use axum::Router;
use rust_decimal::Decimal;
use serde::Deserialize;

use adm_sfa_core::db::queries::{inventory as inventory_qry, outbound as qry};
use adm_sfa_core::format;
use adm_sfa_core::model::inventory::{InventoryItemRow, ItemStatus};
use adm_sfa_core::model::outbound::{OutboundEventDraft, RecipientProject, RecipientProjectDraft};
use adm_sfa_core::service;

use crate::routes::safe_return_to;
use crate::state::AppState;
use crate::templates::{
    HtmlTemplate, ItemOption, OutboundFormTemplate, OutboundListTemplate, OutboundRow,
    RecipientOption, RecipientRow, RecipientsTemplate,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/outbound", get(list).post(create))
        .route("/outbound/new", get(new_form))
        .route("/outbound/{id}/edit", get(edit_form))
        .route("/outbound/{id}", axum::routing::post(update))
        .route(
            "/outbound/recipients",
            get(recipients).post(create_recipient),
        )
}

fn recipient_options(
    recipients: &[RecipientProject],
    selected: Option<i64>,
) -> Vec<RecipientOption> {
    recipients
        .iter()
        // Same rule as desktop's show_recipient_project_picker: an inactive
        // recipient is hidden from new selections but still shown (and thus
        // still selectable/keepable) if it's the event's current recipient.
        .filter(|p| p.active || selected == Some(p.id))
        .map(|p| RecipientOption {
            id: p.id,
            name: p.name.clone(),
            selected: selected == Some(p.id),
        })
        .collect()
}

fn item_options(items: &[InventoryItemRow], selected_ids: &[i64]) -> Vec<ItemOption> {
    items
        .iter()
        .filter(|item| item.status == ItemStatus::Available || selected_ids.contains(&item.id))
        .map(|item| ItemOption {
            id: item.id,
            label: format!(
                "{} \u{2014} {} \u{2014} {}",
                item.name, item.category_name, item.source_desc
            ),
            selected: selected_ids.contains(&item.id),
        })
        .collect()
}

fn form_template(
    conn: &rusqlite::Connection,
    id: Option<i64>,
    draft: &OutboundEventDraft,
    selected_item_ids: &[i64],
    error: Option<String>,
    locale: String,
) -> OutboundFormTemplate {
    let recipients = qry::list_recipient_projects(conn).unwrap_or_default();
    let items = inventory_qry::list(conn).unwrap_or_default();
    OutboundFormTemplate {
        id,
        date: draft.date.clone(),
        recipients: recipient_options(&recipients, draft.recipient_project_id),
        cash_amount_brl_str: draft.cash_amount_brl_str.clone(),
        notes: draft.notes.clone(),
        items: item_options(&items, selected_item_ids),
        error,
        locale,
    }
}

fn event_summary(item_count: i64, cash: Option<Decimal>, locale: &str) -> String {
    let mut s = if item_count == 1 {
        rust_i18n::t!("web.outbound.summary_one", locale = locale).to_string()
    } else {
        rust_i18n::t!(
            "web.outbound.summary_other",
            locale = locale,
            count = item_count
        )
        .to_string()
    };
    if let Some(cash) = cash {
        if cash > Decimal::ZERO {
            let amount = format::amount(cash);
            s.push_str(
                rust_i18n::t!(
                    "web.outbound.summary_cash_suffix",
                    locale = locale,
                    cash = &amount
                )
                .as_ref(),
            );
        }
    }
    s
}

async fn list(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let events = qry::list(&conn).unwrap_or_default();
    // `list` returns newest-first (shared with desktop, which relies on
    // that same order — reversed here, web-presentation-only, matching the
    // precedent set for the EUR Ledger list page).
    let rows = events
        .into_iter()
        .rev()
        .map(|e| OutboundRow {
            id: e.id,
            date_display: format::date(&e.date),
            recipient_name: e.recipient_name,
            summary: event_summary(e.item_count, e.cash_amount_brl, &locale),
        })
        .collect();
    HtmlTemplate(OutboundListTemplate {
        events: rows,
        locale,
    })
}

/// Query params the "+ New recipient" round trip comes back with —
/// `form.html`'s JS populates these onto the link's `return_to` on the way
/// out, and `create_recipient` appends `recipient_project_id` on the way
/// back in. Mirrors `eur_ledger.rs`'s `NewEntryQuery`/`donor_id` handling.
/// All fields optional so a plain `GET /outbound/new` behaves exactly as
/// before.
#[derive(Deserialize)]
struct NewEventQuery {
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    cash_amount_brl_str: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    recipient_project_id: Option<i64>,
}

async fn new_form(
    State(state): State<AppState>,
    Query(query): Query<NewEventQuery>,
) -> impl IntoResponse {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    // Validate the id actually resolves before trusting it — mirrors
    // `eur_ledger.rs::new_form`'s `donor_id` check: a stale link or a
    // hand-edited query string with a nonexistent `recipient_project_id`
    // would otherwise silently fail to preselect anything.
    let recipients = qry::list_recipient_projects(&conn).unwrap_or_default();
    let recipient_project_id = query
        .recipient_project_id
        .filter(|id| recipients.iter().any(|r| r.id == *id));
    let draft = OutboundEventDraft {
        date: query
            .date
            .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string()),
        recipient_project_id,
        cash_amount_brl_str: query.cash_amount_brl_str.unwrap_or_default(),
        notes: query.notes.unwrap_or_default(),
    };
    HtmlTemplate(form_template(&conn, None, &draft, &[], None, locale))
}

async fn edit_form(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let Some(event) = qry::get(&conn, id).ok().flatten() else {
        return (axum::http::StatusCode::NOT_FOUND, "event not found").into_response();
    };
    let selected_item_ids = qry::item_ids_for_event(&conn, id).unwrap_or_default();
    let draft = OutboundEventDraft {
        date: event.date,
        recipient_project_id: Some(event.recipient_project_id),
        cash_amount_brl_str: event
            .cash_amount_brl
            .map(|d| d.to_string())
            .unwrap_or_default(),
        notes: event.notes.unwrap_or_default(),
    };
    HtmlTemplate(form_template(
        &conn,
        Some(id),
        &draft,
        &selected_item_ids,
        None,
        locale,
    ))
    .into_response()
}

/// Outbound's form needs a repeated `item_ids` field (one checkbox per
/// eligible item, all sharing the same `name`) — `axum::Form`'s
/// `serde_urlencoded` backend rejects that outright (`Vec<T>` from
/// duplicate keys isn't something it supports, confirmed empirically: it
/// errors "invalid type: string, expected a sequence" rather than
/// collecting). `RawForm` + `form_urlencoded::parse` reads the raw
/// key-value pairs directly instead, so a manual pass here replaces
/// `#[derive(Deserialize)] struct ...Form` used by every other section's
/// route in this crate.
struct OutboundForm {
    date: String,
    recipient_project_id: Option<i64>,
    cash_amount_brl_str: String,
    notes: String,
    item_ids: Vec<i64>,
}

fn parse_outbound_form(bytes: &[u8]) -> OutboundForm {
    let mut date = String::new();
    let mut recipient_project_id = None;
    let mut cash_amount_brl_str = String::new();
    let mut notes = String::new();
    let mut item_ids = Vec::new();

    for (key, value) in form_urlencoded::parse(bytes) {
        match key.as_ref() {
            "date" => date = value.into_owned(),
            "recipient_project_id" => {
                recipient_project_id = value.trim().parse().ok();
            }
            "cash_amount_brl_str" => cash_amount_brl_str = value.into_owned(),
            "notes" => notes = value.into_owned(),
            "item_ids" => {
                if let Ok(id) = value.trim().parse() {
                    item_ids.push(id);
                }
            }
            _ => {}
        }
    }

    OutboundForm {
        date,
        recipient_project_id,
        cash_amount_brl_str,
        notes,
        item_ids,
    }
}

async fn create(State(state): State<AppState>, RawForm(bytes): RawForm) -> Response {
    let form = parse_outbound_form(&bytes);
    let draft = OutboundEventDraft {
        date: form.date,
        recipient_project_id: form.recipient_project_id,
        cash_amount_brl_str: form.cash_amount_brl_str,
        notes: form.notes,
    };
    let conn = state.conn();
    match service::donate_items(&conn, &draft, &form.item_ids) {
        // Lands on the Outbound list, not this event's own edit page — matches
        // the same fix already applied to donors.rs's create() and
        // inventory.rs's create_donation(): the normal "section page → + New
        // X" flow should return to the list, not detour through an edit view
        // the user didn't ask to open.
        Ok(_id) => Redirect::to("/outbound").into_response(),
        Err(e) => {
            let locale = crate::i18n::resolve_locale(&conn);
            HtmlTemplate(form_template(
                &conn,
                None,
                &draft,
                &form.item_ids,
                Some(e.to_string()),
                locale,
            ))
            .into_response()
        }
    }
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RawForm(bytes): RawForm,
) -> Response {
    let form = parse_outbound_form(&bytes);
    let draft = OutboundEventDraft {
        date: form.date,
        recipient_project_id: form.recipient_project_id,
        cash_amount_brl_str: form.cash_amount_brl_str,
        notes: form.notes,
    };
    let conn = state.conn();
    match qry::update(&conn, id, &draft, &form.item_ids) {
        Ok(()) => Redirect::to(&format!("/outbound/{id}/edit")).into_response(),
        Err(e) => {
            let locale = crate::i18n::resolve_locale(&conn);
            HtmlTemplate(form_template(
                &conn,
                Some(id),
                &draft,
                &form.item_ids,
                Some(e.to_string()),
                locale,
            ))
            .into_response()
        }
    }
}

#[derive(Deserialize)]
struct RecipientsQuery {
    #[serde(default)]
    return_to: Option<String>,
}

/// Recipient projects, like physical donations (see `inventory.rs`'s
/// `/inventory/donations`), are only ever created inline from this
/// section's own sub-form in desktop — no standalone CRUD page. Same
/// no-JS constraint, same standalone-detour solution: create here, then
/// get carried straight back to `/outbound/new` via the `return_to` round
/// trip (mirroring `donors.rs` / `inventory.rs`'s "+ New donor"/"+ New
/// donation" flows) with the new recipient preselected in the dropdown —
/// `create_recipient` appends `recipient_project_id` on the way back, and
/// `new_form` reads it back to preselect. Unlike donations, recipient
/// projects *can* be edited in principle (an `active` flag exists), but
/// `core` has no `update` for this table either — matching what's
/// actually implemented rather than adding one speculatively.
async fn recipients(
    State(state): State<AppState>,
    Query(query): Query<RecipientsQuery>,
) -> impl IntoResponse {
    let conn = state.conn();
    let locale = crate::i18n::resolve_locale(&conn);
    let recipients = qry::list_recipient_projects(&conn).unwrap_or_default();
    let rows = recipients
        .into_iter()
        .map(|p| RecipientRow {
            name: p.name,
            contact_info: p.contact_info.unwrap_or_default(),
            location: p.location.unwrap_or_default(),
            active: p.active,
        })
        .collect();
    HtmlTemplate(RecipientsTemplate {
        recipients: rows,
        error: None,
        return_to: query.return_to.filter(|s| safe_return_to(s)),
        locale,
    })
}

#[derive(Deserialize)]
struct RecipientForm {
    name: String,
    #[serde(default)]
    contact_info: String,
    #[serde(default)]
    location: String,
    #[serde(default)]
    return_to: String,
}

async fn create_recipient(
    State(state): State<AppState>,
    Form(form): Form<RecipientForm>,
) -> Response {
    let return_to = form.return_to;
    let draft = RecipientProjectDraft {
        name: form.name,
        contact_info: form.contact_info,
        location: form.location,
        active: true,
    };
    let conn = state.conn();
    match qry::insert_recipient_project(&conn, &draft) {
        Ok(id) => {
            if safe_return_to(&return_to) {
                // Assumes return_to carries no #fragment (none of today's
                // callers emit one) — appending a query after a fragment
                // would produce a syntactically-wrong-order URL. Mirrors
                // `donors.rs::create` / `inventory.rs::create_donation`.
                let sep = if return_to.contains('?') { '&' } else { '?' };
                Redirect::to(&format!("{return_to}{sep}recipient_project_id={id}"))
                    .into_response()
            } else {
                // No caller-supplied return path (a direct visit to this
                // page's own nav entry) or an unsafe one (rejected above,
                // falls back here too) — either way lands on the recipients
                // list itself, matching the pre-existing behavior for every
                // visit that isn't a "+ New recipient" round trip.
                Redirect::to("/outbound/recipients").into_response()
            }
        }
        Err(e) => {
            let locale = crate::i18n::resolve_locale(&conn);
            let recipients = qry::list_recipient_projects(&conn).unwrap_or_default();
            let rows = recipients
                .into_iter()
                .map(|p| RecipientRow {
                    name: p.name,
                    contact_info: p.contact_info.unwrap_or_default(),
                    location: p.location.unwrap_or_default(),
                    active: p.active,
                })
                .collect();
            HtmlTemplate(RecipientsTemplate {
                recipients: rows,
                error: Some(e.to_string()),
                return_to: Some(return_to).filter(|s| safe_return_to(s)),
                locale,
            })
            .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support;
    use adm_sfa_core::model::outbound::{OutboundEventDraft, RecipientProjectDraft};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};

    /// Regression coverage for CLAUDE.md's "Sorting" backlog item, mirroring
    /// `eur_ledger.rs`'s `list_shows_the_oldest_entry_first`.
    #[tokio::test]
    async fn list_shows_the_oldest_event_first() {
        let (state, dir) = test_support::test_app("outbound-list-oldest-first");
        let conn = state.conn();
        let rp_id = super::qry::insert_recipient_project(&conn, &recipient_draft("OlderRecipient"))
            .unwrap();
        let rp2_id =
            super::qry::insert_recipient_project(&conn, &recipient_draft("NewerRecipient"))
                .unwrap();
        service_donate(&conn, rp_id, "2026-01-01");
        service_donate(&conn, rp2_id, "2026-06-01");
        drop(conn);

        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("GET")
            .uri("/outbound")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        let older_pos = body
            .find("OlderRecipient")
            .expect("older recipient not found");
        let newer_pos = body
            .find("NewerRecipient")
            .expect("newer recipient not found");
        assert!(
            older_pos < newer_pos,
            "expected the older event to render before the newer one"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    fn service_donate(conn: &rusqlite::Connection, recipient_project_id: i64, date: &str) {
        let draft = OutboundEventDraft {
            date: date.to_string(),
            recipient_project_id: Some(recipient_project_id),
            cash_amount_brl_str: "50.00".to_string(),
            notes: String::new(),
        };
        adm_sfa_core::service::donate_items(conn, &draft, &[]).unwrap();
    }

    fn recipient_draft(name: &str) -> RecipientProjectDraft {
        RecipientProjectDraft {
            name: name.to_string(),
            contact_info: String::new(),
            location: String::new(),
            active: true,
        }
    }

    /// Regression coverage for the bug report: creating an outbound event
    /// used to redirect to this event's own `/outbound/{id}/edit` instead of
    /// back to the Outbound list — the same fix already applied to
    /// `donors.rs::create` and `inventory.rs::create_donation`.
    #[tokio::test]
    async fn create_redirects_to_the_outbound_list_not_the_edit_page() {
        let (state, dir) = test_support::test_app("outbound-create-redirect");
        let conn = state.conn();
        let rp_id =
            super::qry::insert_recipient_project(&conn, &recipient_draft("Recipient")).unwrap();
        drop(conn);

        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body = format!(
            "date=2026-01-01&recipient_project_id={rp_id}&cash_amount_brl_str=50.00&notes="
        );
        let req = Request::builder()
            .method("POST")
            .uri("/outbound")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let location = res.headers().get("location").unwrap().to_str().unwrap();
        assert_eq!(location, "/outbound");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The "+ New recipient" flow: creating a recipient with a `return_to`
    /// carried over from `/outbound/new` should redirect there (with the new
    /// recipient's id appended) instead of landing on the recipients list —
    /// mirrors `donors.rs`'s
    /// `create_with_a_return_to_redirects_there_with_donor_id_appended`.
    #[tokio::test]
    async fn create_recipient_with_a_return_to_redirects_there_with_recipient_id_appended() {
        let (state, dir) = test_support::test_app("outbound-recipient-return-to");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body = "name=NewOrg&contact_info=&location=\
                     &return_to=%2Foutbound%2Fnew%3Fdate%3D2026-01-01";
        let req = Request::builder()
            .method("POST")
            .uri("/outbound/recipients")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let location = res.headers().get("location").unwrap().to_str().unwrap();
        assert_eq!(location, "/outbound/new?date=2026-01-01&recipient_project_id=1");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// No caller-supplied `return_to` (a direct visit to `/outbound/recipients`
    /// from its own nav entry) still lands on the recipients list, unchanged
    /// from before this fix.
    #[tokio::test]
    async fn create_recipient_without_a_return_to_redirects_to_the_recipients_list() {
        let (state, dir) = test_support::test_app("outbound-recipient-no-return-to");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body = "name=NewOrg&contact_info=&location=";
        let req = Request::builder()
            .method("POST")
            .uri("/outbound/recipients")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let location = res.headers().get("location").unwrap().to_str().unwrap();
        assert_eq!(location, "/outbound/recipients");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A `return_to` that isn't a root-relative path (an open-redirect
    /// attempt) is rejected — falls back to the recipients list instead of
    /// ever being handed to `Redirect::to`. Same guard as `donors.rs`'s
    /// `create_ignores_an_unsafe_return_to`.
    #[tokio::test]
    async fn create_recipient_ignores_an_unsafe_return_to() {
        let (state, dir) = test_support::test_app("outbound-recipient-unsafe-return-to");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let body = "name=NewOrg&contact_info=&location=&return_to=https%3A%2F%2Fevil.example";
        let req = Request::builder()
            .method("POST")
            .uri("/outbound/recipients")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let location = res.headers().get("location").unwrap().to_str().unwrap();
        assert_eq!(location, "/outbound/recipients");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The read-back side of the round trip: a valid `recipient_project_id`
    /// query param on `/outbound/new` preselects it in the dropdown — mirrors
    /// `eur_ledger.rs`'s
    /// `new_form_with_a_donor_id_query_param_preselects_and_shows_donor`.
    #[tokio::test]
    async fn new_form_with_a_recipient_project_id_query_param_preselects_it() {
        let (state, dir) = test_support::test_app("outbound-new-form-preselect");
        let conn = state.conn();
        let rp_id =
            super::qry::insert_recipient_project(&conn, &recipient_draft("PreselectedOrg"))
                .unwrap();
        drop(conn);

        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("GET")
            .uri(format!(
                "/outbound/new?date=2026-02-02&recipient_project_id={rp_id}"
            ))
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        assert!(body.contains(&format!(r#"value="{rp_id}" selected"#)));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A `recipient_project_id` that doesn't resolve to a real recipient
    /// (stale link, hand-edited query string) doesn't preselect anything —
    /// mirrors `eur_ledger.rs`'s
    /// `new_form_with_a_nonexistent_donor_id_query_param_does_not_preselect_donor`-shaped
    /// precedent.
    #[tokio::test]
    async fn new_form_with_a_nonexistent_recipient_project_id_does_not_preselect() {
        let (state, dir) = test_support::test_app("outbound-new-form-bad-preselect");
        let app = crate::build_app(state.clone());
        let cookie = test_support::login(&app).await;

        let req = Request::builder()
            .method("GET")
            .uri("/outbound/new?recipient_project_id=999999")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = test_support::send(app, req).await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = test_support::body_text(res).await;
        assert!(!body.contains("999999"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
