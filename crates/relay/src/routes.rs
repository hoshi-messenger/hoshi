use axum::{
    Json,
    extract::State,
    response::{Html, IntoResponse},
};

use crate::{ServerState, api::HealthzResponse};

pub async fn index_get(State(_state): State<ServerState>) -> Html<String> {
    Html("<h1>Welcome to the Hoshi relay!</h1>".to_string())
}

pub async fn healthz_get(State(state): State<ServerState>) -> impl IntoResponse {
    Json(HealthzResponse {
        status: "ok".to_string(),
        guid: state.config.guid.clone(),
        control_plane_uri: state.config.control_plane_uri.clone(),
    })
}
