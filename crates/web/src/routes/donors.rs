use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect};
use axum::routing::get;
use axum::Form;
use axum::Router;
use serde::Deserialize;

use adm_sfa_core::db::queries::donors as donors_qry;
use adm_sfa_core::model::donor::DonorDraft;

use crate::state::AppState;
use crate::templates::{DonorFormTemplate, DonorRow, DonorsListTemplate, HtmlTemplate};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/donors", get(list).post(create))
        .route("/donors/new", get(new_form))
        .route("/donors/{id}/edit", get(edit_form).post(update))
}

async fn list(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.conn();
    let donors = donors_qry::list(&conn).unwrap_or_default();
    let rows = donors
        .into_iter()
        .map(|d| DonorRow {
            id: d.id,
            name: d.name,
            contact_info: d.contact_info.unwrap_or_default(),
        })
        .collect();
    HtmlTemplate(DonorsListTemplate { donors: rows })
}

async fn new_form() -> impl IntoResponse {
    HtmlTemplate(DonorFormTemplate {
        id: None,
        draft: DonorDraft::default(),
        error: None,
    })
}

async fn edit_form(State(state): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    let conn = state.conn();
    let Some(donor) = donors_qry::list(&conn)
        .unwrap_or_default()
        .into_iter()
        .find(|d| d.id == id)
    else {
        return (axum::http::StatusCode::NOT_FOUND, "donor not found").into_response();
    };
    let draft = DonorDraft {
        name: donor.name,
        contact_info: donor.contact_info.unwrap_or_default(),
        notes: donor.notes.unwrap_or_default(),
    };
    HtmlTemplate(DonorFormTemplate {
        id: Some(id),
        draft,
        error: None,
    })
    .into_response()
}

#[derive(Deserialize)]
struct DonorForm {
    name: String,
    #[serde(default)]
    contact_info: String,
    #[serde(default)]
    notes: String,
}

async fn create(State(state): State<AppState>, Form(form): Form<DonorForm>) -> impl IntoResponse {
    let draft = DonorDraft {
        name: form.name,
        contact_info: form.contact_info,
        notes: form.notes,
    };
    let conn = state.conn();
    match donors_qry::insert(&conn, &draft) {
        Ok(id) => Redirect::to(&format!("/donors/{id}/edit")).into_response(),
        Err(e) => HtmlTemplate(DonorFormTemplate {
            id: None,
            draft,
            error: Some(e.to_string()),
        })
        .into_response(),
    }
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<DonorForm>,
) -> impl IntoResponse {
    let draft = DonorDraft {
        name: form.name,
        contact_info: form.contact_info,
        notes: form.notes,
    };
    let conn = state.conn();
    match donors_qry::update(&conn, id, &draft) {
        Ok(()) => Redirect::to(&format!("/donors/{id}/edit")).into_response(),
        Err(e) => HtmlTemplate(DonorFormTemplate {
            id: Some(id),
            draft,
            error: Some(e.to_string()),
        })
        .into_response(),
    }
}
