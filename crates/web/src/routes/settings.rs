use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Form;
use axum::Router;
use serde::Deserialize;

use adm_sfa_core::db::queries::{categories as cat_qry, documents as documents_qry};

use crate::state::AppState;
use crate::templates::{HtmlTemplate, SettingsTemplate};

/// Category and document-label CRUD only — desktop's Settings also has a
/// locale picker, a screenshot-command field, and a manual "backup now"
/// button, none of which are ported here. The locale picker has no web
/// counterpart yet (this crate has no i18n at all — an existing, logged
/// gap, not something this section should silently half-solve). The
/// screenshot command is permanently desktop-only per CLAUDE.md ("a
/// browser cannot invoke the OS screenshot tool on the client machine").
/// Manual backup is skipped deliberately, not overlooked: phase 6 already
/// gives the web deployment its own unattended nightly backup
/// (deploy/adm-sfa-backup.service+.timer) that desktop's manual zip button
/// has no equivalent of, so a duplicate manual trigger here isn't pulling
/// its weight for this pass.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/settings", get(index))
        .route("/settings/categories", post(create_category))
        .route("/settings/categories/{id}", post(rename_category))
        .route("/settings/categories/{id}/delete", post(delete_category))
        .route("/settings/labels", post(create_label))
        .route("/settings/labels/{id}", post(rename_label))
        .route("/settings/labels/{id}/delete", post(delete_label))
}

fn settings_template(conn: &rusqlite::Connection, error: Option<String>) -> SettingsTemplate {
    let categories = cat_qry::list(conn)
        .unwrap_or_default()
        .into_iter()
        .map(|c| (c.id, c.name))
        .collect();
    let labels = documents_qry::list_labels(conn).unwrap_or_default();
    SettingsTemplate {
        categories,
        labels,
        error,
    }
}

async fn index(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.conn();
    HtmlTemplate(settings_template(&conn, None))
}

#[derive(Deserialize)]
struct NameForm {
    name: String,
}

async fn create_category(State(state): State<AppState>, Form(form): Form<NameForm>) -> Response {
    let conn = state.conn();
    match cat_qry::insert(&conn, &form.name) {
        Ok(_) => Redirect::to("/settings").into_response(),
        Err(e) => HtmlTemplate(settings_template(&conn, Some(e.to_string()))).into_response(),
    }
}

async fn rename_category(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<NameForm>,
) -> Response {
    let conn = state.conn();
    match cat_qry::update(&conn, id, &form.name) {
        Ok(()) => Redirect::to("/settings").into_response(),
        Err(e) => HtmlTemplate(settings_template(&conn, Some(e.to_string()))).into_response(),
    }
}

async fn delete_category(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let conn = state.conn();
    match cat_qry::in_use(&conn, id) {
        Err(e) => HtmlTemplate(settings_template(&conn, Some(e.to_string()))).into_response(),
        Ok(true) => HtmlTemplate(settings_template(
            &conn,
            Some(
                "This category is used by at least one inventory item and can't be deleted."
                    .to_string(),
            ),
        ))
        .into_response(),
        Ok(false) => match cat_qry::delete(&conn, id) {
            Ok(()) => Redirect::to("/settings").into_response(),
            Err(e) => HtmlTemplate(settings_template(&conn, Some(e.to_string()))).into_response(),
        },
    }
}

async fn create_label(State(state): State<AppState>, Form(form): Form<NameForm>) -> Response {
    let conn = state.conn();
    match documents_qry::insert_label(&conn, &form.name) {
        Ok(_) => Redirect::to("/settings").into_response(),
        Err(e) => HtmlTemplate(settings_template(&conn, Some(e.to_string()))).into_response(),
    }
}

async fn rename_label(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<NameForm>,
) -> Response {
    let conn = state.conn();
    match documents_qry::update_label(&conn, id, &form.name) {
        Ok(()) => Redirect::to("/settings").into_response(),
        Err(e) => HtmlTemplate(settings_template(&conn, Some(e.to_string()))).into_response(),
    }
}

async fn delete_label(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let conn = state.conn();
    match documents_qry::delete_label(&conn, id) {
        Ok(()) => Redirect::to("/settings").into_response(),
        Err(e) => HtmlTemplate(settings_template(&conn, Some(e.to_string()))).into_response(),
    }
}
