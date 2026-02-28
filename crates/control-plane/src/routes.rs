use axum::{extract::State, response::Html};

use crate::{ServerState, utils::response_html};

pub async fn index_get(
    State(_state): State<ServerState>,
) -> Html<String> {
    let html = "<h1>Welcome to the Hoshi control plane!</h1>";
    response_html(html, "Hoshi Control Plane")
}