use axum::{
    Json,
    extract::State,
    response::{Html, IntoResponse, Response},
};
use hoshi_protocol::control_plane::NoisePublicKeyResponse;
use hoshi_protocol::noise::REGISTRATION_NOISE_PATTERN;

use crate::{ServerState, utils::response_html};

pub(crate) async fn index_get(State(_state): State<ServerState>) -> Html<String> {
    let html = "<h1>Welcome to the Hoshi control plane!</h1>";
    response_html(html, "Hoshi Control Plane")
}

pub(crate) async fn noise_public_key_get(State(state): State<ServerState>) -> Response {
    (
        axum::http::StatusCode::OK,
        Json(NoisePublicKeyResponse {
            pattern: REGISTRATION_NOISE_PATTERN.to_string(),
            public_key: state.noise_public_key().to_string(),
        }),
    )
        .into_response()
}
