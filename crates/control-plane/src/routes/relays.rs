use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use hoshi_protocol::control_plane::{
    RelayEntry, 
};

use crate::{ServerState, now};

const RELAY_TTL_SECONDS: i64 = 90;

pub(crate) async fn list_relays_get(State(state): State<ServerState>) -> Response {
    let now_ts = now();
    let mut stale_keys = Vec::new();
    let mut relays: Vec<RelayEntry> = Vec::new();

    for relay in state.relays.iter() {
        let relay_age = now_ts.saturating_sub(relay.value().last_seen);
        if relay_age > RELAY_TTL_SECONDS {
            stale_keys.push(relay.key().clone());
            continue;
        }
        relays.push(relay.value().entry.clone());
    }

    for key in stale_keys {
        state.relays.remove(&key);
    }

    relays.sort_by(|a, b| a.public_key.cmp(&b.public_key));

    (StatusCode::OK, Json(relays)).into_response()
}
